"""Time a few warm CLI searches, alternating base and branch call by call.

Usage: warm_ab.py <base_run> <base_binary> <branch_run> <branch_binary> <corpus> [--rounds 80]

search_baseline.py runs each binary's whole batch in turn, so a load swing on a
shared host lands on one side; this alternates every call (and which side goes
first each round) to judge a warm rise that batch runs flagged.
"""

import argparse
import os
import statistics
import subprocess
import time

QUERIES = {
    'fhir': [('path-filter', 'FhirContext', 'hapi-fhir-base/src/main/java/ca/uhn/fhir/context/FhirContext.java'),
             ('rare-term', 'frozen', None),
             ('miss', 'zzqxnomatch', None)],
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('base_run', 'base_binary', 'branch_run', 'branch_binary', 'corpus'):
        parser.add_argument(name)
    parser.add_argument('--rounds', type=int, default=80)
    args = parser.parse_args()
    sides = {'base': (args.base_run, args.base_binary), 'branch': (args.branch_run, args.branch_binary)}
    times = {}
    with open('/proc/loadavg') as f:
        before = f.read().split()[:3]
    for i in range(args.rounds):
        for side in (('base', 'branch') if i % 2 else ('branch', 'base')):
            run, binary = sides[side]
            env = dict(os.environ, XDG_DATA_HOME=f'{run}/data/{args.corpus}-1', XDG_CACHE_HOME=f'{run}/cache')
            for cls, text, path in QUERIES[args.corpus]:
                command = [binary, 'search', args.corpus, text, '--limit', '10', '--json']
                if path:
                    command += ['--path', path]
                t = time.perf_counter_ns()
                subprocess.run(command, env=env, stdout=subprocess.DEVNULL, check=True)
                times.setdefault((cls, side), []).append((time.perf_counter_ns() - t) / 1e6)
    with open('/proc/loadavg') as f:
        after = f.read().split()[:3]
    print(f'# Warm A/B, {args.corpus}, {args.rounds} alternating rounds\n')
    print(f'Load average (1/5/15 min): {" ".join(before)} → {" ".join(after)}\n')
    print('| Class | Side | p50 ms | p95 ms | min ms |')
    print('| --- | --- | ---: | ---: | ---: |')
    for (cls, side), v in sorted(times.items()):
        v.sort()
        print(f'| {cls} | {side} | {statistics.median(v):.1f} | {v[int(len(v) * 0.95)]:.1f} | {v[0]:.1f} |')


if __name__ == '__main__':
    main()
