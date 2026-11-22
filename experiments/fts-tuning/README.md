# FTS tuning baseline

These artifacts measure the untuned build for #255; they do not establish that any FTS alternative is safe. No production schema or ranking change is included.

## Reproduction

Use a quiet host, Linux, Python with `tomllib`, GNU time and a SQLite CLI with `dbstat`. Build `cargo build --release --locked -p codecite -p codecite-bench`. Create a private empty temporary run directory on a filesystem with ample space. Populate its `cache/codecite/{repos,documents}` with **copies** of the pinned caches, not links to shared caches: indexing git sources checks out and cleans their cache. Then run:

```sh
python3 experiments/fts-tuning/index_baseline.py "$run" --bin-dir target/release --checkout-root "$HOME/repos"
python3 experiments/fts-tuning/search_baseline.py "$run" --binary target/release/codecite --prefix frozen
```

The index harness refuses a dirty or wrong local checkout, uses only the new run's `data` and `cache`, measures each fresh/no-op pair serially, and checks the run metadata and no-op counts. It builds FHIR first, then the three public checkouts, identity and the unchanged retrieval/history corpora. Supply `$run` as a new absolute directory; existing `manifests`, `data`, or raw index output cause it to refuse to start. The local manifest pins and the empty include/exclude lists are in `corpora.toml`, while the generated manifests and their hashes are in the private run directory. The existing committed FHIR, identity and benchmark manifests supply the other pins. An isolated copy of caches avoids modifying an existing checkout or index.

Search uses `queries.json` and checks the binary hash against the index run. Rare and broad queries are one lowercase alphanumeric token, so the literal FTS5 match count is the search query's count; the harness verifies the predeclared strata, rare 1–200 and broad at least 10,000 matching chunks. One untimed sweep precedes five timed shuffled sweeps per mode. CLI times include startup; a single persistent MCP process is initialized once per corpus, but opens a new store per request. Neither mode is a cold-disk test. Returned citations are read back outside the timed sweeps; ranked citations must remain identical across rounds and modes. The raw records retain ranks and citations, not source snippets.

`exploratory-queries.json` and `results/exploratory-*` retain an earlier search trial, **not a correctness gate**. Its compound identifiers labelled rare were split into OR terms, and ambiguous symbols had just one accepted location. The revised workload keeps those ambiguous questions and broad terms for latency only; their acceptable answer sets require independent adjudication before quality scoring. It retains source-verified single-location example labels for the remaining questions, not exhaustive relevance judgements. One labelled example per several classes and five timed samples are too few to establish robust corpus-wide quality or tail latency. Existing retrieval/history labels were not changed. The baseline measures them with `codecite-bench --queries <existing queries.toml> --codecite <release binary> --json` against their separate index run directories; anonymized aggregate summaries are in `results/`.

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

`#283` was open when these measurements ran. If it merges, rebuild and rerun on the new `origin/main` before comparisons; its C changes can change the corpus and ranking. The #255 `detail=column`, per-document path and `optimize` experiments, further label adjudication, and comparison against the frozen labels and non-identity holdout remain necessary.
