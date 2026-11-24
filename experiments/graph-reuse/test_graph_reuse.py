"""Offline tests for the CodeGraph reuse benchmark (#295).

    python3 -m unittest experiments/graph-reuse/test_graph_reuse.py

Fixtures are tiny hand-built CodeGraph-shaped databases and source trees in
a temporary directory; no CLI, network or real index is used.
"""

import hashlib
import json
import os
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import normalize  # noqa: E402
import run  # noqa: E402

SOURCE = '''class A:
    def run(self):
        return helper_a()


class B:
    def run(self):
        return helper_b()


def helper_a():
    return 1


def helper_b():
    return 2


def main(x):
    A().run()
    x.run()
    return helper_a()
'''

OTHER = '''def run():
    return 0
'''

SCHEMA = '''
CREATE TABLE schema_versions (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL, description TEXT);
CREATE TABLE project_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE files (path TEXT PRIMARY KEY, content_hash TEXT NOT NULL, language TEXT NOT NULL, size INTEGER NOT NULL,
  modified_at INTEGER NOT NULL, indexed_at INTEGER NOT NULL, node_count INTEGER DEFAULT 0, errors TEXT, generated INTEGER NOT NULL DEFAULT 0);
CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, qualified_name TEXT NOT NULL,
  file_path TEXT NOT NULL, language TEXT NOT NULL, start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  start_column INTEGER NOT NULL, end_column INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE edges (id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL,
  metadata TEXT, line INTEGER, col INTEGER, provenance TEXT DEFAULT NULL);
CREATE TABLE unresolved_refs (id INTEGER PRIMARY KEY AUTOINCREMENT, from_node_id TEXT NOT NULL, reference_name TEXT NOT NULL,
  reference_kind TEXT NOT NULL, line INTEGER NOT NULL, col INTEGER NOT NULL, candidates TEXT, file_path TEXT NOT NULL DEFAULT '',
  language TEXT NOT NULL DEFAULT 'unknown', status TEXT NOT NULL DEFAULT 'pending', name_tail TEXT NOT NULL DEFAULT '');
'''

NODES = [
    ('class:A', 'class', 'A', 'A', 'pkg/a.py', 1, 3),
    ('method:A.run', 'method', 'run', 'A::run', 'pkg/a.py', 2, 3),
    ('class:B', 'class', 'B', 'B', 'pkg/a.py', 6, 8),
    ('method:B.run', 'method', 'run', 'B::run', 'pkg/a.py', 7, 8),
    ('function:helper_a', 'function', 'helper_a', 'helper_a', 'pkg/a.py', 11, 12),
    ('function:helper_b', 'function', 'helper_b', 'helper_b', 'pkg/a.py', 15, 16),
    ('function:main', 'function', 'main', 'main', 'pkg/a.py', 19, 22),
    ('function:other.run', 'function', 'run', 'run', 'pkg/other.py', 1, 2),
]


def meta(confidence, by='exact-match', name=None):
    return json.dumps({'confidence': confidence, 'resolvedBy': by, 'refName': name})


