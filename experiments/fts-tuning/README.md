# FTS tuning baseline

These artifacts measure the untuned build for #255; they do not establish that any FTS alternative is safe. No production schema or ranking change is included.

## Reproduction

Use a quiet host, Linux, Python with `tomllib`, GNU time and a SQLite CLI with `dbstat`. Build `cargo build --release --locked -p codecite -p codecite-bench`. Create a private empty temporary run directory on a filesystem with ample space. Compile the SQLite-version helper against the same locked release dependency, and populate `cache/codecite/{repos,documents}` with **copies** of the pinned caches, not links to shared caches: indexing git sources checks out and cleans their cache. Then run:

```sh
sql=$(echo target/release/deps/librusqlite-*.rlib)
rustc --edition=2024 -O experiments/fts-tuning/sqlite_version.rs -L dependency=target/release/deps --extern rusqlite="$sql" -o "$run/sqlite-version"
python3 experiments/fts-tuning/index_baseline.py "$run" --bin-dir target/release --checkout-root "$HOME/repos" --sqlite-version "$run/sqlite-version"
python3 experiments/fts-tuning/search_baseline.py "$run" --binary target/release/codecite --prefix frozen
```

The index harness checks both release binaries, the SQLite versions and every local checkout before it creates any run output, refuses a dirty or wrong local checkout, uses only the new run's `data` and `cache`, measures each fresh/no-op pair serially, and checks the run metadata and no-op counts. It records the helper's bundled SQLite version and refuses a SQLite CLI with a different version. The same private cache is reused in pair order, so pair 2 inherits pair 1's cache state; its wall time is not an independent cold-cache measurement. Data directories remain separate. It builds FHIR first, then the three public checkouts, identity and the unchanged retrieval/history corpora. Supply `$run` as a new absolute directory; existing `manifests`, `data`, or raw index output cause it to refuse to start. The local manifest pins and the empty include/exclude lists are in `corpora.toml`, while the generated manifests and their hashes are in the private run directory. The existing committed FHIR, identity and benchmark manifests supply the other pins. An isolated copy of caches avoids modifying an existing checkout or index.

Search uses `queries.json` and checks the binary hash against the index run. It sets both data and cache homes inside the run rather than inheriting the host's cache. Rare and broad queries are one lowercase alphanumeric token, so the literal FTS5 match count is the search query's count; before creating any output or starting a server, the harness verifies every corpus's predeclared strata, rare 1–200 and broad at least 10,000 matching chunks. One untimed sweep precedes five timed shuffled sweeps per mode. CLI times include startup; a single persistent MCP process is initialized once per corpus, but opens a new store per request. Neither mode is a cold-disk test. Returned citations are read back outside the timed sweeps; ranked citations must remain identical across rounds and modes. The raw records retain ranks and citations, not source snippets.

`exploratory-queries.json` and `results/exploratory-*` retain an earlier search trial, **not a correctness gate**. Its compound identifiers labelled rare were split into OR terms, and ambiguous symbols had just one accepted location. The revised workload keeps those ambiguous questions and broad terms for latency only; their acceptable answer sets require independent adjudication before quality scoring. It retains source-verified single-location example labels for the remaining questions, not exhaustive relevance judgements. One labelled example per several classes and five timed samples are too few to establish robust corpus-wide quality or tail latency. Existing retrieval/history labels were not changed. The baseline measures them with `codecite-bench --queries <existing queries.toml> --codecite <release binary> --json` against their separate index run directories; anonymized aggregate summaries are in `results/`. Harness unit tests run with `python3 -m unittest experiments/fts-tuning/test_harness.py`.

## Observations

The run used source revision `9b779886c0e3aabd084e2226d3481b235bbc0df0`, Rust 1.99.1, and bundled SQLite 3.54.1. `results/index-metadata.json` records both binary hashes, corpus manifest hashes and source pins. `results/index.jsonl` holds individual fresh/no-op runs, including skips. FHIR's first fresh run used 464,308 KiB peak RSS versus 63,648 KiB for the second; this unexplained variation and host activity preclude treating either as a stable RSS estimate.

