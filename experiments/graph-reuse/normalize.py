#!/usr/bin/env python3
"""A scratch normalizer for one pinned CodeGraph snapshot (#295).

This is experiment code, not an importer: it reads a private copy of a
CodeGraph database read-only and prints normalized edges, so the benchmark
can compare what the stored graph knows with what the `codegraph` CLI prints.
Nothing here writes to a database or touches a codecite store.

    python3 -I normalize.py <codegraph.db> <checkout> <pin.json> callers|callees|refs <path>:<line>
    python3 -I normalize.py <codegraph.db> <checkout> <pin.json> resolve <name>

Trust rules (README, "Normalization"):

- The artifact must match its pin (sha256, schema version, producer and
  extraction versions), report a complete index and have no pending WAL
  frames; otherwise it is refused outright.
- An endpoint is a stored node ID with its path, span and kind. A symbol is
  found by path and line or by an exact qualified name, never merged with
  another by bare name; a bare name that matches several nodes is reported
  as ambiguous with every ID.
- An edge is `verified` only when both endpoint files are inside the
  checkout (no `..`, absolute path or symlink), their bytes match the hash
  CodeGraph recorded, both spans fit the file, and the site, when there is
  one, lies inside the source's span. Anything else is `unverified` and is
  not evidence. A name appearing on the site line is never checked or used:
  it would not establish which definition the call binds to.
- Certainty is carried, never promoted: `extracted` for a stored confidence
  of at least 0.7, `low` below it, `unknown` when the edge has no
  confidence. Nothing from CodeGraph is ever `exact`. A pair-only edge (no
  line) keeps `site: null`; per-site evidence is never invented.
"""

import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path

PROVIDER = 'codegraph'
REQUIRED = {
    'schema_versions': {'version'},
    'project_metadata': {'key', 'value'},
    'files': {'path', 'content_hash'},
    'nodes': {'id', 'kind', 'name', 'qualified_name', 'file_path', 'start_line', 'end_line'},
    'edges': {'source', 'target', 'kind', 'metadata', 'line', 'col', 'provenance'},
    'unresolved_refs': {'from_node_id', 'reference_name', 'reference_kind', 'line', 'col', 'file_path', 'status'},
}
EDGE_KINDS = ('calls',)
REF_KINDS = ('calls', 'references', 'imports', 'instantiates', 'extends', 'implements')
EXTRACTED_AT = 0.7
MAX_FILE_BYTES = 16 * 1024 * 1024


