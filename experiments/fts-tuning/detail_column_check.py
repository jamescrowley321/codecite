#!/usr/bin/env python3
"""Show why a quoted underscore identifier fails with FTS5 detail=column."""
import subprocess


def query(detail):
    sql = f'''CREATE VIRTUAL TABLE t USING fts5(value, tokenize='unicode61', detail={detail});
INSERT INTO t VALUES ('tcp_v4_connect');
SELECT count(*) FROM t WHERE t MATCH '"tcp_v4_connect"';
'''
    return subprocess.run(['sqlite3', ':memory:'], input=sql, text=True, capture_output=True)


def main():
    full = query('full')
    column = query('column')
    if full.returncode != 0 or full.stdout.strip() != '1':
        raise RuntimeError(f'unexpected detail=full result: {full.stdout} {full.stderr}')
    if column.returncode == 0 or 'phrase queries are not supported' not in column.stderr:
        raise RuntimeError(f'unexpected detail=column result: {column.stdout} {column.stderr}')
    print('detail=full: 1 match; detail=column: phrase queries are not supported')


if __name__ == '__main__':
    main()
