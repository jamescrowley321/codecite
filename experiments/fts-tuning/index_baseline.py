#!/usr/bin/env python3
"""Run fresh/no-op pairs serially and record raw index baselines for #255."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tomllib

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent


def output(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def check_checkout(path, commit):
    git = ['git', '--no-optional-locks', '-c', 'core.fsmonitor=false', '-C', path]
    if Path(output(*git, 'rev-parse', '--show-toplevel')) != path:
        raise ValueError(f'{path} is not a checkout root')
    if output(*git, 'rev-parse', 'HEAD') != commit:
        raise ValueError(f'{path} is not at {commit}')
    if output(*git, 'status', '--porcelain', '--untracked-files=all'):
        raise ValueError(f'{path} is not clean')


def inspect_database(db):
    sql = '''SELECT
      (SELECT count(*) FROM document) AS documents,
      (SELECT count(*) FROM document WHERE dirty) AS dirty_documents,
      (SELECT count(*) FROM chunk) AS chunks,
      (SELECT count(*) FROM symbol) AS symbols,
      (SELECT count(DISTINCT segid) FROM chunk_fts_idx) AS fts_segments,
      (SELECT sum(pgsize) FROM dbstat WHERE name = 'chunk_fts_data') AS fts_data_bytes,
      (SELECT sum(pgsize) FROM dbstat WHERE name GLOB 'chunk_fts_*') AS fts_bytes,
      (SELECT value FROM meta WHERE key = 'schema_version') AS schema_version,
      id, status, written, unchanged, removed, chunks AS chunks_written,
      chunker_version, terms_version, tokenizer, manifest_sha256
      FROM index_run ORDER BY id DESC LIMIT 1;'''
    return json.loads(output('sqlite3', '-readonly', '-json', db, sql))[0]


def run_pair(binary, corpus, manifest, data, cache, raw):
    data.mkdir()
    env = dict(os.environ, XDG_DATA_HOME=str(data), XDG_CACHE_HOME=str(cache))
    db = data / 'codecite' / f'{corpus}.db'
    for phase in ('fresh', 'unchanged'):
        print(f'{data.name}: {phase}', flush=True)
        result = subprocess.run(
            ['/usr/bin/time', '-f', '%e %M', '-o', data / 'time.txt',
             binary, 'index', corpus, '--manifest', manifest],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            cwd=REPO,
        )
        wall, rss = (data / 'time.txt').read_text().splitlines()[-1].split()
        record = dict(corpus=corpus, pair=data.name, phase=phase, exit=result.returncode,
                      wall_seconds=float(wall), peak_rss_kib=int(rss),
                      output=result.stdout.splitlines()[1:], stderr=result.stderr)
        # The first stdout line only names the temporary database's local path.
        if result.returncode == 0:
            record.update(database_bytes=db.stat().st_size, **inspect_database(db))
        with raw.open('a') as stream:
            stream.write(json.dumps(record, sort_keys=True) + '\n')
        result.check_returncode()
        if record['status'] != 'succeeded' or record['manifest_sha256'] != digest(manifest):
            raise ValueError(f'inconsistent index metadata: {data.name} {phase}')
        if record['dirty_documents']:
            raise ValueError(f'dirty indexed files: {data.name}')
        expected = (record['documents'], 0, 0, record['chunks']) if phase == 'fresh' else (
            0, record['documents'], 0, 0)
        if tuple(record[key] for key in ('written', 'unchanged', 'removed', 'chunks_written')) != expected:
            raise ValueError(f'not a {phase} index: {data.name}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_dir', type=Path, help='new private mktemp directory')
    parser.add_argument('--bin-dir', type=Path, required=True, help='locked release binaries')
    parser.add_argument('--checkout-root', type=Path, required=True)
    parser.add_argument('--sqlite-version', type=Path, required=True,
                        help='sqlite_version helper linked to the locked release rusqlite')
    args = parser.parse_args()
    binary, bench = (args.bin_dir.resolve() / name for name in ('codecite', 'codecite-bench'))
    sqlite_version = args.sqlite_version.resolve(strict=True)
    run = args.run_dir.resolve(strict=True)
    cache = run / 'cache'
    cache.mkdir(exist_ok=True)
    manifests = run / 'manifests'
    manifests.mkdir()
    data = run / 'data'
    data.mkdir()
    raw = run / 'index.jsonl'
    raw.touch(exist_ok=False)

    paths = {'fhir': REPO / 'corpora/fhir.toml'}
    locals_ = tomllib.loads((HERE / 'corpora.toml').read_text())['corpus']
    for local in locals_:
        name = local['name']
        checkout = (args.checkout_root / name).resolve(strict=True)
        check_checkout(checkout, local['commit'])
        manifest = manifests / f'{name}.toml'
        settings = dict(kind='local', path=str(checkout), **{k: v for k, v in local.items() if k != 'commit'})
        manifest.write_text(f'name = "{name}"\n\n[[source]]\n' + ''.join(
            f'{key} = {json.dumps(value, ensure_ascii=False)}\n' for key, value in settings.items()))
        paths[name] = manifest
    paths['identity'] = REPO / 'corpora/identity.toml'
    frozen = [REPO / 'evals/retrieval/identity-bench.toml', *sorted(REPO.glob('evals/history/*/manifest.toml'))]
    for path in frozen:
        paths[tomllib.loads(path.read_text())['name']] = path

    inspection_sqlite = output('sqlite3', '-version')
    bundled_sqlite = output(sqlite_version)
    if inspection_sqlite.partition(' ')[0] != bundled_sqlite:
        raise ValueError('SQLite CLI differs from the bundled SQLite')
    metadata = dict(
        source_revision=output('git', 'rev-parse', 'HEAD', cwd=REPO),
        rustc=output('rustc', '-Vv', cwd=REPO), codecite_version=output(binary, '--version'),
        inspection_sqlite=inspection_sqlite, bundled_sqlite=bundled_sqlite,
        sqlite_version_helper_sha256=digest(sqlite_version),
        binary_sha256={p.name: digest(p) for p in (binary, bench)},
        manifest_sha256={name: digest(path) for name, path in paths.items()},
        local_pins={p['name']: p['commit'] for p in locals_},
    )
    (run / 'index-metadata.json').write_text(json.dumps(metadata, sort_keys=True, indent=2) + '\n')
    for name, manifest in paths.items():
        pairs = 1 if manifest in frozen else 2
        for pair in range(1, pairs + 1):
            run_pair(binary, name, manifest, data / f'{name}-{pair}', cache, raw)
        for local in locals_:
            if local['name'] == name:
                check_checkout((args.checkout_root / name).resolve(strict=True), local['commit'])


if __name__ == '__main__':
    main()
