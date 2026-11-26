#!/usr/bin/env python3
"""Time warm searches, alternating base and branch call by call (#308).

Usage: warm_ab.py <base_run> <base_binary> <branch_run> <branch_binary>
       [--corpus C ...] [--class K ...] [--rounds 30]

search_baseline.py runs each binary's whole batch in turn, so a load swing on a
shared host lands on one side. This runs every #288 query (queries.json) in
both modes, `cli` (one process per call) and `mcp` (one persistent server per
side), and alternates every call: each query goes to base and branch back to
back, and which side goes first flips each round. Round 0 is a warmup and is
not recorded. Prints a Markdown table per corpus, class and mode, and exits
non-zero if base and branch ever return different citations.
"""

import argparse
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'fts-tuning'))
from search_baseline import LABELS, BufferedLineReader, stop, write_all  # noqa: E402


class Server:
    """One `codecite mcp` process, initialized, answering search calls."""

    def __init__(self, binary, env):
        self.process = subprocess.Popen([binary, 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, env=env, bufsize=0)
        self.reader = BufferedLineReader(self.process.stdout)
        self.request_id = 0
        self.request('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                                    'clientInfo': {'name': 'warm-ab', 'version': '1'}})
        write_all(self.process.stdin, b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')

    def request(self, method, params):
        self.request_id += 1
        data = {'jsonrpc': '2.0', 'id': self.request_id, 'method': method, 'params': params}
        write_all(self.process.stdin, (json.dumps(data) + '\n').encode())
        deadline = time.monotonic() + 120
        while True:
            obj = json.loads(self.reader.readline(deadline))
            if obj.get('id') == self.request_id:
                if 'error' in obj:
                    raise RuntimeError(str(obj['error']))
                return obj['result']


def search(mode, binary, env, server, corpus, q):
    """Run one search; return (milliseconds, citations)."""
    t = time.perf_counter_ns()
    if mode == 'cli':
        command = [binary, 'search', corpus, q['text'], '--limit', '10', '--json']
        if 'path' in q:
            command += ['--path', q['path']]
        data = json.loads(subprocess.check_output(command, env=env, text=True))
    else:
        params = {'corpus': corpus, 'query': q['text'], 'limit': 10}
        if 'path' in q:
            params['path'] = q['path']
        response = server.request('tools/call', {'name': 'search', 'arguments': params})
        if response.get('isError'):
            raise RuntimeError(str(response))
        data = response['structuredContent']
    return (time.perf_counter_ns() - t) / 1e6, [hit['citation'] for hit in data['results']]


def loadavg():
    with open('/proc/loadavg') as f:
        return ' '.join(f.read().split()[:3])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name in ('base_run', 'base_binary', 'branch_run', 'branch_binary'):
        parser.add_argument(name)
    parser.add_argument('--corpus', action='append', help='only this corpus (repeatable); default all four')
    parser.add_argument('--class', dest='classes', action='append',
                        help='only queries of this class (repeatable); default every class')
    parser.add_argument('--rounds', type=int, default=30, help='recorded rounds, after one warmup round')
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error('rounds must be positive')
    labels = json.loads(LABELS.read_text())['corpora']
    if args.corpus:
        unknown = sorted(set(args.corpus) - set(labels))
        if unknown:
            parser.error(f'unknown corpus: {", ".join(unknown)}')
        labels = {c: q for c, q in labels.items() if c in args.corpus}
    if args.classes:
        labels = {c: [q for q in qs if q['class'] in args.classes] for c, qs in labels.items()}
        labels = {c: qs for c, qs in labels.items() if qs}
        if not labels:
            parser.error('no query has that class')
    sides = {'base': (args.base_run, args.base_binary), 'branch': (args.branch_run, args.branch_binary)}
    rng = random.Random(308)
    times = {}
    mismatches = []
    loads = []
    for corpus, queries in labels.items():
        envs = {side: dict(os.environ, XDG_DATA_HOME=f'{run}/data/{corpus}-1', XDG_CACHE_HOME=f'{run}/cache')
                for side, (run, _) in sides.items()}
        servers = {}
        try:
            for side, (_, binary) in sides.items():
                servers[side] = Server(binary, envs[side])
            load_before = loadavg()
            for i in range(args.rounds + 1):
                order = list(queries)
                rng.shuffle(order)
                pair = ('base', 'branch') if i % 2 else ('branch', 'base')
                for mode in ('cli', 'mcp'):
                    for q in order:
                        citations = {}
                        for side in pair:
                            ms, citations[side] = search(mode, sides[side][1], envs[side], servers[side], corpus, q)
                            if i:
                                times.setdefault((corpus, q['class'], mode, side), []).append(ms)
                        if citations['base'] != citations['branch']:
                            mismatches.append((corpus, q['id'], mode))
            loads.append((corpus, load_before, loadavg()))
        finally:
            for server in servers.values():
                stop(server.process)
        print(f'{corpus} complete', file=sys.stderr, flush=True)

    print(f'# Warm A/B, alternating every call, {args.rounds} rounds\n')
    print('Each query runs on base and branch back to back; the side that goes first flips each round.\n')
    for corpus, before, after in loads:
        print(f'- {corpus} load average (1/5/15 min): {before} → {after}')
    print('\n| Corpus | Class | Mode | Base p50 | Branch p50 | Change | Base p95 | Branch p95 | Change |')
    print('| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |')

    def p95(v):
        return sorted(v)[min(len(v) - 1, int(len(v) * 0.95))]

    for corpus, cls, mode in sorted({k[:3] for k in times}):
        base, branch = times[(corpus, cls, mode, 'base')], times[(corpus, cls, mode, 'branch')]
        row = [corpus, cls, mode]
        for stat in (statistics.median, p95):
            a, b = stat(base), stat(branch)
            row += [f'{a:.1f}', f'{b:.1f}', f'{(b - a) / a * 100:+.1f}%']
        print('| ' + ' | '.join(row[:3] + row[3:6] + row[6:]) + ' |')
    if mismatches:
        print(f'\nCitations differ: {mismatches}', file=sys.stderr)
        sys.exit(1)
    print('\nRanked citations identical between base and branch on every call.')


if __name__ == '__main__':
    main()