EDGES = [
    ('method:A.run', 'function:helper_a', 'calls', meta(0.9, name='helper_a'), 3, 15),
    ('method:B.run', 'function:helper_b', 'calls', meta(0.9, name='helper_b'), 8, 15),
    # A().run() bound to B.run by name: the kind of edge a reader must see as uncertain.
    ('function:main', 'method:B.run', 'calls', meta(0.4, name='run'), 20, 4),
    ('function:main', 'function:helper_a', 'calls', meta(0.9, name='helper_a'), 22, 11),
    # A pair-only relationship with no site and no metadata.
    ('function:main', 'class:A', 'references', None, None, None),
]


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Fixture:
    """A source tree and a CodeGraph-shaped database built from it."""

    def __init__(self, tmp, *, schema_version=9, producer='1.6.0', extraction='25', state='complete',
                 hashes=None, nodes=NODES, edges=EDGES):
        self.root = Path(tmp) / 'repo'
        (self.root / 'pkg').mkdir(parents=True)
        (self.root / 'pkg' / 'a.py').write_text(SOURCE)
        (self.root / 'pkg' / 'other.py').write_text(OTHER)
        self.db = Path(tmp) / 'snap' / 'codegraph.db'
        self.db.parent.mkdir()
        con = sqlite3.connect(self.db)
        con.executescript(SCHEMA)
        con.execute('INSERT INTO schema_versions VALUES (?, 0, ?)', (schema_version, 'x'))
        for key, value in (('indexed_with_version', producer), ('indexed_with_extraction_version', extraction),
                           ('index_state', state)):
            con.execute('INSERT INTO project_metadata VALUES (?, ?, 0)', (key, value))
        hashes = hashes or {}
        for path in ('pkg/a.py', 'pkg/other.py'):
            digest = hashes.get(path, sha((self.root / path).read_bytes()))
            con.execute("INSERT INTO files VALUES (?, ?, 'python', 0, 0, 0, 0, NULL, 0)", (path, digest))
        for path in sorted({n[4] for n in nodes} - {'pkg/a.py', 'pkg/other.py'}):
            con.execute("INSERT INTO files VALUES (?, ?, 'python', 0, 0, 0, 0, NULL, 0)", (path, '0' * 64))
        for node in nodes:
            con.execute("INSERT INTO nodes VALUES (?, ?, ?, ?, ?, 'python', ?, ?, 0, 0, 0)", node)
        for edge in edges:
            con.execute('INSERT INTO edges (source, target, kind, metadata, line, col) VALUES (?, ?, ?, ?, ?, ?)', edge)
        con.execute("INSERT INTO unresolved_refs (from_node_id, reference_name, reference_kind, line, col, file_path, status) "
                    "VALUES ('function:main', 'x.run', 'calls', 21, 4, 'pkg/a.py', 'failed')")
        con.commit()
        con.close()

    def pin(self, **overrides):
        pin = {'schema_version': 9, 'producer_version': '1.6.0', 'extraction_version': '25',
               'sha256': sha(self.db.read_bytes())}
        pin.update(overrides)
        return pin

    def open(self, **overrides):
        return normalize.Artifact(self.db, self.root, self.pin(**overrides))


class NormalizeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def fixture(self, **kw):
        return Fixture(self.tmp.name, **kw)

    # Two same-named callers with disjoint targets, and a negative edge.

    def test_same_named_callers_keep_separate_source_ids_and_exact_sites(self):
        art = self.fixture().open()
        a = art.edges('function:helper_a', 'callers')
        b = art.edges('function:helper_b', 'callers')
        self.assertEqual({e['source']['id'] for e in a}, {'method:A.run', 'function:main'})
        self.assertEqual([e['source']['id'] for e in b], ['method:B.run'])
        a_run = next(e for e in a if e['source']['id'] == 'method:A.run')
        self.assertEqual(a_run['site'], {'path': 'pkg/a.py', 'line': 3, 'col': 15})
        self.assertEqual(b[0]['site'], {'path': 'pkg/a.py', 'line': 8, 'col': 15})
        # Both callers are named `run`; they stay distinct endpoints with their own spans.
        self.assertEqual(a_run['source']['name'], b[0]['source']['name'])
        self.assertNotEqual(a_run['source']['span'], b[0]['source']['span'])

    def test_negative_edge_is_absent(self):
        art = self.fixture().open()
        self.assertNotIn('function:helper_b', {e['target']['id'] for e in art.edges('method:A.run', 'callees')})

    def test_bare_name_resolution_exposes_ambiguity(self):
        art = self.fixture().open()
        found = art.resolve_name('run')
        self.assertEqual(found['status'], 'ambiguous')
        self.assertEqual(set(found['ids']), {'method:A.run', 'method:B.run', 'function:other.run'})

    def test_location_resolution_uses_path_and_line_not_name(self):
        art = self.fixture().open()
        self.assertEqual(art.resolve_at('pkg/a.py', 7), ['method:B.run', 'class:B'])  # innermost first
        self.assertEqual(art.resolve_at('pkg/other.py', 1), ['function:other.run'])
        self.assertEqual(art.resolve_at('pkg/a.py', 5), [])

    def test_qualified_resolution_is_exact(self):
        art = self.fixture().open()
        self.assertEqual(art.resolve_name('B::run'), {'status': 'unique', 'ids': ['method:B.run']})
        self.assertEqual(art.resolve_name('C::run')['status'], 'not_found')

    def test_no_invented_certainty(self):
        art = self.fixture().open()
        edges = {(e['source']['id'], e['target']['id']): e for e in art.edges('function:main', 'callees', kinds=None)}
        low = edges[('function:main', 'method:B.run')]
        self.assertEqual(low['uncertainty']['confidence'], 0.4)
        self.assertEqual(low['certainty'], 'low')
        high = edges[('function:main', 'function:helper_a')]
        self.assertEqual(high['certainty'], 'extracted')
        bare = edges[('function:main', 'class:A')]
        self.assertEqual(bare['certainty'], 'unknown')
        self.assertIsNone(bare['site'])
        for edge in edges.values():
            self.assertNotEqual(edge['certainty'], 'exact')
            self.assertEqual(edge['provider'], 'codegraph')
            self.assertEqual(edge['producer_version'], '1.6.0')

    def test_unresolved_sites_are_reported_not_dropped(self):
        art = self.fixture().open()
        unresolved = art.unresolved('function:main')
        self.assertEqual(unresolved, [{'name': 'x.run', 'kind': 'calls', 'site': {'path': 'pkg/a.py', 'line': 21, 'col': 4},
                                       'status': 'failed'}])

    def test_verified_edge(self):
        art = self.fixture().open()
        edge = art.edges('function:helper_b', 'callers')[0]
        self.assertEqual(edge['evidence'], 'verified')
        self.assertEqual(edge['problems'], [])

    # Rejections.

    def test_rejects_unknown_schema(self):
        fx = self.fixture(schema_version=10)
        with self.assertRaisesRegex(normalize.ArtifactError, 'schema'):
            fx.open()

    def test_rejects_other_producer_version(self):
        fx = self.fixture(producer='1.6.4')
        with self.assertRaisesRegex(normalize.ArtifactError, 'producer'):
            fx.open()

    def test_rejects_incomplete_index(self):
        fx = self.fixture(state='indexing')
        with self.assertRaisesRegex(normalize.ArtifactError, 'complete'):
            fx.open()

    def test_rejects_pin_mismatch(self):
        fx = self.fixture()
        with self.assertRaisesRegex(normalize.ArtifactError, 'sha256'):
            fx.open(sha256='0' * 64)

    def test_rejects_unpinned_artifact(self):
        fx = self.fixture()
        with self.assertRaisesRegex(normalize.ArtifactError, 'sha256'):
            fx.open(sha256=None)

    def test_stamp_skips_rehash_only_while_unchanged(self):
        fx = self.fixture()
        good = fx.pin(verified_stamp=normalize.stamp(fx.db))
        normalize.Artifact(fx.db, fx.root, dict(good, sha256='0' * 64)).close()  # stamp matches: not rehashed
        con = sqlite3.connect(fx.db)
        con.execute("UPDATE nodes SET name = 'x' WHERE id = 'function:main'")
        con.commit()
        con.close()
        with self.assertRaisesRegex(normalize.ArtifactError, 'sha256'):
            normalize.Artifact(fx.db, fx.root, good)  # changed file, stale stamp: rehashed and refused

    def test_rejects_nonempty_wal(self):
        fx = self.fixture()
        pin = fx.pin()
        Path(str(fx.db) + '-wal').write_bytes(b'x')
        with self.assertRaisesRegex(normalize.ArtifactError, 'WAL'):
            normalize.Artifact(fx.db, fx.root, pin)

    def test_rejects_symlinked_database(self):
        fx = self.fixture()
        link = fx.db.parent / 'link.db'
        link.symlink_to(fx.db)
        with self.assertRaisesRegex(normalize.ArtifactError, 'symlink'):
            normalize.Artifact(link, fx.root, fx.pin())

    def test_rejects_missing_table(self):
        fx = self.fixture()
        con = sqlite3.connect(fx.db)
        con.execute('DROP TABLE unresolved_refs')
        con.commit()
        con.close()
        with self.assertRaisesRegex(normalize.ArtifactError, 'schema'):
            fx.open()

    def test_changed_source_is_not_evidence(self):
        art = self.fixture(hashes={'pkg/a.py': '1' * 64}).open()
        edge = art.edges('function:helper_b', 'callers')[0]
        self.assertEqual(edge['evidence'], 'unverified')
        self.assertIn('pkg/a.py: source bytes differ from the recorded hash', edge['problems'])

    def test_escaped_paths_are_not_evidence(self):
        nodes = NODES + [('function:esc', 'function', 'esc', 'esc', '../outside.py', 1, 1),
                         ('function:abs', 'function', 'abs', 'abs', '/etc/passwd', 1, 1)]
        edges = EDGES + [('function:esc', 'function:helper_a', 'calls', meta(0.9), 1, 0),
                         ('function:abs', 'function:helper_a', 'calls', meta(0.9), 1, 0)]
        fx = self.fixture(nodes=nodes, edges=edges)
        (Path(self.tmp.name) / 'outside.py').write_text('esc()\n')
        art = fx.open()
        by_id = {e['source']['id']: e for e in art.edges('function:helper_a', 'callers')}
        for node in ('function:esc', 'function:abs'):
            self.assertEqual(by_id[node]['evidence'], 'unverified')
            self.assertTrue(any('outside the checkout' in p for p in by_id[node]['problems']), by_id[node]['problems'])

    def test_symlink_escape_is_not_evidence(self):
        nodes = NODES + [('function:link', 'function', 'link', 'link', 'pkg/link.py', 1, 1)]
        edges = EDGES + [('function:link', 'function:helper_a', 'calls', meta(0.9), 1, 0)]
        fx = self.fixture(nodes=nodes, edges=edges)
        outside = Path(self.tmp.name) / 'secret.py'
        outside.write_text('link()\n')
        (fx.root / 'pkg' / 'link.py').symlink_to(outside)
        art = fx.open()
        edge = next(e for e in art.edges('function:helper_a', 'callers') if e['source']['id'] == 'function:link')
        self.assertEqual(edge['evidence'], 'unverified')
        self.assertTrue(any('symlink' in p for p in edge['problems']), edge['problems'])

    def test_invalid_span_is_not_evidence(self):
        nodes = [n if n[0] != 'method:B.run' else ('method:B.run', 'method', 'run', 'B::run', 'pkg/a.py', 7, 99)
                 for n in NODES]
        art = self.fixture(nodes=nodes).open()
        edge = art.edges('function:helper_b', 'callers')[0]
        self.assertEqual(edge['evidence'], 'unverified')
        self.assertTrue(any('span' in p for p in edge['problems']), edge['problems'])

    def test_site_outside_source_span_is_not_evidence(self):
        edges = [e if e[1] != 'function:helper_b' else ('method:B.run', 'function:helper_b', 'calls', meta(0.9), 12, 0)
                 for e in EDGES]
        art = self.fixture(edges=edges).open()
        edge = art.edges('function:helper_b', 'callers')[0]
        self.assertEqual(edge['evidence'], 'unverified')
        self.assertTrue(any('site' in p for p in edge['problems']), edge['problems'])

    def test_database_is_never_written(self):
        fx = self.fixture()
        before = sha(fx.db.read_bytes())
        art = fx.open()
        art.edges('function:helper_a', 'callers')
        art.close()
        self.assertEqual(sha(fx.db.read_bytes()), before)
        self.assertFalse(Path(str(fx.db) + '-wal').exists())


