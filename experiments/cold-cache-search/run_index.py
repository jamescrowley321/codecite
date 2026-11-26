#!/usr/bin/env python3
"""Index the four #288 corpora once each with one binary, for search_baseline.py (#308).

Writes `<run>/data/<corpus>-1`, a private copy of the codecite cache in
`<run>/cache`, `<run>/index-metadata.json` (the binary's SHA-256, as
search_baseline.py checks it) and `<run>/index.jsonl` (chunks, database bytes
and fresh wall seconds per corpus). Refuses a dirty or wrong local checkout.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE.parent / 'fts-tuning'))
from index_baseline import check_checkout, digest, inspect_database, output  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_dir', type=Path, help='new directory; must not exist')
    parser.add_argument('--binary', type=Path, required=True, help='release codecite binary')
    parser.add_argument('--checkout-root', type=Path, required=True,
                        help='directory holding clean linux, kubernetes and keycloak checkouts')
    parser.add_argument('--cache', type=Path, default=Path.home() / '.cache/codecite',
                        help='codecite cache to copy (repos and documents)')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    locals_ = tomllib.loads((REPO / 'experiments/fts-tuning/corpora.toml').read_text())['corpus']
    checkouts = {}
    for local in locals_:
        checkout = (args.checkout_root / local['name']).resolve(strict=True)
        check_checkout(checkout, local['commit'])
        checkouts[local['name']] = checkout

    run = args.run_dir.resolve()
    run.mkdir(parents=True)
    cache = run / 'cache'
    # A copy, not a link: indexing a git source checks out and cleans its cache.
    for part in ('repos', 'documents'):
        shutil.copytree(args.cache / part, cache / 'codecite' / part, symlinks=True)
    manifests = run / 'manifests'
    manifests.mkdir()
    paths = {'fhir': REPO / 'corpora/fhir.toml'}
    for local in locals_:
        name = local['name']
        settings = dict(kind='local', path=str(checkouts[name]),
                        **{k: v for k, v in local.items() if k != 'commit'})
        manifest = manifests / f'{name}.toml'
        manifest.write_text(f'name = "{name}"\n\n[[source]]\n' + ''.join(
            f'{key} = {json.dumps(value, ensure_ascii=False)}\n' for key, value in settings.items()))
        paths[name] = manifest

    metadata = dict(
        source_revision=output('git', 'rev-parse', 'HEAD', cwd=REPO),
        codecite_version=output(binary, '--version'),
        binary_sha256={'codecite': digest(binary)},
        manifest_sha256={name: digest(path) for name, path in paths.items()},
        local_pins={p['name']: p['commit'] for p in locals_},
    )
    (run / 'index-metadata.json').write_text(json.dumps(metadata, sort_keys=True, indent=2) + '\n')
    raw = run / 'index.jsonl'
    for name, manifest in paths.items():
        data = run / 'data' / f'{name}-1'
        data.mkdir(parents=True)
        env = dict(os.environ, XDG_DATA_HOME=str(data), XDG_CACHE_HOME=str(cache))
        print(f'{name}: indexing', flush=True)
        result = subprocess.run(
            ['/usr/bin/time', '-f', '%e %M', '-o', data / 'time.txt',
             binary, 'index', name, '--manifest', manifest],
            env=env, capture_output=True, text=True, cwd=REPO)
        wall, rss = (data / 'time.txt').read_text().splitlines()[-1].split()
        db = data / 'codecite' / f'{name}.db'
        record = dict(corpus=name, exit=result.returncode, wall_seconds=float(wall),
                      peak_rss_kib=int(rss), stderr=result.stderr)
        if result.returncode == 0:
            record.update(database_bytes=db.stat().st_size, **inspect_database(db))
        with raw.open('a') as stream:
            stream.write(json.dumps(record, sort_keys=True) + '\n')
        result.check_returncode()
        if record['status'] != 'succeeded' or record['dirty_documents']:
            raise ValueError(f'{name}: index did not finish clean')
        for local in locals_:
            if local['name'] == name:
                check_checkout(checkouts[name], local['commit'])
        print(f'{name}: {record["chunks"]} chunks, {record["database_bytes"]} bytes, '
              f'{record["wall_seconds"]} s', flush=True)


if __name__ == '__main__':
    main()
