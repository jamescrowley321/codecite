# Cold-cache search (#308)

Where a search spends its time when the database is not in the page cache,
and what moving each document's text to its own table does to it.

## Scripts

- `run_index.py <binary> <run_dir>` indexes Linux, Kubernetes and Keycloak
  from clean local checkouts at the pins in `../fts-tuning/corpora.toml` and
  FHIR from its committed manifest, each into `data/<corpus>-1` with a private
  copy of the cache. It writes `index-metadata.json` (the binary's SHA-256, as
  `../fts-tuning/search_baseline.py` expects) and one `index.jsonl` row per
  corpus: chunks, database bytes, fresh wall seconds.
- `profile.py <run_dir> --binary <bin> --corpus <c>...` takes each
  exact-symbol query of `../fts-tuning/queries.json`, evicts the database and
  binary from the page cache (`posix_fadvise(DONTNEED)`), runs one
  `codecite search` under `strace -f -y -e trace=pread64`, and maps every
  4 KiB page it read to its b-tree and page type with `dbstat`. It prints JSON.

Run the indexer as a `systemd-run --user` unit with `--setenv=PATH="$PATH"`
(the unit's own PATH has no `sqlite3`).

## Base: where the cold reads go

`results/profile-base.json`: the base binary (`d9d7701`, schema 15) on its own
fresh index, all exact-symbol queries summed.

| B-tree, page type | Linux reads | Share | FHIR reads | Share |
| --- | ---: | ---: | ---: | ---: |
| `document` overflow | 826,182 | 73.8% | 64,129 | 68.1% |
| `chunk` leaf | 127,619 | 11.4% | 11,240 | 11.9% |
| `document` leaf | 124,548 | 11.1% | 15,564 | 16.5% |
| `chunk_fts_docsize` leaf | 32,406 | 2.9% | 1,667 | 1.8% |
| `chunk_fts_data` leaf | 6,965 | 0.6% | 1,157 | 1.2% |
| Total | 1,120,181 | | 94,166 | |

Phase 1 of search reads `document.role` and `document.role_weight` for every
candidate chunk. Both columns sit after `document.text`, a whole file's text,
so SQLite follows each text's overflow chain page by page to reach them. The
FTS index itself is about 1% of the reads.
