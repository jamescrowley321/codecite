#!/usr/bin/env python3
"""Attribute a cold-cache search's database reads to b-trees (#308).

For each exact-symbol query of experiments/fts-tuning/queries.json on the
given corpora: evict the database and the binary from the page cache, run one
`codecite search` under `strace -f -y -e trace=pread64`, and map every read of
the database to the b-tree and page type `dbstat` gives for its page. Prints
JSON: per corpus and query, the page reads, how many followed the previous page
(sequential), the wall seconds, and the reads per (b-tree, page type).
"""

import argparse
import collections
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
QUERIES = HERE.parent / 'fts-tuning' / 'queries.json'
sys.path.insert(0, str(HERE.parent / 'fts-tuning'))
from search_baseline import evict  # noqa: E402

READ = re.compile(r'pread64\(\d+<([^>]*)>, .*?, (\d+), (\d+)\) = (\d+)')


def page_map(db):
    """Page number → (b-tree name, page type), and the page size."""
    size = int(subprocess.check_output(['sqlite3', '-readonly', db, 'PRAGMA page_size;'], text=True))
    rows = subprocess.check_output(
        ['sqlite3', '-readonly', '-separator', '|', db, 'SELECT pageno, name, pagetype FROM dbstat;'],
        text=True)
    pages = {}
    for line in rows.splitlines():
        page, name, kind = line.split('|')
        pages[int(page)] = (name, kind)
    return pages, size


def profile(binary, corpus, data_home, query, pages, page_size):
    db = data_home / 'codecite' / f'{corpus}.db'
    env = dict(os.environ, XDG_DATA_HOME=str(data_home))
    with tempfile.NamedTemporaryFile('r', suffix='.strace') as trace:
        evict([binary, db, db.with_name(db.name + '-wal'), db.with_name(db.name + '-shm')])
        start = time.perf_counter()
        subprocess.run(['strace', '-f', '-y', '-qq', '-e', 'trace=pread64', '-o', trace.name,
                        binary, 'search', corpus, query, '--limit', '10', '--json'],
                       env=env, check=True, stdout=subprocess.DEVNULL)
        seconds = time.perf_counter() - start
        counts = collections.Counter()
        reads = sequential = 0
        previous = None
        for line in trace:
            m = READ.search(line)
            if not m or m.group(1) != str(db):
                continue
            size, offset = int(m.group(2)), int(m.group(3))
            first, last = offset // page_size + 1, (offset + size - 1) // page_size + 1
            for page in range(first, last + 1):
                reads += 1
                sequential += previous is not None and page == previous + 1
                previous = page
                counts[pages.get(page, ('?', '?'))] += 1
    btrees = [{'btree': name, 'pagetype': kind, 'reads': n, 'share': round(n / reads, 4)}
              for (name, kind), n in counts.most_common()]
    return {'corpus': corpus, 'query': query, 'page_reads': reads, 'sequential': sequential,
            'seconds_under_strace': round(seconds, 3), 'btrees': btrees}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_dir', type=Path, help='a run_index.py run directory')
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--corpus', action='append', required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    labels = json.loads(QUERIES.read_text())['corpora']
    results = []
    for corpus in args.corpus:
        data_home = args.run_dir.resolve(strict=True) / 'data' / f'{corpus}-1'
        pages, page_size = page_map(data_home / 'codecite' / f'{corpus}.db')
        for q in labels[corpus]:
            if q['class'] == 'exact-symbol':
                results.append(profile(binary, corpus, data_home, q['text'], pages, page_size))
                print(corpus, q['text'], results[-1]['page_reads'], file=sys.stderr, flush=True)
    json.dump({'binary': str(binary), 'results': results}, sys.stdout, indent=2)
    print()


if __name__ == '__main__':
    main()
