#!/usr/bin/env python3
"""Measure frozen source-label searches with CLI and persistent MCP."""

import argparse
import collections
import hashlib
import json
import math
import os
from pathlib import Path
import random
import re
import select
import statistics
import subprocess
import time

HERE = Path(__file__).resolve().parent
LABELS = HERE / 'queries.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_dir', type=Path)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=5)
    parser.add_argument('--prefix', default='frozen', help='new output filename prefix')
    args = parser.parse_args()
    if not re.fullmatch(r'[a-zA-Z0-9_-]+', args.prefix):
        parser.error('prefix must contain only letters, digits, underscores or hyphens')
    if args.rounds < 1:
        parser.error('rounds must be positive')
    run = args.run_dir.resolve(strict=True)
    binary = args.binary.resolve(strict=True)
    metadata = json.loads((run / 'index-metadata.json').read_text())
    with binary.open('rb') as f:
        if hashlib.file_digest(f, 'sha256').hexdigest() != metadata['binary_sha256']['codecite']:
            raise ValueError('binary differs from index baseline')
    labels = json.loads(LABELS.read_text())['corpora']
    output = run / f'{args.prefix}-search.jsonl'
    output.touch(exist_ok=False)
    (run / f'{args.prefix}-metadata.json').write_text(json.dumps({
        'query_sha256': hashlib.sha256(LABELS.read_bytes()).hexdigest(),
        'rounds': args.rounds, 'seed': 255, 'limit': 10,
        'cache': 'one untimed sweep per mode; warm OS page cache; no cache eviction',
        'cli': 'one new process per query, startup included',
        'mcp': 'one initialized persistent process per corpus; store reopened per call',
    }, indent=2) + '\n')
    all_records = []
    for corpus, queries in labels.items():
        data_home = run / 'data' / f'{corpus}-1'
        env = dict(os.environ, XDG_DATA_HOME=str(data_home))
        for q in queries:
            if q['class'] not in ('rare-term', 'broad-term'):
                continue
            if not re.fullmatch(r'[a-z0-9]+', q['text']):
                raise ValueError(f"stratum requires one lowercase token: {q['id']}")
            sql = f'''SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH '"{q['text']}"';'''
            count = int(subprocess.check_output(['sqlite3', '-readonly', data_home / 'codecite' / f'{corpus}.db', sql], text=True))
            if (q['class'] == 'rare-term' and not 1 <= count <= 200) or (q['class'] == 'broad-term' and count < 10_000):
                raise ValueError(f"match-count stratum violated: {q['id']} {count}")
            q['matches'] = count
        process = subprocess.Popen([binary, 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, env=env)
        request_id = 0

        def request(method, params):
            nonlocal request_id
            request_id += 1
            data = {'jsonrpc': '2.0', 'id': request_id, 'method': method, 'params': params}
            process.stdin.write((json.dumps(data) + '\n').encode())
            process.stdin.flush()
            while True:
                if not select.select([process.stdout], [], [], 120)[0]:
                    raise TimeoutError(f'{corpus}: {method}')
                line = process.stdout.readline()
                if not line:
                    raise RuntimeError(f'{corpus}: server exited')
                obj = json.loads(line)
                if obj.get('id') == request_id:
                    if 'error' in obj:
                        raise RuntimeError(str(obj['error']))
                    return obj['result']

        try:
            request('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                                   'clientInfo': {'name': 'fts-tuning', 'version': '1'}})
            process.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            process.stdin.flush()
            rng = random.Random(255)
            references = {}
            for mode in ('cli', 'mcp'):
                for iteration in range(args.rounds + 1):
                    order = list(queries)
                    rng.shuffle(order)
                    for q in order:
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
                            response = request('tools/call', {'name': 'search', 'arguments': params})
                            if response.get('isError'):
                                raise RuntimeError(str(response))
                            data = response['structuredContent']
                        ms = (time.perf_counter_ns() - t) / 1_000_000
                        citations = [hit['citation'] for hit in data['results']]
                        if q['id'] in references and references[q['id']] != citations:
                            raise ValueError(f'changed ranking: {corpus} {q["id"]}')
                        references[q['id']] = citations
                        rank = None
                        for i, citation in enumerate(citations, 1):
                            m = re.search(r':([^:]+)#L(\d+)(?:-L(\d+))?$', citation)
                            if m and any(m.group(1) == a['path'] and int(m.group(2)) <= a['line'] <= int(m.group(3) or m.group(2)) for a in q['answers']):
                                rank = i
                                break
                        record = {'corpus': corpus, 'id': q['id'], 'class': q['class'], 'mode': mode,
                                  'round': iteration, 'milliseconds': ms, 'rank': rank, 'citations': citations,
                                  'scored': q.get('score', True), 'fts_matches': q.get('matches')}
                        with output.open('a') as f:
                            f.write(json.dumps(record, sort_keys=True) + '\n')
                        all_records.append(record)
            citation_check = []
            for citation in sorted({c for citations in references.values() for c in citations}):
                result = subprocess.run([binary, 'read', corpus, citation, '--json'], env=env,
                                        capture_output=True, text=True)
                citation_check.append({'citation': citation, 'exit': result.returncode,
                                       'error': result.stderr if result.returncode else ''})
            (run / f'{args.prefix}-{corpus}-citations.json').write_text(json.dumps(citation_check, indent=2) + '\n')
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        print(corpus, 'complete', flush=True)
    groups = collections.defaultdict(list)
    for r in all_records:
        if r['round']:
            groups[(r['corpus'], r['class'], r['mode'])].append(r)
    summary = []
    for (corpus, cls, mode), records in sorted(groups.items()):
        times = sorted(r['milliseconds'] for r in records)
        ranks = {r['id']: r['rank'] for r in records if r['scored']}
        row = {'corpus': corpus, 'class': cls, 'mode': mode, 'samples': len(times),
               'queries': len({r['id'] for r in records}), 'scored_queries': len(ranks), 'p50_ms': statistics.median(times),
               'p95_ms': times[math.ceil(.95 * len(times)) - 1]}
        if cls == 'miss':
            row['nonempty'] = sum(bool(r['citations']) for r in records)
        elif ranks:
            row.update(recall_at_1=sum(r == 1 for r in ranks.values()) / len(ranks),
                       recall_at_10=sum(r is not None for r in ranks.values()) / len(ranks),
                       mrr=sum(1 / r for r in ranks.values() if r is not None) / len(ranks))
        summary.append(row)
    (run / f'{args.prefix}-summary.json').write_text(json.dumps(summary, indent=2) + '\n')


if __name__ == '__main__':
    main()
