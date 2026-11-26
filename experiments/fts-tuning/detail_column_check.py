#!/usr/bin/env python3
"""Show the two ways FTS5 detail=column breaks codecite's contentless chunk_fts.

1. A quoted underscore identifier is a multi-token phrase, which detail=column rejects.
2. On a contentless table (content=''), bm25() scores every detail=column match 0.0,
   so ranking collapses to ties.
"""
import subprocess


def run(sql):
    return subprocess.run(['sqlite3', ':memory:'], input=sql, text=True, capture_output=True)


def phrase(detail):
    return run(f'''CREATE VIRTUAL TABLE t USING fts5(value, tokenize='unicode61', detail={detail});
INSERT INTO t VALUES ('tcp_v4_connect');
SELECT count(*) FROM t WHERE t MATCH '"tcp_v4_connect"';
''')


def bm25(detail):
    result = run(f'''CREATE VIRTUAL TABLE t USING fts5(value, content='', contentless_delete=1, tokenize='unicode61', detail={detail});
INSERT INTO t(rowid, value) VALUES (1, 'nonce nonce x'), (2, 'nonce');
WITH RECURSIVE n(i) AS (SELECT 3 UNION ALL SELECT i + 1 FROM n WHERE i < 52)
INSERT INTO t(rowid, value) SELECT i, 'filler ' || i FROM n;
SELECT bm25(t) FROM t WHERE t MATCH 'nonce' ORDER BY rowid;
''')
    if result.returncode != 0:
        raise RuntimeError(f'bm25 query failed with detail={detail}: {result.stderr}')
    return [float(v) for v in result.stdout.split()]


def main():
    full = phrase('full')
    column = phrase('column')
    if full.returncode != 0 or full.stdout.strip() != '1':
        raise RuntimeError(f'unexpected detail=full result: {full.stdout} {full.stderr}')
    if column.returncode == 0 or 'phrase queries are not supported' not in column.stderr:
        raise RuntimeError(f'unexpected detail=column result: {column.stdout} {column.stderr}')
    print('phrase: detail=full: 1 match; detail=column: phrase queries are not supported')
    full_scores, column_scores = bm25('full'), bm25('column')
    if len(full_scores) != 2 or any(s >= 0 for s in full_scores) or full_scores[0] == full_scores[1]:
        raise RuntimeError(f'unexpected detail=full bm25: {full_scores}')
    if column_scores != [0.0, 0.0]:
        raise RuntimeError(f'unexpected detail=column bm25: {column_scores}')
    print(f'contentless bm25: detail=full: {full_scores}; detail=column: {column_scores}')


if __name__ == '__main__':
    main()
