# Cold-cache search Implementation Plan

> Executed by the ralph loop's `implement` phase, task by task, test-first.

**Goal:** cut the latency of a search that finds the database out of the OS page cache (#308), without moving any result.

**Architecture:** move each document's text out of the `document` row into its own table, `document_text`, keyed by document ID. Ranking reads only small columns of `document`, so the per-candidate join no longer walks the text's overflow pages.

**Tech stack:** Rust, rusqlite on bundled SQLite 3.54.1, the #288 harness in `experiments/fts-tuning/`.

## Where the cold time goes

Measured before planning, on a copy of a Linux index (1,369,065 chunks, schema 15, built by the base binary of this branch) with `posix_fadvise(DONTNEED)` on the database and binary before each call, `strace -e pread64` on the release CLI, and each 4 KiB read mapped to its b-tree with `dbstat`:

| `search linux vfs_read`, cold | Page reads | Share |
| --- | ---: | ---: |
| `document` overflow pages | 169,068 | 76% |
| `document` leaf pages | 24,205 | 11% |
| `chunk` leaf pages | 23,016 | 10% |
| `chunk_fts_docsize` leaf pages | 5,568 | 2.5% |
| `chunk_fts_data` (the FTS index itself) | 855 | 0.4% |

`tcp_v4_connect` (75,106 reads) and `sock_create` (123,673) split the same way. Opening the file, the SCIP tables and the FTS postings are noise.

The cause is the row layout. `document.text` holds a whole file's text (#249 made code chunks cut their text from it), and migration 0010 appended `role` and `role_weight` after it. Phase 1 of search (`RANK*` in `store/search.rs`) reads `d.role_weight` and `d.role` for every candidate chunk. SQLite reaches a column stored after a large value only by following that value's overflow chain page by page, so each candidate's document costs its whole file's text in reads, scattered across the file.

Three layouts of the same index, same binary where it could run, timing the phase-1 query in the `sqlite3` CLI cold, two reps each (seconds):

| Layout | `vfs_read` | `sock_create` |
| --- | ---: | ---: |
| As built | 8.7, 9.9 | 6.3, 6.3 |
| `VACUUM` only | 2.7, 2.3 | 2.6, 2.0 |
| `text` moved to the last column, vacuumed | 2.6, 2.2 | 1.9, 1.5 |
| `text` in its own table, vacuumed | 0.6, 0.3 | 0.7, 0.4 |

The first three gave byte-identical CLI output. `VACUUM` only helps because it lays each overflow chain out contiguously, so readahead catches it; a fresh index interleaves them again. Moving `text` to the last column ends the overflow walk but leaves each row's local share of the text (up to about 4 KiB) on the `document` leaves, 51,618 leaf pages. In its own table, `document` shrinks to 2,251 leaf pages and the database to 2,373,746,688 bytes from 2,409,009,152.

**The change:** `document_text(document_id INTEGER PRIMARY KEY REFERENCES document(id) ON DELETE CASCADE, text TEXT NOT NULL)`, one row per document that stores text, and `document.text` dropped. Rejected: `mmap_size` and prefetch on open (they read the same scattered pages, or all 2.4 GB, on every first call), a smaller posting set (the FTS index is 0.4% of the reads), and a `VACUUM` on every index run (minutes of rewrite on large corpora for a smaller win that the next run undoes). The remaining cold cost is the `chunk` and `chunk_fts_docsize` leaves one per candidate; it is left for a later issue and recorded in `docs/design.md`. No rule names a project, path or language.

## Files

- Create `crates/codecite-core/migrations/0016_document_text.sql`.
- Modify `crates/codecite-core/src/store.rs` (migration list, vacuum flag, the open/reader helpers that read `document.text`, tests).
- Modify readers in `crates/codecite-core/src/store/`: `search.rs`, `grep.rs`, `definition.rs`, `read.rs`, `cite.rs`, `context.rs`, `requirements.rs`, `diff.rs`, `graph.rs`, `scip.rs`, `lines.rs`, and the writer `write.rs`.
- Modify `crates/codecite-index/src/index.rs` and `crates/codecite-index/src/spec/relations.rs`.
- Create `experiments/cold-cache-search/` (scripts, results, README).
- Modify `docs/design.md` (schema block, write path, the latency paragraph in Goals).
- Commit `evals/history/results/`.

## Task 1: Profile and the base run

Build the base release binary from `origin/fix/missed-definitions` (`d9d7701`) in a scratch worktree under `/var/tmp/r308/`. Write `experiments/cold-cache-search/run_index.py`: given a binary and a new run directory, it indexes Linux, Kubernetes and Keycloak from the clean local checkouts at the pins in `experiments/fts-tuning/corpora.toml` (refusing a dirty or wrong checkout) and FHIR from its committed manifest, each into `data/<corpus>-1` with a private copy of the cache, and writes `index-metadata.json` with the binary's SHA-256 as `search_baseline.py` expects, plus chunks, database bytes and fresh wall seconds per corpus. Write `experiments/cold-cache-search/profile.py`: it evicts the database and binary, runs one CLI search under `strace -f -e trace=pread64`, maps each read to its b-tree and page type with `dbstat`, and prints the table above as JSON. Run the indexer for the base binary in the background (`systemd-run --user` or `run_in_background`), then profile Linux and FHIR on the exact-symbol queries of `experiments/fts-tuning/queries.json`. Commit the scripts and `results/profile-base.json` with a README stating the method and the table.

Commit: `experiment(search): profile where cold-cache search time goes (#308)`.

## Task 2: Migration 0016

Test first in `store.rs`: a v15 file with a document that stores text and one that does not (a spec) opens read-write at version 16, `document_text` holds exactly the first document's text, `pragma_table_info('document')` has no `text`, `meta.vacuum_pending` is gone after the open, and `freelist_count` is 0. A second test: a v15 file opened read-only is refused with the re-run-index message, since the migration is not read-compatible.

The migration file:

```sql
-- Search joins every candidate chunk's document for its role and weight, which sat after the file's text in the row, so SQLite walked each text's overflow pages to reach them; with the page cache cold that was most of a search's time (#308). Each document's text moves to its own table, which only the readers that cite text join.
CREATE TABLE document_text (
  document_id INTEGER PRIMARY KEY REFERENCES document(id) ON DELETE CASCADE,
  text TEXT NOT NULL
);
INSERT INTO document_text (document_id, text) SELECT id, text FROM document WHERE text IS NOT NULL;
ALTER TABLE document DROP COLUMN text;
```

In `store.rs`, append the `Migration` with `read_compatible: false` and a comment saying every text reader joins the new table. Add `const DOCUMENT_TEXT: u32 = 16;` and set the vacuum flag for it the way `DERIVE_CHUNK_TEXT` does when `found > 0`, sharing one helper.

> Learn: `match version { REFILL_FTS => ..., DOCUMENT_TEXT => ... }` matches on `const` items. A name in a pattern that is a `const` in scope compares against its value; any other lowercase name would bind a new variable instead, so these stay `SCREAMING_CASE` consts.

The migration helpers for versions 10 and 12 (`fill_rank_inputs`, `clear_derivable_text`) run before 16 in the same transaction, while `document.text` still exists, so they keep reading it. Pin any test that sets `schema_version` to `SCHEMA_VERSION - 1` to its literal version.

Commit: `perf(store): move document text to its own table (#308)`.

## Task 3: The write path

Test first in `write.rs`: storing a document with text writes one `document_text` row; storing it again with new text replaces it; storing it with no text (a spec) leaves none; deleting the document or its source removes the row through the cascade.

In `write.rs`, the document upsert drops `text`; after it, with the document's ID, either `INSERT INTO document_text ... ON CONFLICT (document_id) DO UPDATE SET text = excluded.text` or `DELETE FROM document_text WHERE document_id = ?1` when the text is `None`. Every other `text` site in `write.rs` (role re-derivation, derived chunk text checks) joins the table.

> Learn: `Option<&str>` maps to SQL NULL through rusqlite's `ToSql`, but here `None` means "no row", so the code branches with `match text { Some(t) => ..., None => ... }` rather than writing a NULL.

Commit with Task 4 if the tree is not green alone.

## Task 4: The readers

Test first: the existing reader tests in each file already assert snippets and citations against stored text; run them against the new schema and keep them passing. Add one test in `search.rs` that a code document's search snippet and a generic document's still come from its text, and that a spec section's still comes from `chunk.text`.

Each `coalesce(c.text, codecite_lines(d.text, ...))` becomes `coalesce(c.text, codecite_lines(t.text, ...))` with `LEFT JOIN document_text t ON t.document_id = d.id`. Each `d.text IS NOT NULL` (the "stores its text" flag in `search.rs`, `grep.rs`, `store.rs`) becomes `t.document_id IS NOT NULL` on the same join. Each `SELECT text FROM document WHERE id = ?1` becomes `SELECT text FROM document_text WHERE document_id = ?1` with `.optional()` where a missing row was a NULL. `RANK`, `RANK_BOOSTED` and `RANK_LEGACY` do not change: they never read text, which is the point.

> Learn: `LEFT JOIN` keeps a document that has no text row and gives its `t.*` columns NULL, so `t.text` reads back as `Option<String>` exactly as `d.text` did; an inner `JOIN` would silently drop spec documents.

Then `crates/codecite-index` (`index.rs`, `spec/relations.rs`): the same rewrite. Run `cargo test --workspace --locked` in the background (the index crate's tests take minutes) and fix what fails.

Commit: `perf(store): read document text from its own table (#308)`.

## Task 5: Real CLI and an adversarial path

Release build. In a scratch corpus under `/var/tmp/r308/t5`: index a small local git repo with a code file and a Markdown file, then `search`, `grep`, `def`, `read` and `cite` each, and compare the JSON with the base binary's output on its own index (identical apart from `indexed_at`). Adversarial: copy a v15 database the base binary wrote; `search` against it must refuse with the re-run message and leave the file at version 15; `codecite index` on it must migrate in place, vacuum, and then answer the same results as a fresh index; a document deleted from the repo must leave no `document_text` row after reindex.

No commit unless a fix is needed.

## Task 6: Measure

Run `run_index.py` for the branch binary into a second run directory (background). Then, one after the other and never alongside another benchmark, run `search_baseline.py <run> --binary <bin> --prefix cold --corpus fhir --corpus keycloak --corpus kubernetes --corpus linux --cold` for the base run and the branch run, then the base again if the host load moved more than 2 between them. Check ranked citations are identical between the two runs for every query. Acceptance:

- Exact-symbol `cli-cold` p95 improves on Linux and FHIR.
- `cli` (warm) p50 and p95 per class do not rise more than 10% on any corpus; a rise over 10% is rerun once interleaved before it counts.
- Each branch database is no more than 10% larger than its base (expected smaller).

Run `profile.py` on the branch's Linux and FHIR for `results/profile-branch.json`. Write `experiments/cold-cache-search/compare.py` to put both runs side by side in `results/summary.md` with the load averages. Release build, then `evals/history/run.sh` in the background; compare each repository's pooled `all` rows for `codecite` with `scorecard.md` on `origin/fix/missed-definitions`. Ranking is untouched, so any change in Recall@1, Recall@10 or MRR is a bug to find, not a result. A loss parks the issue.

Commits: `experiment(search): measure cold-cache search after moving document text (#308)` and `chore(evals): rerun the history benchmark for #308`.

## Task 7: Docs and gates

`docs/design.md`: the schema block gets `document_text` and loses `document.text` (its comment moves); the write path notes migration 0016 and its vacuum; the Goals latency paragraph replaces the cold figures with the before and after numbers from Task 6 and names what is left (the `chunk` and `chunk_fts_docsize` leaves per candidate). Check `docs/usage.md` for any mention of where text lives. Then the gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.

Commit: `docs(design): record the cold-cache numbers and document_text (#308)`.
