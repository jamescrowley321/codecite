#!/usr/bin/env python3
"""Draw candidate graph cases for labelling, from definitions and source text (#295).

    python3 -I sample.py <codecite db> <checkout> <repo> [--n 12] [--seed 295] [--context]

Reads a codecite corpus file's tree-sitter definitions (the `symbol` table:
names, kinds and lines, never edges) and the checkout's text through
`git grep`. It never runs CodeGraph and reads no graph table, so cases drawn
from it cannot be tuned to either tool's edges.

Definitions are drawn with a seeded RNG from documents codecite classes as
`source` (its own role rule, not a path list here). For each, `mentions`
lists every other line in the repository that holds the bare name as a whole
word, so a person can read each one and decide whether it calls the drawn
definition. The buckets only steer labelling effort:

- `callers`: a name defined once, mentioned on 1 to 6 other lines;
- `same_name`: a name defined two or more times, mentioned on at most 12 lines;
- `callees`: a function or method 4 to 25 lines long.

A label is kept only after a person reads the source it points at; the draw
order and every rejection are recorded in `draws.json`. `--context` prints the
lines around each mention for that reading; it is never committed.
"""

import argparse
import json
import random
import sqlite3
import subprocess
from pathlib import Path

KINDS = ('function', 'method')


def definitions(db):
    con = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
    rows = con.execute(
        'SELECT s.display_name, s.kind, d.path, s.start_line, s.end_line '
        'FROM symbol s JOIN document d ON d.id = s.document_id '
        "WHERE d.role = 'source' AND s.kind IN (?, ?) ORDER BY d.path, s.start_line, s.id",
        KINDS).fetchall()
    counts = dict(con.execute('SELECT display_name, count(*) FROM symbol GROUP BY display_name'))
    con.close()
    return rows, counts


def mentions(checkout, name, path, line):
    out = subprocess.run(['git', '-C', checkout, 'grep', '-n', '-w', '-F', '-I', '-e', name],
                         capture_output=True, text=True, check=False).stdout
    hits = []
    for row in out.splitlines():
        file, number, _ = row.split(':', 2)
        if (file, int(number)) != (path, line):
            hits.append(f'{file}:{number}')
    return hits


def draw(rows, counts, checkout, n, seed):
    rng = random.Random(seed)
    order = list(range(len(rows)))
    rng.shuffle(order)
    buckets = {'callers': [], 'same_name': [], 'callees': []}
    for index in order:
        if all(len(v) >= n for v in buckets.values()):
            break
        name, kind, path, start, end = rows[index]
        if len(name) < 4 or not name.replace('_', '').isalnum():
            continue
        span = end - start + 1
        defs = counts.get(name, 0)
        hits = None
        entry = {'draw': index, 'name': name, 'kind': kind, 'path': path,
                 'start_line': start, 'end_line': end, 'defs': defs}
        if defs == 1 and len(buckets['callers']) < n:
            hits = mentions(checkout, name, path, start)
            if 1 <= len(hits) <= 6:
                buckets['callers'].append({**entry, 'mentions': hits})
                continue
        if defs >= 2 and len(buckets['same_name']) < n:
            hits = hits if hits is not None else mentions(checkout, name, path, start)
            if len(hits) <= 12:
                buckets['same_name'].append({**entry, 'mentions': hits})
                continue
        if 4 <= span <= 25 and len(buckets['callees']) < n:
            buckets['callees'].append(entry)
    return buckets


def show(checkout, entry):
    root = Path(checkout)
    lines = (root / entry['path']).read_text(errors='replace').splitlines()
    print(f"\n### {entry['name']} {entry['kind']} {entry['path']}:{entry['start_line']}-{entry['end_line']}")
    for i in range(entry['start_line'] - 1, min(len(lines), entry['end_line'])):
        print(f'  {i + 1}: {lines[i][:150]}')
    for mention in entry.get('mentions', []):
        file, number = mention.rsplit(':', 1)
        text = (root / file).read_text(errors='replace').splitlines()
        print(f'  > {mention}: {text[int(number) - 1].strip()[:150]}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('db')
    parser.add_argument('checkout')
    parser.add_argument('repo')
    parser.add_argument('--n', type=int, default=12)
    parser.add_argument('--seed', type=int, default=295)
    parser.add_argument('--context', action='store_true')
    args = parser.parse_args()
    rows, counts = definitions(args.db)
    buckets = draw(rows, counts, args.checkout, args.n, args.seed)
    if args.context:
        for name, entries in buckets.items():
            print(f'\n## {args.repo} {name}')
            for entry in entries:
                show(args.checkout, entry)
        return
    print(json.dumps({'repo': args.repo, 'seed': args.seed, 'n': args.n, 'buckets': buckets}, indent=1))


if __name__ == '__main__':
    main()
