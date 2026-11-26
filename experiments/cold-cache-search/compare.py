"""Put a base run and a branch run of the cold-cache measurement side by side.

Usage: compare.py <base_run> <branch_run> [--prefix cold] > results/summary.md

Each run directory holds `index.jsonl` (from run_index.py), the
`<prefix>-summary.json` and `<prefix>-search.jsonl` files written by
`../fts-tuning/search_baseline.py --cold`, and optionally `<prefix>-load.json`,
a list of `/proc/loadavg` lines taken before and after the search run.

It fails when any query's ranked citations differ between the two runs: the
change under test moves storage, not ranking.
"""

import argparse
import json
import sys
from pathlib import Path


def load_jsonl(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def citations(run, prefix):
    # Every round of a run gives the same ranking (search_baseline checks), so keep one.
    out = {}
    for r in load_jsonl(run / f'{prefix}-search.jsonl'):
        out[(r['corpus'], r['id'])] = r['citations']
    return out


def change(base, branch):
    if not base:
        return 'n/a'
    return f'{(branch - base) / base * 100:+.1f}%'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base', type=Path)
    parser.add_argument('branch', type=Path)
    parser.add_argument('--prefix', default='cold')
    args = parser.parse_args()

    base_cites, branch_cites = citations(args.base, args.prefix), citations(args.branch, args.prefix)
    if base_cites.keys() != branch_cites.keys():
        sys.exit('the two runs measured different queries')
    differ = sorted(k for k in base_cites if base_cites[k] != branch_cites[k])
    if differ:
        sys.exit('ranked citations differ: ' + ', '.join(f'{c} {q}' for c, q in differ))

    def summary(run):
        rows = json.loads((run / f'{args.prefix}-summary.json').read_text())
        return {(r['corpus'], r['class'], r['mode']): r for r in rows}

    base_rows, branch_rows = summary(args.base), summary(args.branch)
    print('# Cold-cache search: base vs branch (#308)\n')
    print(f'Ranked citations identical for all {len(base_cites)} queries.\n')

    for name, run in (('base', args.base), ('branch', args.branch)):
        load = run / f'{args.prefix}-load.json'
        if load.exists():
            samples = json.loads(load.read_text())
            print(f'- {name} load average (1/5/15 min): ' + ' → '.join(' '.join(s.split()[:3]) for s in samples))
    print()

    print('## Search latency (ms)\n')
    print('| Corpus | Class | Mode | Base p50 | Branch p50 | Change | Base p95 | Branch p95 | Change |')
    print('| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |')
    for key in sorted(base_rows):
        if key not in branch_rows:
            continue
        b, n = base_rows[key], branch_rows[key]
        print(f'| {key[0]} | {key[1]} | {key[2]} | {b["p50_ms"]:.1f} | {n["p50_ms"]:.1f} | '
              f'{change(b["p50_ms"], n["p50_ms"])} | {b["p95_ms"]:.1f} | {n["p95_ms"]:.1f} | '
              f'{change(b["p95_ms"], n["p95_ms"])} |')

    print('\n## Index\n')
    print('| Corpus | Base bytes | Branch bytes | Change | Base wall s | Branch wall s | Change |')
    print('| --- | ---: | ---: | ---: | ---: | ---: | ---: |')
    base_index = {r['corpus']: r for r in load_jsonl(args.base / 'index.jsonl')}
    branch_index = {r['corpus']: r for r in load_jsonl(args.branch / 'index.jsonl')}
    for corpus in base_index:
        if corpus not in branch_index:
            continue
        b, n = base_index[corpus], branch_index[corpus]
        print(f'| {corpus} | {b["database_bytes"]:,} | {n["database_bytes"]:,} | '
              f'{change(b["database_bytes"], n["database_bytes"])} | {b["wall_seconds"]:.1f} | '
              f'{n["wall_seconds"]:.1f} | {change(b["wall_seconds"], n["wall_seconds"])} |')


if __name__ == '__main__':
    main()
