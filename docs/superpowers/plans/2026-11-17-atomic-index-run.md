# Atomic index run Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `codecite index` publish a corpus in one transaction, under an exclusive lock, and record each run (#248).

**Architecture:** `index_corpus` takes `<corpus>.db.lock`, records an `index_run` row, then does every write of the run inside one `BEGIN IMMEDIATE` transaction. The store's per-call transactions become savepoints, so they nest inside the run's. On success the run stamps `source.index_run_id`, writes `meta.manifest_hash` and commits; on any error it rolls back and marks the run `failed`. WAL readers keep the old snapshot until the commit.

**Tech Stack:** Rust, rusqlite (bundled SQLite, WAL), `std::fs::File::try_lock`.

**Spec:** issue #248; `docs/design.md` (Data model, Indexing and retrieval).

## Global Constraints

- Search output is unchanged: a storage-only change.
- Next free migration on `main` is `0011` (#261 took `0010`); #242 and #260 also add migrations, and whichever lands later renumbers.
- Keep `crates/codecite-index/src/index.rs` edits small: #242 edits it too.
- No edits under `evals/` or `.github/`.

## Review Focus

- A run that fails after writing some documents: the corpus keeps its old revision and rows (Task 2 test).
- Two runs on one corpus: the second fails at once with an error naming the lock file (Task 1 test).
- A run killed mid-way: SQLite discards the uncommitted transaction; its `index_run` row stays `running`, with no `finished_at` (documented).
- A failure in the commit itself: the run rolls back and is marked `failed` (same code path as any error).
- A query during a run sees the old snapshot (WAL semantics; documented).

---

### Task 1: Lock, run record and migration in codecite-core

**Files:**
- Create: `crates/codecite-core/migrations/0011_index_run.sql`, `crates/codecite-core/src/store/run.rs`
- Modify: `crates/codecite-core/src/store.rs` (MIGRATIONS, `Store.lock`), `crates/codecite-core/src/store/write.rs` and `store/context.rs` (`transaction()` → `savepoint()`), `crates/codecite-core/src/error.rs` (`Error::CorpusLocked { path }`), `crates/codecite-core/src/terms.rs` (`TERMS_VERSION`)

**Interfaces (produces):**
- `Store::open_for_index(path: &Path) -> Result<Store>`: takes the lock, then opens read-write.
- `Store::lock_for_index(&mut self) -> Result<()>`: idempotent; `try_lock` on `<path>.lock`, `Error::CorpusLocked` when held.
- `Store::start_run(&mut self, run: &IndexRun<'_>) -> Result<i64>` with `IndexRun { codecite_version: &str, manifest_sha256: &str, chunker_version: u32 }`; inserts a `running` row.
- `Store::begin_publish(&mut self) -> Result<()>`: `BEGIN IMMEDIATE`.
- `Store::publish_run(&mut self, run_id: i64, counts: &RunCounts) -> Result<()>` with `RunCounts { written, unchanged, removed, chunks: usize }`; stamps sources, writes `meta.manifest_hash`, marks `succeeded`, commits.
- `Store::fail_run(&mut self, run_id: i64, error: &str) -> Result<()>`: rolls back an open transaction, marks `failed`.

- [ ] Test `second_lock_fails_clearly`: `open_for_index` twice on one path, the second is `Error::CorpusLocked`; dropping the first frees it.
- [ ] Test `failed_run_rolls_back`: start, begin, upsert a source, `fail_run` → no source rows, run row `failed` with the error.
- [ ] Test `published_run_is_recorded`: run row `succeeded` with counts and versions, `source.index_run_id` set, `meta.manifest_hash` set.
- [ ] Implement; run `cargo test -p codecite-core`; commit.

### Task 2: Wrap `index_corpus` in a published run

**Files:**
- Modify: `crates/codecite-index/src/index.rs`, `crates/codecite-index/src/manifest.rs` (`Manifest::sha256`, the sha256 of the manifest text), `crates/codecite-index/src/chunk.rs` (`CHUNKER_VERSION`), `crates/codecite/src/main.rs` (`open_for_index`)

- [ ] Test `failed_run_leaves_the_corpus_unchanged`: index a local source; change and commit two files; index a manifest with that source and a second source of the same name, so the run fails with `DuplicateSource` after writing the first source's documents; assert revision, `contents()`, source rows and `meta` are as before, and the newest `index_run` is `failed`.
- [ ] Test `a_run_is_recorded`: after a run, `meta.manifest_hash` equals the manifest's sha256 and one `succeeded` run row stamps every source.
- [ ] Test `a_second_run_fails_while_one_holds_the_lock`.
- [ ] Rename the body to `fn index_sources`, call `begin_publish` before the first write; new `pub fn index_corpus` locks, starts, and publishes or fails the run.
- [ ] `cargo test --workspace --locked`; commit.

### Task 3: Docs and ranking check

- [ ] `docs/design.md`: `index_run`, `source.index_run_id`, `meta.manifest_hash`, the lock and the one-transaction publish.
- [ ] Index `identity-bench` with the `main` binary and with this branch's into separate data dirs; run `codecite-bench --json` and every query's `codecite search --json` against each; diff. Report on the PR.
- [ ] Follow-up, not in this PR: build a full rebuild into `<corpus>.db.tmp` and `rename()` it over the old file.
