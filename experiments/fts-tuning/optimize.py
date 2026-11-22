#!/usr/bin/env python3
"""Copy baseline databases, then time FTS optimize on the copies only."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def reject_uncheckpointed_wal(db):
    wal = Path(str(db) + '-wal')
    if wal.exists() and wal.stat().st_size:
        raise ValueError('source has uncheckpointed WAL')


def check_sqlite_version(metadata):
    inspection = metadata.get('inspection_sqlite')
    bundled = metadata.get('bundled_sqlite')
    if not inspection or not bundled:
        raise ValueError('baseline lacks recorded SQLite versions; regenerate it with '
                         'index_baseline.py --sqlite-version')
    actual = subprocess.check_output(['sqlite3', '-version'], text=True).strip()
    if actual != inspection:
        raise ValueError('SQLite CLI differs from the index baseline')
    if actual.partition(' ')[0] != bundled:
        raise ValueError('SQLite CLI differs from the bundled SQLite')


def stats(db):
    sql = """SELECT (SELECT count(*) FROM chunk) chunks,
      (SELECT count(distinct segid) FROM chunk_fts_idx) segments,
      (SELECT sum(pgsize) FROM dbstat WHERE name='chunk_fts_data') fts_data_bytes,
      (SELECT sum(pgsize) FROM dbstat WHERE name GLOB 'chunk_fts_*') fts_bytes;"""
    row = json.loads(subprocess.check_output(['sqlite3', '-readonly', '-json', db, sql], text=True))[0]
    row['database_bytes'] = db.stat().st_size
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline', type=Path)
    parser.add_argument('output', type=Path, help='new directory')
    parser.add_argument('corpora', nargs='+', help='corpus names in baseline/data/<name>-1')
    args = parser.parse_args()
    baseline = args.baseline.resolve(strict=True)
    root = args.output.absolute()
    metadata = json.loads((baseline / 'index-metadata.json').read_text())
    check_sqlite_version(metadata)
    sources = [(corpus, baseline / 'data' / (corpus + '-1') / 'codecite' / (corpus + '.db'))
               for corpus in args.corpora]
    for _, src in sources:
        reject_uncheckpointed_wal(src)
    root.mkdir()
    (root / 'index-metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    records = root / 'optimize.jsonl'
    for corpus, src in sources:
        before = root / 'before' / 'data' / (corpus + '-1') / 'codecite' / (corpus + '.db')
        after = root / 'after' / 'data' / (corpus + '-1') / 'codecite' / (corpus + '.db')
        for db in (before, after):
            db.parent.mkdir(parents=True)
            shutil.copyfile(src, db)
        pre = stats(before)
        if stats(after) != pre:
            raise ValueError(f'{corpus}: copies differ')
        wal = Path(str(after) + '-wal')
        t = time.perf_counter_ns()
        proc = subprocess.Popen(['sqlite3', after,
            "PRAGMA wal_autocheckpoint=0; INSERT INTO chunk_fts(chunk_fts) VALUES('optimize');"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        peak = 0
        while proc.poll() is None:
            try:
                peak = max(peak, wal.stat().st_size)
            except FileNotFoundError:
                pass
            time.sleep(.005)
        out, err = proc.communicate()
        seconds = (time.perf_counter_ns() - t) / 1e9
        if proc.returncode:
            raise RuntimeError(f'{corpus}: {err}')
        post = stats(after)
        if pre['chunks'] != post['chunks']:
            raise ValueError(f'{corpus}: chunk count changed')
        record = dict(corpus=corpus, before=pre, after=post, wall_seconds=seconds,
                      peak_wal_bytes=peak, wal_sampling_ms=5, stdout=out.strip(), stderr=err.strip())
        with records.open('a') as f:
            f.write(json.dumps(record, sort_keys=True) + '\n')
        print(corpus, json.dumps(record), flush=True)


if __name__ == '__main__':
    main()
