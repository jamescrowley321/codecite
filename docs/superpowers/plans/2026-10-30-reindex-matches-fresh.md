# Reindex Matches Fresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Indexing a git source at one commit and then another leaves the store, and every search, exactly as a fresh index of the second commit would (#186).

**Architecture:** Two causes, both found while planning:

1. **FTS5 totals.** The rows, chunks, terms and full-text rows of the two stores are identical; only the FTS5 averages record (`chunk_fts_data` row 1: the row count and each column's total size, which BM25 reads) differs. Reproduced on `history-blind-peer-review`: indexing `f7704c7` (1,650 chunks) then `39e9e62` (455 written) leaves a row count of 2,105 where a fresh index has 1,389, so every BM25 score is off. The bundled SQLite 3.53.2 (`rusqlite` 0.40, `libsqlite3-sys` 0.38.2) doesn't take a deleted row out of a `contentless_delete=1` table's totals; SQLite 3.54.1 (`rusqlite` 0.41, `libsqlite3-sys` 0.39.1) does. With 0.41, the same reindex scores 0.33 / 0.89 / 0.46, identical to the fresh index, query by query. So: upgrade `rusqlite`, and add migration 0008, which empties both full-text tables and refills them from `chunk`, so a corpus already reindexed under the old SQLite gets correct totals on its next `index`.
2. **Ties broken by chunk ID.** `search` orders by `score DESC, c.id`. A chunk ID records when the chunk was written, and a reindex keeps the IDs of unchanged files, so two equally scored chunks can swap places between a reindexed store and a fresh one. Break ties on what the result cites instead: `s.name, d.path, c.start_line, c.end_line`. A fresh index walks a source's files sorted by path and writes each file's chunks in line order, so for one source this is the order IDs gave.

**Tech Stack:** Rust (rusqlite, SQLite FTS5), SQL migrations, bash (`evals/history/run.sh`).

## Global Constraints

- Done when (issue #186): a test indexes a git source at one commit, then another, and asserts the store (chunks, terms, FTS rows) and `search` results equal a fresh index of the second commit.
- Search ranking may change only on ties. The history benchmark judges it: any repository's pooled `all` row for `codecite` lower on Recall@1, Recall@10 or MRR than `scorecard.md` on `origin/feat/generic-documents` is a loss.
- Chunk IDs are not compared: a reindex keeps an unchanged file's IDs on purpose (`reindexing_a_git_source_at_a_new_commit_rewrites_only_what_changed`).
- Before every push: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.

## Decisions taken here (DEC-008 in the loop's journal)

- **Upgrade, don't work around.** Rebuilding the full-text tables on every run would also fix the totals, but costs a full rewrite per index and hides the bug. The fix is SQLite's; the migration repairs what the old one left.
- **Tie-break on the citation.** `s.name, d.path, c.start_line, c.end_line` is what the result cites, so equal scores always come back in the same order whatever order the corpus was built in. Across two sources it orders by name, not by the order the manifest first listed them.

## Review Focus

1. **The repair migration on a corpus with spec chunks.** `spec_fts` holds only chunks with a `number` (`insert_chunks`), not every document chunk as migration 5 filled it. Task 3 refills it with the same rule and pins it.
2. **`delete-all` on a `contentless_delete` table** resets the totals as well as the rows. Task 3 checks the averages record, not only row counts.
3. **The `rusqlite` 0.41 upgrade** may change APIs or `cargo deny` results. Task 2 runs every gate.

---

### Task 1: A failing test — reindexing matches a fresh index

**Files:**
- Modify: `crates/codecite-index/src/index.rs` (tests module)

- [ ] **Step 1: Write the test.** Next to `reindexing_a_git_source_at_a_new_commit_rewrites_only_what_changed`, add `reindexing_at_a_new_commit_matches_a_fresh_index`. Build a repository with enough files that the second commit deletes some chunks, rewrites some and keeps some: the `fixture()` files plus a second Rust file with several functions and a Markdown file with a few sections. Push commit A; then change `src/lib.rs`, delete `notes.txt` and one function from the second Rust file, add a new file, and push commit B. Index A then B into one `Corpus`, and B alone into another.

  Compare, with a helper `Corpus::contents(&self) -> Contents` that reads each as sorted vectors without IDs:
  - documents: `(path, language, hex(content_hash), size_bytes, dirty, text)`
  - chunks: `(path, start_line, end_line, kind, symbol_id, number, anchor, title, strength, text, terms)`
  - symbols: `(id, display_name, kind, path, start_line, end_line)`
  - full-text rows: `SELECT count(*) FROM chunk_fts_docsize` and `spec_fts_docsize`
  - full-text totals: `SELECT block FROM chunk_fts_data WHERE id = 1` and the same for `spec_fts` (the averages record BM25 reads)

  Then search both stores for a few queries that hit kept, rewritten and new chunks (`"validate token"`, `"refresh session"`, a word from the Markdown file) through `Store::search` with `limit: 20` and compare `(citation, score)` lists exactly.

  - Learn: `#[derive(PartialEq, Debug)]` on a test-only struct lets one `assert_eq!` compare two whole stores and print the difference.

- [ ] **Step 2: Watch it fail.** `cargo test -p codecite-index --lib reindexing_at_a_new_commit_matches` (filter by name; the full lib suite takes ~2.5 min). Expect the totals record and the scores to differ, with every row equal.

- [ ] **Step 3: Commit.** Don't commit a red test alone; carry it into Task 2's commit.

### Task 2: Upgrade rusqlite to 0.41 (bundled SQLite 3.54.1)

**Files:**
- Modify: `Cargo.toml` (`rusqlite = { version = "0.41", ... }`), `Cargo.lock`
- Modify: any call site the 0.41 API changed

- [ ] **Step 1:** Change the workspace dependency and run `cargo update -p rusqlite`. Check `Cargo.lock` moved only `rusqlite`, `libsqlite3-sys` and what they pull in.
  - Learn: a workspace's `[workspace.dependencies]` pins one version for every crate that says `rusqlite.workspace = true`; `cargo update -p` moves one package in `Cargo.lock` without touching the rest, and `--locked` in CI refuses a lockfile that doesn't match `Cargo.toml`.
- [ ] **Step 2:** `cargo build --workspace --all-targets --locked`; fix any API change at its call site.
- [ ] **Step 3:** Task 1's test passes. Run every gate, including `cargo deny check` (licenses and advisories for the new crates).
- [ ] **Step 4: Commit** `fix(core): upgrade rusqlite so a reindex keeps FTS5 totals right (#186)` with Task 1's test, the body naming the SQLite versions and the symptom.

### Task 3: Migration 0008 repairs full-text totals in existing corpora

**Files:**
- Create: `crates/codecite-core/migrations/0008_refill_fts.sql`
- Modify: `crates/codecite-core/src/store.rs` (register the migration; generalize `fill_spec_fts`)

- [ ] **Step 1: Write the failing test** in `store.rs`'s tests, after the version 6 test: build a version 7 corpus (`MIGRATIONS[..7]`) with a code chunk and a numbered spec chunk, filled into `chunk_fts` and `spec_fts`. Record the averages records, then overwrite `chunk_fts_data` row 1 with a wrong one (copy the record of a store with an extra row, or bump its first varint), as an old SQLite left it. Open with `Store::open_read_write`; assert both averages records equal those of the same rows freshly filled, `chunk_fts_docsize` and `spec_fts_docsize` counts equal the chunk and numbered-chunk counts, and a `MATCH` finds each chunk.
- [ ] **Step 2: Watch it fail** (no migration 8 yet: the wrong record stays).
- [ ] **Step 3: Implement.** `0008_refill_fts.sql`:

  ```sql
  -- Before SQLite 3.54, deleting a row from a contentless_delete FTS5 table left it in the table's totals, so a reindexed corpus ranked by wrong BM25 statistics. Empty both tables; open_read_write refills them from chunk.
  INSERT INTO chunk_fts (chunk_fts) VALUES ('delete-all');
  INSERT INTO spec_fts (spec_fts) VALUES ('delete-all');
  ```

  In `store.rs`, turn `fill_spec_fts` into `fill_fts(tx, table, filter)` used by migration 5 (`spec_fts`, `s.kind = 'document'`, as it was) and migration 8 (`chunk_fts` with every chunk; `spec_fts` with `c.number IS NOT NULL`, the rule `insert_chunks` writes by). The table names are constants in code, never input, so `format!` into the SQL is safe; say so in a comment.
  - Learn: FTS5 special commands are written as an `INSERT` into the table's own hidden column (`INSERT INTO t (t) VALUES ('delete-all')`); `delete-all` is only allowed on contentless and external-content tables.
- [ ] **Step 4:** The test passes; `every_migration_file_is_registered` and the existing migration tests pass.
- [ ] **Step 5: Commit** `fix(core): refill the full-text tables so old corpora rank right (#186)`.

### Task 4: Break search ties on the citation, not the chunk ID

**Files:**
- Modify: `crates/codecite-core/src/store/search.rs` (`ranked`'s `ORDER BY`, and `code_as_before` in tests, which must stay the same query)
- Modify: `crates/codecite-index/src/index.rs` (tests)

- [ ] **Step 1: Write the failing test** in `index.rs`: index a local source with only `b.rs` holding `pub fn tied() {}`; then add `a.rs` with the same text and index again, so `a.rs`'s chunk has the higher ID. Search `tied`: both score the same; assert `a.rs` comes first, as in a fresh index. Watch it fail.
- [ ] **Step 2: Implement** `ORDER BY score DESC, s.name, d.path, c.start_line, c.end_line` in `ranked` and in `code_as_before`. Comment why: a chunk ID records when it was written, and a reindex keeps unchanged files' IDs.
- [ ] **Step 3:** The test passes, and every existing search test (they pin ranked citations) still passes.
- [ ] **Step 4: Commit** `fix(search): break ties by what a result cites, not when it was indexed (#186)`.

### Task 5: Docs

**Files:**
- Modify: `docs/design.md`, the `index_corpus` doc comment in `crates/codecite-index/src/index.rs`

- [ ] **Step 1:** In `docs/design.md`'s store and incremental-index text, say a reindex leaves the store as a fresh index of the same pins would (same rows, same full-text totals), that ties break by citation, and that migration 0008 refilled the full-text tables once. Name the SQLite floor (3.54, through `rusqlite` 0.41's bundled build) and why.
- [ ] **Step 2:** Add the same promise to `index_corpus`'s doc comment.
- [ ] **Step 3: Commit** `docs: a reindex matches a fresh index (#186)`.

### Task 6: History benchmark

- [ ] **Step 1:** `cargo build --release --locked -p codecite -p codecite-bench`, then `RG=/home/james/.pi/agent/bin/rg evals/history/run.sh` in the background (log under `/var/tmp/186-bench/`).
- [ ] **Step 2:** Compare each repository's pooled `all` row for `codecite` with `git show origin/feat/generic-documents:evals/history/results/scorecard.md`. Lower on Recall@1, Recall@10 or MRR anywhere is a loss: park the issue with both rows.
- [ ] **Step 3: Commit** `chore(evals): rerun the history benchmark on the reindex fix (#186)` with `evals/history/results/`.
- [ ] **Step 4:** Run every gate.
