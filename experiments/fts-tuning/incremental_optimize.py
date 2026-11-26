#!/usr/bin/env python3
"""Paired synthetic same-content FTS rewrites on disposable identity-bench copies."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def stats(db):
    sql = "SELECT (SELECT count(*) FROM chunk) chunks, (SELECT count(distinct segid) FROM chunk_fts_idx) segments, (SELECT sum(pgsize) FROM dbstat WHERE name='chunk_fts_data') fts_data_bytes;"
    r = json.loads(subprocess.check_output(['sqlite3', '-readonly', '-json', db, sql], text=True))[0]
    r['database_bytes'] = db.stat().st_size
    return r


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('baseline', type=Path, help='completed baseline run directory')
    p.add_argument('output', type=Path, help='new private directory')
    p.add_argument('--helper', type=Path, required=True, help='compiled incremental_rewrite')
    p.add_argument('--bin-dir', type=Path, required=True)
    a = p.parse_args()
    baseline = a.baseline.resolve(strict=True)
    helper = a.helper.resolve(strict=True)
    binary = a.bin_dir.resolve(strict=True) / 'codecite'
    bench = binary.with_name('codecite-bench')
    meta = json.loads((baseline / 'index-metadata.json').read_text())
    for exe in (binary, bench):
        if sha(exe) != meta['binary_sha256'][exe.name]:
            raise ValueError(f'wrong binary: {exe.name}')
    labels = ROOT / 'evals/retrieval/queries.toml'
    manifest = ROOT / 'evals/retrieval/identity-bench.toml'
    if sha(manifest) != meta['manifest_sha256']['identity-bench']:
        raise ValueError('manifest differs from baseline')
    source = (baseline / 'data/identity-bench-1/codecite/identity-bench.db').resolve(strict=True)
    wal = Path(str(source) + '-wal')
    if wal.exists() and wal.stat().st_size:
        raise ValueError('source has uncheckpointed WAL')
    root = a.output.absolute()
    root.mkdir()
    cache = root / 'cache'
    cache.mkdir()
    output = root / 'records.jsonl'
    query_data = tomllib.loads(labels.read_text())['query']
    # Fixed spread through frozen labels, independent of system results.
    probes = [(i, query_data[i]['text']) for i in (0, 15, 30, 45, 60, 75, 90, 105)]
    summary = {'source_db_sha256': sha(source), 'queries_sha256': sha(labels),
               'binary_sha256': sha(binary), 'manifest_sha256': sha(manifest),
               'fractions': [1, 10, 50], 'probe_indexes': [i for i, _ in probes],
               'rounds': 5, 'cache': 'warm sweep, sequential interleaved CLI calls, no eviction'}
    (root / 'metadata.json').write_text(json.dumps(summary, indent=2) + '\n')
    baseline_env = dict(os.environ, XDG_DATA_HOME=str(baseline / 'data/identity-bench-1'),
                        XDG_CACHE_HOME=str(cache))
    baseline_quality = json.loads(subprocess.check_output([bench, '--queries', labels, '--codecite', binary, '--json'], env=baseline_env, text=True))
    for fraction in (1, 10, 50):
        homes = {}
        measures = {}
        for arm in ('control', 'optimize'):
            home = root / str(fraction) / arm
            db = home / 'codecite/identity-bench.db'
            db.parent.mkdir(parents=True)
            shutil.copyfile(source, db)
            if sha(db) != summary['source_db_sha256']:
                raise ValueError('copied database differs')
            before = stats(db)
            wal = Path(str(db) + '-wal')
            start = time.perf_counter_ns()
            proc = subprocess.Popen([helper, db, str(fraction), arm],
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            peak = 0
            while proc.poll() is None:
                try:
                    peak = max(peak, wal.stat().st_size)
                except FileNotFoundError:
                    pass
                time.sleep(.001)
            out, err = proc.communicate()
            elapsed = (time.perf_counter_ns() - start) / 1e9
            if proc.returncode:
                raise RuntimeError(f'{fraction} {arm}: {err}')
            measures[arm] = dict(before=before, after=stats(db), timings=json.loads(out),
                                 wall_seconds=elapsed, peak_wal_bytes=peak, wal_sampling_ms=1)
            homes[arm] = home
        # Whole frozen retrieval benchmark: compare every outcome, without saving private paths.
        quality = {}
        outcomes = {}
        for arm, home in homes.items():
            env = dict(os.environ, XDG_DATA_HOME=str(home), XDG_CACHE_HOME=str(cache))
            data = json.loads(subprocess.check_output([bench, '--queries', labels, '--codecite', binary, '--json'], env=env, text=True))
            quality[arm] = data['all']
            outcomes[arm] = data['outcomes']
        equal_outcomes = outcomes['control'] == outcomes['optimize']
        equal_baseline = outcomes['control'] == baseline_quality['outcomes']
        if not equal_outcomes or not equal_baseline:
            raise ValueError(f'{fraction}: frozen quality outcomes differ')
        records = []
        distinct = {'control': set(), 'optimize': set()}
        for iteration in range(6):
            for index, text in probes:
                result = {}
                arms = ('control', 'optimize') if iteration % 2 == 0 else ('optimize', 'control')
                for arm in arms:
                    env = dict(os.environ, XDG_DATA_HOME=str(homes[arm]), XDG_CACHE_HOME=str(cache))
                    t = time.perf_counter_ns()
                    value = json.loads(subprocess.check_output([binary, 'search', 'identity-bench', text, '--limit', '10', '--json'], env=env, text=True))
                    ms = (time.perf_counter_ns() - t) / 1e6
                    citations = [v['citation'] for v in value['results']]
                    distinct[arm].update(citations)
                    result[arm] = citations
                    records.append(dict(query_index=index, iteration=iteration, arm=arm, milliseconds=ms))
                if result['control'] != result['optimize']:
                    raise ValueError(f'{fraction} query {index}: different results')
        checks = {}
        for arm, citations in distinct.items():
            env = dict(os.environ, XDG_DATA_HOME=str(homes[arm]), XDG_CACHE_HOME=str(cache))
            failures = 0
            for citation in sorted(citations):
                read = subprocess.run([binary, 'read', 'identity-bench', citation, '--json'], env=env, capture_output=True)
                failures += read.returncode != 0
            checks[arm] = {'distinct': len(citations), 'failures': failures}
            if failures:
                raise ValueError(f'{fraction} {arm}: citation failures')
        record = dict(fraction=fraction, measures=measures, frozen_quality=quality,
                      same_frozen_outcomes=equal_outcomes, same_baseline_outcomes=equal_baseline,
                      citation_checks=checks, paired_reads=records)
        with output.open('a') as f:
            f.write(json.dumps(record, sort_keys=True) + '\n')
        print(f'{fraction}% complete', flush=True)


if __name__ == '__main__':
    main()
