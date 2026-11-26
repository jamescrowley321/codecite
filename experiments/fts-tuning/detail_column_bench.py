#!/usr/bin/env python3
"""Run the frozen retrieval and history labels against a baseline and a detail=column build.

codecite-bench stops at the first query whose search fails, and the detail=column
variant fails on phrase queries. So the baseline is scored on each whole labels
file, and the variant one query at a time, recording each failure. A failed
query counts as a miss in the variant's scores.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

HISTORY = ('blind-peer-review', 'codecite', 'identity-model', 'identity-stack', 'terraform-provider-descope')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def label_sets(source):
    """(label, corpus, manifest, queries) for the retrieval labels and each history repository.

    History repositories are recorded as history-1 to history-5, in the order of the
    committed history-N summaries, and their query ids as ordinals.
    """
    sets = [('identity-bench', 'identity-bench', source / 'evals/retrieval/identity-bench.toml',
             source / 'evals/retrieval/queries.toml')]
    for i, repo in enumerate(HISTORY, 1):
        d = source / 'evals/history' / repo
        sets.append((f'history-{i}', f'history-{repo}', d / 'manifest.toml', d / 'queries.toml'))
    return sets


def reason(stderr):
    """The innermost cause of a failed bench run, without the query id."""
    lines = [line.strip() for line in stderr.splitlines() if line.strip()]
    causes = [line.split(': ', 1)[1] for line in lines if line.startswith('0: ')]
    return causes[-1] if causes else lines[-1] if lines else 'failed'


def scores(ranks):
    n = len(ranks)
    return {'queries': n,
            'recall_at_1': sum(r == 1 for r in ranks) / n,
            'recall_at_10': sum(r is not None and r <= 10 for r in ranks) / n,
            'mrr': sum(1 / r for r in ranks if r is not None) / n}


def bench(scorer, codecite, queries, env):
    out = subprocess.run([scorer, '--queries', queries, '--codecite', codecite, '--json'],
                         env=env, capture_output=True, text=True)
    if out.returncode:
        return None, out.stderr.strip() or f'exit {out.returncode}'
    return json.loads(out.stdout), ''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path, help='new directory for the run')
    parser.add_argument('--base', type=Path, required=True, help='release dir of the unchanged build, with codecite-bench')
    parser.add_argument('--variant', type=Path, required=True, help='the detail=column codecite binary')
    parser.add_argument('--source', type=Path, required=True, help='checkout at the measured revision')
    parser.add_argument('--cache', type=Path, required=True, help='private copy of the codecite cache')
    args = parser.parse_args()
    base_dir = args.base.resolve(strict=True)
    scorer = (base_dir / 'codecite-bench').resolve(strict=True)
    builds = {'base': (base_dir / 'codecite').resolve(strict=True), 'variant': args.variant.resolve(strict=True)}
    source = args.source.resolve(strict=True)
    sets = label_sets(source)
    for _, _, manifest, queries in sets:
        manifest.resolve(strict=True)
        queries.resolve(strict=True)
    cache = args.cache.resolve(strict=True)
    revision = subprocess.check_output(['git', '-C', source, 'rev-parse', 'HEAD'], text=True).strip()
    out = args.output.resolve()
    out.mkdir()
    (out / 'metadata.json').write_text(json.dumps({
        'source_revision': revision,
        'binary_sha256': {'base': sha(builds['base']), 'variant': sha(builds['variant']), 'codecite-bench': sha(scorer)},
        'manifest_sha256': {label: sha(m) for label, _, m, _ in sets},
        'queries_sha256': {label: sha(q) for label, _, _, q in sets},
    }, indent=2) + '\n')
    summary = []
    for label, codecite in builds.items():
        env = dict(os.environ, XDG_DATA_HOME=str(out / f'data-{label}'), XDG_CACHE_HOME=str(cache))
        for _, corpus, manifest, _ in sets:
            subprocess.run([codecite, 'index', corpus, '--manifest', manifest], env=env, check=True,
                           stdout=subprocess.DEVNULL)
    base_env = dict(os.environ, XDG_DATA_HOME=str(out / 'data-base'), XDG_CACHE_HOME=str(cache))
    variant_env = dict(os.environ, XDG_DATA_HOME=str(out / 'data-variant'), XDG_CACHE_HOME=str(cache))
    for label, corpus, _, queries in sets:
        report, error = bench(scorer, builds['base'], queries, base_env)
        if report is None:
            raise RuntimeError(f'{label}: baseline bench failed: {reason(error)}')
        base = {o['id']: o['rank'] for o in report['outcomes']}
        file = tomllib.loads(queries.read_text())
        records = []
        for n, q in enumerate(file['query'], 1):
            with tempfile.NamedTemporaryFile('w', suffix='.toml', dir=out, delete=False) as one:
                one.write(f'corpus = {json.dumps(file["corpus"])}\n\n[[query]]\n')
                for key in ('id', 'class', 'text', 'source'):
                    if key in q:
                        one.write(f'{key} = {json.dumps(q[key])}\n')
                one.write(f'answers = {json.dumps(q["answers"])}\n')
            try:
                single, error = bench(scorer, builds['variant'], Path(one.name), variant_env)
            finally:
                os.unlink(one.name)
            rank = single['outcomes'][0]['rank'] if single else None
            records.append({'id': q['id'] if label == corpus else f'q{n}', 'class': q['class'],
                            'base_rank': base[q['id']], 'variant_rank': rank,
                            'variant_error': reason(error) if error else ''})
        (out / f'{label}-queries.json').write_text(json.dumps(records, indent=2) + '\n')
        answered = [r for r in records if not r['variant_error']]
        summary.append({
            'corpus': label,
            'queries': len(records),
            'variant_errors': len(records) - len(answered),
            'base': scores([r['base_rank'] for r in records]),
            'variant_errors_as_misses': scores([r['variant_rank'] for r in records]),
            'answered_by_both': len(answered),
            'answered_same_rank': sum(r['base_rank'] == r['variant_rank'] for r in answered),
        })
        print(label, 'complete', flush=True)
    (out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')


if __name__ == '__main__':
    main()