| Corpus | Documents | Chunks | Fresh wall seconds | No-op wall seconds | Fresh database bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| FHIR | 23,291 | 231,843 | 78.91, 70.72 | 5.96, 2.16 | 806,481,920 |
| Linux | 90,971 | 1,369,055 | 225.13, 216.07 | 10.65, 8.91 | 2,352,668,672 |
| Kubernetes | 30,973 | 436,812 | 50.52, 48.37 | 2.28, 2.26 | 686,567,424 |
| Keycloak | 12,785 | 116,785 | 18.45, 17.27 | 0.84, 0.94 | 255,008,768 |
| Identity | 886 | 13,468 | 1.64, 1.52 | 0.13, 0.13 | 23,257,088 |

Linux is over one million chunks on this build. Its exact-symbol CLI class has p50 117.90 ms and p95 536.32 ms across 30 samples (six questions, five rounds); the separate behaviour class is p50 196.61 ms and p95 364.10 ms across ten samples. These are not pooled with other corpora and miss the 100 ms p95 target for these samples. The frozen identity retrieval baseline has 122 queries, Recall@1 0.148, Recall@10 0.475 and MRR 0.245. Frozen per-class measures, both search modes, per-query records and citations are in `results/`. All 361 distinct citations returned by the revised four-corpus workload resolved through `read`; all four miss controls returned empty results in every timed round. The ambiguity probes returned 49, 518 and 596 definitions for Linux, Keycloak and Kubernetes respectively; FHIR's `FhirContext` returned eight. `def` returned at most 50 results, so totals, not displayed counts, describe ambiguity.

`#283` was open when these measurements ran and merged afterwards. Its C changes can change the Linux corpus and ranking, so rebuild and rerun on the current `origin/main` before comparing any later variant against these Linux numbers. The experiments below compare the three #255 levers; further label adjudication and broader comparisons are needed before any ranking change.

## `detail=column` isolated prototype

A scratch build from the same base revision changed only the initial FTS DDL as recorded in `results/detail-column/variant.patch`; it is a fresh-index experiment, not a migration. Binary hashes and raw completed records are in `results/detail-column/`. The two FHIR fresh runs each had 231,843 chunks and 79,720,448 FTS data bytes, versus 451,588,096 bytes with `detail=full`; the first Linux run had 1,369,055 chunks and 230,682,624 FTS data bytes, versus 607,195,136 bytes. The second Linux run was interrupted and is not measured. These size observations do not establish a latency or ranking win.

**Actionable incompatibility:** the frozen Linux `tcp_v4_connect` search succeeds with the baseline binary but the `detail=column` binary exits with `fts5: phrase queries are not supported (detail!=full)`. A naive DDL-only change is therefore not deployable: some emitted quoted terms tokenize to multiple FTS tokens even though the application intends single-term queries. A partial paired probe, including the error and two single-token controls, is recorded in `probe.json`; the longer frozen comparison stopped on this error, so it is not a quality or latency result. Fixing query semantics would be a separate ranking-changing design requiring independent evaluation; do not infer that shrinking postings alone is safe.

The saved scratch prototype's remaining indexes and frozen retrieval/history comparisons were not run because its query semantics are incompatible. For a separately designed compatible variant, rerun the frozen labels and citation checks on every corpus, then compare per-corpus quality and paired latency before proposing production tuning. The following sections measure the other two levers independently.

## Path postings

`path_postings.rs` builds two new path-only contentless FTS5 tables from the same read-only corpus, using production `text_terms` and `unicode61`: one row per chunk and one per document having chunks. It does not reproduce the full multi-column index, ranking or a joined query. On the original untuned FHIR data (231,843 chunks, 23,208 documents with chunks), per-chunk `paths_data` was 7,450,624 bytes versus 983,040 per document, saving 6,467,584 bytes. On identity (13,468 chunks, 868 documents), it was 266,240 versus 32,768 bytes, saving 233,472. Build time and peak RSS for both tables together were 0.89 s / 12,496 KiB and 0.07 s / 6,864 KiB respectively; complete table and database sizes are in `results/path-postings.json`. Path-only postings save about 1.43% of FHIR's existing 451,588,096-byte full FTS data, without accounting for any additional join or table cost. This narrow storage result does not warrant building the more complicated joined ranking prototype now; it does not prove equivalence or predict query latency.

