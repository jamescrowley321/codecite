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


class BufferedLineReader:
    def __init__(self, stream):
        self.stream = stream
        self.buffer = bytearray()

    def readline(self, deadline):
        while True:
            end = self.buffer.find(b'\n')
            if end >= 0:
                line = bytes(self.buffer[:end + 1])
                del self.buffer[:end + 1]
                return line
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError
            ready, _, _ = select.select([self.stream], [], [], remaining)
            if not ready:
                raise TimeoutError
            chunk = os.read(self.stream.fileno(), 65536)
            if not chunk:
                raise EOFError
            self.buffer.extend(chunk)


def write_all(stream, data):
    remaining = memoryview(data)
    while remaining:
        written = stream.write(remaining)
        if written is None:
            raise BlockingIOError('MCP stdin write made no progress')
        if written <= 0:
            raise OSError('MCP stdin write made no progress')
        remaining = remaining[written:]
    stream.flush()


def evict(paths):
    """Drop each existing file's clean pages from the OS page cache. Needs no root."""
    for path in paths:
        try:
            fd = os.open(path, os.O_RDONLY)
        except FileNotFoundError:
            continue
        try:
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
        finally:
            os.close(fd)


def stop(process):
    """Terminate a server and wait for it, killing it after ten seconds."""
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def check_strata(db, queries):
    """Verify the predeclared rare/broad match counts and record them on each query."""
    for q in queries:
        if q['class'] not in ('rare-term', 'broad-term'):
            continue
        if not re.fullmatch(r'[a-z0-9]+', q['text']):
            raise ValueError(f"stratum requires one lowercase token: {q['id']}")
        sql = f'''SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH '"{q['text']}"';'''
        count = int(subprocess.check_output(['sqlite3', '-readonly', db, sql], text=True))
        if (q['class'] == 'rare-term' and not 1 <= count <= 200) or (q['class'] == 'broad-term' and count < 10_000):
            raise ValueError(f"match-count stratum violated: {q['id']} {count}")
        q['matches'] = count


def read_back_citations(binary, corpus, citations, env, path):
    """Read every citation back, record each exit code, and fail if any did not resolve."""
    checks = []
    for citation in citations:
        result = subprocess.run([binary, 'read', corpus, citation, '--json'], env=env,
                                capture_output=True, text=True)
        checks.append({'citation': citation, 'exit': result.returncode,
                       'error': result.stderr if result.returncode else ''})
    path.write_text(json.dumps(checks, indent=2) + '\n')
    failed = [c['citation'] for c in checks if c['exit']]
    if failed:
        raise RuntimeError(f'{corpus}: {len(failed)} of {len(checks)} citations did not resolve: {", ".join(failed)}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_dir', type=Path)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=5)
    parser.add_argument('--prefix', default='frozen', help='new output filename prefix')
    parser.add_argument('--corpus', action='append', help='measure only this corpus (repeatable)')
    parser.add_argument('--cold', action='store_true',
                        help='add a cli-cold mode: evict the database and binary from the page cache before every call')
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
    if args.corpus:
        unknown = sorted(set(args.corpus) - set(labels))
        if unknown:
            parser.error(f'unknown corpus: {", ".join(unknown)}')
        labels = {c: q for c, q in labels.items() if c in args.corpus}
    # Check every corpus's match-count strata before creating outputs or servers.
    for corpus, queries in labels.items():
        check_strata(run / 'data' / f'{corpus}-1' / 'codecite' / f'{corpus}.db', queries)
    output = run / f'{args.prefix}-search.jsonl'
    output.touch(exist_ok=False)
    modes = ('cli', 'mcp', 'cli-cold') if args.cold else ('cli', 'mcp')
    metadata = {
        'query_sha256': hashlib.sha256(LABELS.read_bytes()).hexdigest(),
        'corpora': list(labels),
        'rounds': args.rounds, 'seed': 255, 'limit': 10,
        'cache': 'cli and mcp: one untimed warmup sweep per mode, not recorded; warm OS page cache; no cache eviction',
        'cli': 'one new process per query, startup included',
        'mcp': 'one initialized persistent process per corpus; store reopened per call',
    }
    if args.cold:
        metadata['cli-cold'] = ('as cli, but before every call posix_fadvise(DONTNEED) evicts the corpus database, '
                                'its -wal and -shm, and the binary from the page cache; shared libraries, '
                                'directory entries and inodes stay cached; runs after the MCP server stops, so nothing maps the binary; '
                                'round 0 is excluded from summaries like the other modes')
    else:
        metadata['cli-cold'] = 'not measured'
    (run / f'{args.prefix}-metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    all_records = []
    for corpus, queries in labels.items():
        data_home = run / 'data' / f'{corpus}-1'
        env = dict(os.environ, XDG_DATA_HOME=str(data_home), XDG_CACHE_HOME=str(run / 'cache'))
        process = subprocess.Popen([binary, 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, env=env, bufsize=0)
        reader = BufferedLineReader(process.stdout)
        request_id = 0

        def request(method, params):
            nonlocal request_id
            request_id += 1
            data = {'jsonrpc': '2.0', 'id': request_id, 'method': method, 'params': params}
            write_all(process.stdin, (json.dumps(data) + '\n').encode())
            deadline = time.monotonic() + 120
            while True:
                try:
                    line = reader.readline(deadline)
                except TimeoutError:
                    raise TimeoutError(f'{corpus}: {method}') from None
                except EOFError:
                    raise RuntimeError(f'{corpus}: server exited') from None
                obj = json.loads(line)
                if obj.get('id') == request_id:
                    if 'error' in obj:
                        raise RuntimeError(str(obj['error']))
                    return obj['result']

        try:
            request('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                                   'clientInfo': {'name': 'fts-tuning', 'version': '1'}})
            write_all(process.stdin, b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            rng = random.Random(255)
            references = {}
            db = data_home / 'codecite' / f'{corpus}.db'
            cold_files = [binary, db, db.with_name(db.name + '-wal'), db.with_name(db.name + '-shm')]
            for mode in modes:
                if mode == 'cli-cold':
                    # A running server maps the binary, which keeps its pages resident.
                    stop(process)
                for iteration in range(args.rounds + 1):
                    order = list(queries)
                    rng.shuffle(order)
                    for q in order:
                        if mode == 'cli-cold':
                            evict(cold_files)
                        t = time.perf_counter_ns()
                        if mode in ('cli', 'cli-cold'):
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
            read_back_citations(binary, corpus, sorted({c for citations in references.values() for c in citations}),
                                env, run / f'{args.prefix}-{corpus}-citations.json')
        finally:
            stop(process)
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