class RunTest(unittest.TestCase):
    """Case validation, output parsing and scoring in run.py."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'src' / 'toy'
        (self.root / 'pkg').mkdir(parents=True)
        (self.root / 'pkg' / 'a.py').write_text(SOURCE)
        (self.root / 'pkg' / 'other.py').write_text(OTHER)
        os.system(f'git -C {self.root} init -q && git -C {self.root} add -A && '
                  f'git -C {self.root} -c user.email=t@t -c user.name=t commit -qm t')
        self.commit = os.popen(f'git -C {self.root} rev-parse HEAD').read().strip()
        self.repos = {'toy': {'name': 'toy', 'commit': self.commit, 'split': 'evaluation'}}

    def case(self, **kw):
        case = {'id': 'toy/callers/helper_b', 'repo': 'toy', 'class': 'direct_call', 'op': 'callers',
                'symbol': {'name': 'helper_b', 'path': 'pkg/a.py', 'line': 15},
                'expect': [{'targets': [{'path': 'pkg/a.py', 'line': 7, 'name': 'run'}], 'sites': [8]}],
                'certainty': 'certain'}
        case.update(kw)
        return case

    def validate(self, cases):
        return run.validate_cases(cases, self.repos, Path(self.tmp.name) / 'src')

    def test_valid_case_passes(self):
        self.assertEqual(self.validate([self.case()]), [])

    def test_rejects_duplicate_ids(self):
        self.assertIn('duplicate id', ' '.join(self.validate([self.case(), self.case()])))

    def test_rejects_label_line_without_name(self):
        bad = self.case(expect=[{'targets': [{'path': 'pkg/a.py', 'line': 11, 'name': 'run'}], 'sites': [8]}])
        self.assertIn('does not name', ' '.join(self.validate([bad])))

    def test_rejects_symbol_line_without_name(self):
        bad = self.case(symbol={'name': 'helper_b', 'path': 'pkg/a.py', 'line': 11})
        self.assertIn('does not name', ' '.join(self.validate([bad])))

    def test_rejects_missing_line(self):
        bad = self.case(symbol={'name': 'helper_b', 'path': 'pkg/a.py', 'line': 400})
        self.assertIn('missing line', ' '.join(self.validate([bad])))

    def test_rejects_wrong_pin(self):
        self.repos['toy']['commit'] = '0' * 40
        self.assertIn('not at', ' '.join(self.validate([self.case()])))

    def test_rejects_miss_case_with_answers(self):
        bad = self.case(**{'class': 'miss'})
        self.assertIn('miss case', ' '.join(self.validate([bad])))

    def test_rejects_unknown_class(self):
        self.assertIn('class', ' '.join(self.validate([self.case(**{'class': 'vibes'})])))

    # Outcomes.

    def test_missing_scip_is_unsupported_not_a_miss(self):
        stderr = ('Error: no SCIP index covers `helper_b`, so codecite has no references or calls for it; '
                  'name a SCIP index with `scip` on its source in the manifest and index again\n')
        self.assertEqual(run.codecite_status(1, '', stderr), 'unsupported')

    def test_codecite_ambiguity_is_exposed(self):
        stderr = 'Error: `run` is ambiguous: a.b, c.d\n'
        self.assertEqual(run.codecite_status(1, '', stderr), 'ambiguous')

    def test_codegraph_not_found_exits_zero_but_is_not_found(self):
        self.assertEqual(run.codegraph_status(0, '', 'ℹ Symbol "nosuch" not found\n'), 'not_found')

    def test_codegraph_callers_parse(self):
        out = json.dumps({'symbol': 'helper_b', 'callers': [
            {'name': 'run', 'kind': 'method', 'filePath': 'pkg/a.py', 'startLine': 7}]})
        self.assertEqual(run.parse_codegraph(out, 'callers'),
                         [{'path': 'pkg/a.py', 'line': 7, 'end': None, 'name': 'run', 'site': None}])

    def test_codecite_graph_parse(self):
        out = json.dumps({'symbol': 'helper_b', 'id': 'x', 'direction': 'callers', 'depth': 1, 'total': 1, 'results': [
            {'id': 'y', 'name': 'run', 'kind': 'method', 'distance': 1, 'citation': 'toy@abc1234:pkg/a.py#L7-L8',
             'call_site': 'toy@abc1234:pkg/a.py#L8', 'confidence': 3}]})
        self.assertEqual(run.parse_codecite(out, 'callers'),
                         [{'path': 'pkg/a.py', 'line': 7, 'end': 8, 'name': 'run', 'site': 8, 'confidence': 3}])

    # Scoring.

    def test_same_name_in_another_file_does_not_match(self):
        case = self.case()
        got = [{'path': 'pkg/other.py', 'line': 1, 'end': 2, 'name': 'run', 'site': None}]
        score = run.score_case(case, got)
        self.assertEqual(score['tp'], 0)
        self.assertEqual(score['fp'], 1)
        self.assertEqual(score['fn'], 1)

    def test_span_containment_matches_and_site_is_checked(self):
        case = self.case()
        got = [{'path': 'pkg/a.py', 'line': 6, 'end': 8, 'name': 'run', 'site': 8}]
        score = run.score_case(case, got)
        self.assertEqual((score['tp'], score['fp'], score['fn']), (1, 0, 0))
        self.assertEqual(score['site_ok'], 1)
        wrong_site = run.score_case(case, [{**got[0], 'site': 3}])
        self.assertEqual(wrong_site['site_wrong'], 1)

    def test_enclosing_container_does_not_match_a_member(self):
        # A class span holds the method's line, but the class is not the method.
        case = self.case()
        got = [{'path': 'pkg/a.py', 'line': 6, 'end': 8, 'name': 'B', 'site': None}]
        self.assertEqual(run.score_case(case, got)['tp'], 0)

    def test_unknown_site_flags_a_unique_target_as_falsely_unique(self):
        case = self.case(**{'class': 'unknown', 'op': 'callees', 'certainty': 'unknown', 'site': 21,
                            'symbol': {'name': 'main', 'path': 'pkg/a.py', 'line': 19}, 'expect': [],
                            'call_name': 'run'})
        got = [{'path': 'pkg/a.py', 'line': 7, 'end': 8, 'name': 'run', 'site': None}]
        score = run.score_case(case, got)
        self.assertTrue(score['falsely_unique'])
        self.assertEqual(score['fp'], 1)

    def test_indirect_declaration_is_not_falsely_unique(self):
        case = self.case(**{'class': 'indirect', 'op': 'callees', 'certainty': 'indirect', 'site': 20,
                            'symbol': {'name': 'main', 'path': 'pkg/a.py', 'line': 19}, 'call_name': 'run',
                            'expect': [{'targets': [{'path': 'pkg/a.py', 'line': 2, 'name': 'run', 'role': 'declaration'},
                                                    {'path': 'pkg/a.py', 'line': 7, 'name': 'run', 'role': 'implementation'}],
                                        'sites': [20]}]})
        decl = run.score_case(case, [{'path': 'pkg/a.py', 'line': 2, 'end': 3, 'name': 'run', 'site': 20}])
        self.assertFalse(decl['falsely_unique'])
        impl = run.score_case(case, [{'path': 'pkg/a.py', 'line': 7, 'end': 8, 'name': 'run', 'site': 20}])
        self.assertTrue(impl['falsely_unique'])
        self.assertEqual(impl['tp'], 1)

    def test_miss_case_scores_empty_as_correct(self):
        case = self.case(**{'class': 'miss', 'expect': [], 'certainty': 'none'})
        self.assertTrue(run.score_case(case, [])['correct_empty'])
        self.assertEqual(run.score_case(case, [{'path': 'pkg/a.py', 'line': 19, 'end': 22, 'name': 'main',
                                                'site': None}])['fp'], 1)

    def test_optional_edges_neither_help_recall_nor_hurt_precision(self):
        case = self.case(expect=[{'targets': [{'path': 'pkg/a.py', 'line': 7, 'name': 'run'}], 'sites': [8]},
                                 {'targets': [{'path': 'pkg/a.py', 'line': 19, 'name': 'main'}], 'sites': [22],
                                  'required': False}])
        score = run.score_case(case, [{'path': 'pkg/a.py', 'line': 19, 'end': 22, 'name': 'main', 'site': 22}])
        self.assertEqual((score['tp'], score['fp'], score['fn'], score['optional']), (0, 0, 1, 1))

    def test_refs_scoring_is_by_line(self):
        case = self.case(op='refs', expect=[{'line_ref': {'path': 'pkg/a.py', 'line': 8}, 'kind': 'call'}])
        score = run.score_refs(case, [('pkg/a.py', 8), ('pkg/a.py', 3)])
        self.assertEqual((score['tp'], score['fp'], score['fn']), (1, 1, 0))


if __name__ == '__main__':
    unittest.main()