class ArtifactError(Exception):
    """The artifact is unpinned, unsupported, incomplete or inconsistent; none of it is evidence."""


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as fh:
        for block in iter(lambda: fh.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def stamp(db):
    """Size, mtime and inode of a snapshot, so a harness that hashed its own private copy once need not rehash it
    on every query. A stamp is only as good as the snapshot's privacy: it is not a defence against an adversary
    who can write the file."""
    st = os.stat(db)
    return {'size': st.st_size, 'mtime_ns': st.st_mtime_ns, 'inode': st.st_ino}


class Artifact:
    def __init__(self, db, root, pin):
        db = Path(db)
        self.root = Path(root)
        self.real_root = Path(os.path.realpath(self.root))
        if db.is_symlink():
            raise ArtifactError(f'{db}: the database is a symlink')
        if not db.is_file():
            raise ArtifactError(f'{db}: not a regular file')
        wal = Path(str(db) + '-wal')
        if wal.exists() and wal.stat().st_size > 0:
            raise ArtifactError(f'{db}: nonempty WAL; take a consistent snapshot first')
        if not pin.get('sha256'):
            raise ArtifactError(f'{db}: unpinned artifact (no sha256)')
        # A private snapshot hashed once may carry its stat stamp; any change to it means hashing again.
        if pin.get('verified_stamp') != stamp(db):
            actual = sha256_file(db)
            if actual != pin['sha256']:
                raise ArtifactError(f'{db}: sha256 {actual} does not match the pin {pin["sha256"]}')
        self.sha256 = pin['sha256']
        # immutable=1: SQLite takes no locks and writes no WAL or journal.
        self.con = sqlite3.connect(f'file:{db}?mode=ro&immutable=1', uri=True)
        self.con.row_factory = sqlite3.Row
        self._check_schema(pin)
        self._files = {}
        self._lines = {}

    def close(self):
        self.con.close()

    def _check_schema(self, pin):
        for table, columns in REQUIRED.items():
            have = {row['name'] for row in self.con.execute(f'PRAGMA table_info({table})')}
            if not columns <= have:
                raise ArtifactError(f'unsupported schema: {table} lacks {sorted(columns - have)}')
        version = self.con.execute('SELECT max(version) FROM schema_versions').fetchone()[0]
        if version != pin['schema_version']:
            raise ArtifactError(f'unsupported schema version {version}; pinned {pin["schema_version"]}')
        meta = dict(self.con.execute('SELECT key, value FROM project_metadata').fetchall())
        if meta.get('indexed_with_version') != pin['producer_version']:
            raise ArtifactError(f'producer {meta.get("indexed_with_version")} is not the pinned {pin["producer_version"]}')
        if meta.get('indexed_with_extraction_version') != pin['extraction_version']:
            raise ArtifactError(f'extraction version {meta.get("indexed_with_extraction_version")} '
                                f'is not the pinned {pin["extraction_version"]}')
        if meta.get('index_state') != 'complete':
            raise ArtifactError(f'index state {meta.get("index_state")!r} is not complete')
        self.producer_version = meta['indexed_with_version']
        self.schema_version = version

    # Source checks.

    def check_file(self, path):
        """Problems with trusting `path` as a source file of this checkout, cached."""
        if path in self._files:
            return self._files[path]
        problems = []
        parts = Path(path).parts
        if not path or Path(path).is_absolute() or '..' in parts or '\0' in path:
            problems.append(f'{path}: outside the checkout')
        else:
            joined = self.root / path
            real = Path(os.path.realpath(joined))
            if real != self.real_root and self.real_root not in real.parents:
                problems.append(f'{path}: outside the checkout (symlink escape)')
            elif any((self.root / Path(*parts[:i])).is_symlink() for i in range(1, len(parts) + 1)):
                problems.append(f'{path}: a symlink, not a source file')
            elif not joined.is_file():
                problems.append(f'{path}: missing')
            elif joined.stat().st_size > MAX_FILE_BYTES:
                problems.append(f'{path}: larger than {MAX_FILE_BYTES} bytes')
            else:
                data = joined.read_bytes()
                row = self.con.execute('SELECT content_hash FROM files WHERE path = ?', (path,)).fetchone()
                if row is None:
                    problems.append(f'{path}: not in the artifact\'s file table')
                elif hashlib.sha256(data).hexdigest() != row['content_hash']:
                    problems.append(f'{path}: source bytes differ from the recorded hash')
                else:
                    self._lines[path] = data.count(b'\n') + (0 if data.endswith(b'\n') or not data else 1)
        self._files[path] = problems
        return problems

    def endpoint(self, node_id):
        row = self.con.execute('SELECT id, kind, name, qualified_name, file_path, start_line, end_line '
                               'FROM nodes WHERE id = ?', (node_id,)).fetchone()
        if row is None:
            return None, [f'{node_id}: no such node']
        end = {'id': row['id'], 'kind': row['kind'], 'name': row['name'], 'qualified_name': row['qualified_name'],
               'path': row['file_path'], 'span': [row['start_line'], row['end_line']]}
        problems = list(self.check_file(row['file_path']))
        if not problems:
            lines = self._lines[row['file_path']]
            if not 1 <= row['start_line'] <= row['end_line'] <= lines:
                problems.append(f'{node_id}: span {row["start_line"]}-{row["end_line"]} does not fit '
                                f'{row["file_path"]} ({lines} lines)')
        return end, problems

    # Queries. All of them go through stored node IDs.

    def resolve_at(self, path, line):
        """Node IDs whose span in `path` holds `line`, innermost first; files and imports are not symbols."""
        rows = self.con.execute(
            "SELECT id FROM nodes WHERE file_path = ? AND start_line <= ? AND end_line >= ? "
            "AND kind NOT IN ('file', 'import') ORDER BY end_line - start_line, start_line DESC, id",
            (path, line, line)).fetchall()
        return [r['id'] for r in rows]

    def resolve_def(self, path, line):
        """The node defined on `line` of `path`: the innermost one starting there, else the innermost holding it."""
        rows = self.con.execute(
            "SELECT id FROM nodes WHERE file_path = ? AND start_line = ? AND kind NOT IN ('file', 'import') "
            'ORDER BY end_line - start_line, id', (path, line)).fetchall()
        if rows:
            return [r['id'] for r in rows]
        return self.resolve_at(path, line)

    def resolve_name(self, name):
        """Exact match only: a qualified name (`A::b`) against qualified names, a bare name against names and
        qualified names. Several matches are ambiguous and all returned, never merged or picked."""
        if '::' in name or '.' in name:
            sql = "SELECT id FROM nodes WHERE qualified_name = ? AND kind NOT IN ('file', 'import') ORDER BY id"
            params = (name,)
        else:
            sql = ("SELECT id FROM nodes WHERE (name = ? OR qualified_name = ?) AND kind NOT IN ('file', 'import') "
                   'ORDER BY id')
            params = (name, name)
        ids = [r['id'] for r in self.con.execute(sql, params).fetchall()]
        if not ids:
            return {'status': 'not_found', 'ids': []}
        return {'status': 'unique' if len(ids) == 1 else 'ambiguous', 'ids': ids}

    def edges(self, node_id, direction, kinds=EDGE_KINDS):
        """Edges into (`callers`, `refs`) or out of (`callees`) one node, each normalized."""
        near, far = ('target', 'source') if direction in ('callers', 'refs') else ('source', 'target')
        sql = f'SELECT source, target, kind, metadata, line, col, provenance FROM edges WHERE {near} = ?'
        params = [node_id]
        if kinds:
            sql += f' AND kind IN ({",".join("?" * len(kinds))})'
            params += list(kinds)
        sql += ' ORDER BY kind, line, col, source, target'
        out = []
        for row in self.con.execute(sql, params).fetchall():
            out.append(self._normalize(row))
        return out

    def _normalize(self, row):
        source, sp = self.endpoint(row['source'])
        target, tp = self.endpoint(row['target'])
        problems = sp + tp
        try:
            meta = json.loads(row['metadata']) if row['metadata'] else {}
        except ValueError:
            meta = {}
            problems.append('edge metadata is not JSON')
        if not isinstance(meta, dict):
            meta = {}
        confidence = meta.get('confidence')
        if not isinstance(confidence, (int, float)) or isinstance(confidence, bool):
            confidence = None
        site = None
        if row['line'] is not None and source is not None:
            site = {'path': source['path'], 'line': row['line'], 'col': row['col']}
            lo, hi = source['span']
            if not lo <= row['line'] <= hi:
                problems.append(f'site line {row["line"]} is outside the source span {lo}-{hi}')
        if confidence is None:
            certainty = 'unknown'
        elif confidence >= EXTRACTED_AT:
            certainty = 'extracted'
        else:
            certainty = 'low'
        return {
            'provider': PROVIDER,
            'producer_version': self.producer_version,
            'schema_version': self.schema_version,
            'artifact_sha256': self.sha256,
            'kind': row['kind'],
            'source': source,
            'target': target,
            'site': site,
            'uncertainty': {'confidence': confidence, 'resolved_by': meta.get('resolvedBy'),
                            'ref_name': meta.get('refName'), 'provenance': row['provenance']},
            'certainty': certainty,
            'evidence': 'unverified' if problems else 'verified',
            'problems': problems,
        }

    def unresolved(self, node_id):
        """Sites in one node that CodeGraph could not bind: unknowns, kept visible."""
        rows = self.con.execute(
            'SELECT reference_name, reference_kind, line, col, file_path, status FROM unresolved_refs '
            'WHERE from_node_id = ? ORDER BY line, col, reference_name', (node_id,)).fetchall()
        return [{'name': r['reference_name'], 'kind': r['reference_kind'],
                 'site': {'path': r['file_path'], 'line': r['line'], 'col': r['col']}, 'status': r['status']}
                for r in rows]

    def query(self, op, path, line):
        """One benchmark query by definition location: the resolved node and its normalized edges."""
        ids = self.resolve_def(path, line)
        if not ids:
            return {'status': 'not_found', 'ids': [], 'edges': [], 'unresolved': []}
        node = ids[0]
        kinds = REF_KINDS if op == 'refs' else EDGE_KINDS
        edges = self.edges(node, op, kinds)
        unresolved = self.unresolved(node) if op == 'callees' else []
        return {'status': 'ok', 'ids': [node], 'edges': edges, 'unresolved': unresolved}


def main(argv):
    if len(argv) < 5:
        print(__doc__, file=sys.stderr)
        return 2
    db, root, pin_path, op = argv[1:5]
    pin = json.loads(Path(pin_path).read_text())
    try:
        art = Artifact(db, root, pin)
    except ArtifactError as err:
        print(json.dumps({'status': 'rejected', 'error': str(err)}))
        return 1
    if op == 'resolve':
        result = art.resolve_name(argv[5])
    else:
        path, line = argv[5].rsplit(':', 1)
        result = art.query(op, path, int(line))
    print(json.dumps(result, indent=1))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