Reproduce with the locked release build, in a new private directory, with the same baseline corpus files as above. Link against the exact release dependencies, then run the helper once per corpus, providing two nonexistent database paths:

```sh
core=$(echo target/release/deps/libcodecite_core-*.rlib)
sql=$(echo target/release/deps/librusqlite-*.rlib)
rustc --edition=2024 -O experiments/fts-tuning/path_postings.rs -L dependency=target/release/deps --extern codecite_core="$core" --extern rusqlite="$sql" -o "$run/path_postings"
/usr/bin/time "$run/path_postings" "$baseline_db" "$run/chunk.db" "$run/document.db"
```

Run `python3 experiments/fts-tuning/detail_column_check.py` to reproduce the FTS `detail=column` error without the corpus: with SQLite 3.54.1, create an FTS5 table using `detail=column`, insert `tcp_v4_connect`, and match `"tcp_v4_connect"`. The `unicode61` tokenizer splits the quoted identifier at underscores into a multi-token phrase, unsupported by that detail setting. The same MATCH succeeds with `detail=full`. The production query builder preserves and quotes that identifier in addition to its split terms; changing it is a ranking change, not a DDL-only fix.

## FTS optimize

`optimize.py` verifies that the SQLite CLI still matches the baseline's recorded full version and bundled version and checks every requested source for a nonempty WAL before creating the output directory or copying any database. It then copies baseline databases into new before/after directories and invokes that CLI's FTS5 `optimize` command only on the after copies. The committed `results/index-metadata.json` predates bundled-version recording; an old baseline without `bundled_sqlite` is refused with instructions to regenerate it using `index_baseline.py --sqlite-version`. It polls WAL size every 5 ms (the observed peak is a lower bound), disables automatic checkpoints during the command, and records wall time (the SQLite CLI's startup, the optimize and the checkpoint SQLite runs when the CLI closes the database), table and database bytes, chunks and segments. It never changes the original files. These copies came from completed fresh/no-op pairs: the 100%-fresh build state is represented; no incremental rewrite fractions were synthesized, so these data cannot select a rewrite threshold. The command runs after indexing in a separate transaction, not inside the indexer's publish transaction; the latter's wall time and WAL peak could differ.

On FHIR (231,843 chunks), optimize reduced 26 segments to one, FTS data from 451,588,096 to 449,773,568 bytes, and increased the database from 806,842,368 to 826,589,184 bytes. Wall time was 2.739 s and sampled WAL peak 453,772,712 bytes. On the frozen identity retrieval corpus (12,583 chunks), 11 segments became one, FTS data fell from 5,681,152 to 5,492,736 bytes, and the database grew from 21,577,728 to 25,300,992 bytes; wall time was 0.077 s and sampled WAL peak 5,763,912 bytes. This is one measurement per corpus, not a threshold or production recommendation.

The same seven-query frozen FHIR subset ran on the two copies, one warmup plus five sweeps per CLI and MCP mode. All 84 pre/post result lists were identical, with 47 distinct citations resolving on each side. CLI p50/p95 for the broad term was 120.14/168.93 ms before and 111.53/118.89 ms after; other buckets and raw samples are in `results/optimize/`. Runs were sequential, not interleaved, and each class has at most two distinct questions, so the apparent latency improvement is preliminary. The 122 frozen identity retrieval queries and five frozen history corpora produced identical per-query outcomes and aggregate scores before and after. History results are anonymized. More repeated interleaved trials, incremental rewrite fractions and peak-WAL observation inside a publish transaction are needed before choosing whether or when to run optimize.

To reproduce, create an output path that does not yet exist and run the following with the pinned baseline data and no concurrent benchmarks; list the frozen history corpus names as additional arguments to reproduce those checks:

```sh
python3 experiments/fts-tuning/optimize.py "$baseline_run" "$new_run" fhir identity-bench
```

Then run the frozen FHIR query subset against `$new_run/before` and `$new_run/after` with the recorded binary, and the unchanged retrieval/history `codecite-bench` labels against each copy. The exact FHIR subset and its hash, per-sample records, citation checks and aggregate comparisons are under `results/optimize/`. Baseline caches and input databases are never overwritten.

## Incremental optimize comparison

A bounded **synthetic same-content** experiment rewrites 1%, 10%, and 50% of the 12,583 chunk FTS rows in the frozen identity retrieval corpus. Each arm starts from a separate byte-identical private copy of the baseline database. It selects the first chunk IDs, reads the existing chunk, symbol and document data, reconstructs the FTS terms using the production `identifier_terms` and `text_terms`, deletes and reinserts each FTS row, then commits. The optimize arm runs the FTS `optimize` command in that same transaction; the control does not. It does not simulate changed source text, document cascades, tree-sitter, or the rest of the indexer's publish transaction, and first-ID selection is not representative sampling. Segment counts are affected by SQLite's automatic merging. These are **not** production index-run measurements.

One transaction per arm per fraction was timed; raw records, the exact frozen label and binary hashes, source database hash, and per-call timings are in `results/incremental-optimize/`. WAL was polled every millisecond and its sampled maximum is a lower bound. All 122 frozen retrieval outcomes matched the original baseline and across arms for each fraction. For eight frozen questions, one warmup plus five sequential, alternating-arm CLI sweeps returned identical paired citations; 78 distinct citations per arm and fraction all resolved. There are only 40 timed calls per arm per fraction, no disk-cache eviction, and the individual timing distributions should not be generalized to other corpora or runs.

| Rewritten | Control transaction | Optimize transaction | Control / optimize segments | Control / optimize sampled WAL peak |
| --- | ---: | ---: | ---: | ---: |
| 1% (125 chunks) | 24.9 ms | 76.5 ms | 5 / 1 | 1,186,592 / 6,118,232 bytes |
| 10% (1,258 chunks) | 91.6 ms | 121.8 ms | 4 / 1 | 3,539,112 / 7,646,752 bytes |
| 50% (6,291 chunks) | 422.5 ms | 482.5 ms | 4 / 1 | 10,646,112 / 10,646,112 bytes |

**Recommendation:** no automatic optimize threshold now. The fresh and synthetic runs show an extra write and transient WAL cost but insufficient repeated, cross-corpus paired evidence of a search benefit. Likewise, reject the DDL-only `detail=column` change and do not implement the path join on storage savings alone. A future ranking/query change requires a separate plan and frozen quality comparison; an optimize proposal needs realistic changed-document workloads at measured rewrite fractions, repeated interleaved runs, and indexer's own publish-transaction cost and WAL peaks. #283 has since landed, so rebuild on the current base before comparing Linux results.

To reproduce the incremental experiment from the repository root, use the pinned release build and a private, nonexistent output path:

```sh
core=$(echo target/release/deps/libcodecite_core-*.rlib)
sql=$(echo target/release/deps/librusqlite-*.rlib)
rustc --edition=2024 -O experiments/fts-tuning/incremental_rewrite.rs -L dependency=target/release/deps --extern codecite_core="$core" --extern rusqlite="$sql" -o "$helper"
python3 experiments/fts-tuning/incremental_optimize.py "$baseline_run" "$new_run" --helper "$helper" --bin-dir target/release
```

The helper writes to the path it receives; the Python driver passes only newly created private copies, resolves the helper before creating any output, refuses a nonempty source WAL, checks the binary and manifest pins, sets both data and cache homes inside the new run, and never rewrites the original index. Run on a quiet host.
