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
- `compare.py <base_run> <branch_run> --prefix <p>` puts two runs of
  `../fts-tuning/search_baseline.py` side by side, with the load averages, and
  fails if any query's ranked citations differ.
- `warm_ab.py <base_run> <base_bin> <branch_run> <branch_bin> <corpus>` times
  a few warm CLI searches, alternating base and branch on every call.

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

## Branch: document text in its own table

`ae29674` moves `document.text` into `document_text` (migration 0016), so a
`document` row is a few short columns and phase 1 never touches a text.

`results/profile-branch.json`, same queries, summed:

| B-tree, page type | Linux base | Linux branch | FHIR base | FHIR branch |
| --- | ---: | ---: | ---: | ---: |
| `document` overflow | 826,182 | 0 | 64,129 | 0 |
| `chunk` leaf | 127,619 | 127,616 | 11,240 | 11,240 |
| `document` leaf | 124,548 | 11,726 | 15,564 | 1,497 |
| `chunk_fts_docsize` leaf | 32,406 | 32,406 | 1,667 | 1,667 |
| `chunk_fts_data` leaf | 6,965 | 6,769 | 1,157 | 1,150 |
| `document_text` overflow | 0 | 613 | 0 | 278 |
| Total (all page types) | 1,120,181 | 180,698 | 94,166 | 16,158 |

What is left is one `chunk` leaf and one `chunk_fts_docsize` leaf per
candidate chunk.

`results/summary.md` (`--cold`, base then branch, 44 queries, ranked
citations identical): exact-symbol `cli-cold` p95 falls from 11,232 to 5,370 ms
on Linux and from 3,318 to 1,254 ms on FHIR; cold p50 falls 30–65% on every
class that scores candidates. Database sizes move by under 0.2%, fresh index
time by at most +6% (FHIR).

The host was shared (load average 2–14, from a VM and other sessions), so
batch timings drift. The warm `cli` rows of the cold run, and two interleaved
warm reruns (`results/warm-batch-1.md`, `results/warm-batch-2.md`), flagged
FHIR rises over 10%. Alternating every call (`results/warm-ab-fhir.md`, 80
rounds) puts the branch ahead on all three: path-filter p50 39.5 → 31.3 ms,
rare-term 5.6 → 4.3 ms, miss 3.3 → 3.1 ms. On Linux, Kubernetes and Keycloak
the warm batches already show the branch faster on most classes.
