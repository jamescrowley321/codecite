# Corpus SQLite Schema Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #14: `codecite-core` opens or creates a corpus's SQLite file in WAL mode, brings it to the latest schema with numbered `.sql` migrations, refuses a file from a newer schema, and gives queries a read-only handle.

**Architecture:** A new `store` module in `codecite-core` owns a `Store` that wraps one `rusqlite::Connection`. `Store::open_read_write` (indexing) creates the file if it is missing and applies any missing migrations inside one `BEGIN IMMEDIATE` transaction, then switches the file to WAL. `Store::open_read_only` (queries) never creates or changes the file and accepts only a file already at the latest version. Migrations live in `crates/codecite-core/migrations/NNNN_<name>.sql` and are compiled in with `include_str!`. Migration N sets `meta.schema_version` to N.

**Tech Stack:** Rust 1.99.1 (edition 2024), `rusqlite` 0.40 with `bundled` (SQLite 3.53.2), `thiserror` 2.

**Spec:** `docs/design.md#data-model`, `docs/design.md#decisions-and-open-questions` (the two FTS5 open questions), and issue #14.

## Global Constraints

- New dependency: `rusqlite = { version = "0.40", features = ["bundled"] }`, declared once under `[workspace.dependencies]`. Only `codecite-core` takes it. No other new dependencies, including dev-dependencies. Tests make scratch directories under `std::env::temp_dir()` instead of pulling in `tempfile`.
- `cargo deny check` must stay green. A probe build found two failures, and Task 1 fixes both:
  - `foldhash` 0.2.0 (via `hashlink` → `hashbrown` 0.17) is `Zlib`.
  - `sqlite-wasm-rs` (a wasm32-only dependency of `rusqlite`) pulls `hashbrown` 0.16.1 next to 0.17.1.
- The schema is the SQL in `docs/design.md#data-model` with two changes. `chunk_fts` gains `contentless_delete=1`. `chunk_vec` is left out, because the design creates it only when embeddings are enabled and it needs the sqlite-vec extension.
- `meta.schema_version` is stored as TEXT (the `meta.value` column) and read as `u32`. A file without a `meta` table is version 0 only when it holds no schema objects at all.
- The workspace forbids `unsafe_code`. Nothing here needs it.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- Pointing `open_read_write` at a file that isn't a codecite corpus (a text file, or another program's SQLite database) must return an error and leave the file byte-identical. It must not add codecite tables to someone else's database. Tests: `read_write_refuses_non_sqlite_file_unchanged` and `read_write_refuses_foreign_database_unchanged`, Task 2.
- A query against a mistyped path must not leave an empty file behind. Test: `read_only_missing_file_is_invalid_and_not_created`, Task 2.
- A file from a newer schema is refused by both open modes without being touched. Setting WAL first would rewrite header bytes. Tests: `read_write_refuses_newer_schema_unchanged` and `read_only_refuses_newer_schema`, Task 2.
- Someone adds `0002_x.sql` but forgets to register it in `MIGRATIONS`, so the binary silently stays at version 1. Test: `every_migration_file_is_registered`, Task 2.
- Two indexers opening the same fresh file at once must both succeed, and the migrations must run once. Test: `concurrent_first_opens_both_succeed`, Task 2.

---

### Task 1: `rusqlite` dependency and store error variants

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`: add `rusqlite`)
- Modify: `crates/codecite-core/Cargo.toml` (add `rusqlite.workspace = true`)
- Modify: `deny.toml`
- Modify: `crates/codecite-core/src/error.rs`
- Modify: `Cargo.lock`

**Interfaces:**
- Produces two new variants on `codecite_core::Error`. Task 2 uses both.
  - `Sqlite(#[from] rusqlite::Error)`
  - `SchemaTooNew { path: PathBuf, found: u32, supported: u32 }`

- [ ] **Step 1: Add the dependency**

Root `Cargo.toml`, `[workspace.dependencies]`, after `anyhow`: `rusqlite = { version = "0.40", features = ["bundled"] }`. In `crates/codecite-core/Cargo.toml` `[dependencies]`: `rusqlite.workspace = true`.

Learn: `bundled` compiles the SQLite C source shipped inside `libsqlite3-sys` and links it statically. Every build gets the same SQLite version (3.53.2 for `libsqlite3-sys` 0.38) with the same compile options, FTS5 among them, whatever the machine has installed. The SQLite version therefore changes only when we bump `rusqlite` in `Cargo.lock`. That is why the design can depend on a version-gated feature like `contentless_delete=1`.

- [ ] **Step 2: Make `cargo deny check` pass**

Run: `cargo deny check`
Expected before the fix: `bans FAILED, licenses FAILED` (the two findings in Global Constraints).

In `deny.toml`:
- Add `"Zlib"` to `[licenses] allow`. Comment: a permissive OSI license, compatible with Apache-2.0. `foldhash` (via `rusqlite` → `hashlink` → `hashbrown`) uses it.
- Add a `[bans] skip` entry: `{ crate = "hashbrown@0.16.1", reason = "sqlite-wasm-rs, a wasm32-only dependency of rusqlite, still depends on hashbrown 0.16" }`.

Run: `cargo deny check`
Expected: `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 3: Write the failing tests** in `error.rs`'s `tests` module

```rust
#[test]
fn sqlite_error_converts_and_is_the_source() {
    let inner = rusqlite::Error::InvalidQuery;
    let msg = inner.to_string();
    let err = Error::from(inner);
    assert!(matches!(err, Error::Sqlite(_)));
    assert_eq!(err.to_string(), "SQLite error");
    assert_eq!(err.source().unwrap().to_string(), msg);
}

#[test]
fn schema_too_new_display_names_path_and_versions() {
    let err = Error::SchemaTooNew {
        path: "/tmp/corpus.db".into(),
        found: 7,
        supported: 1,
    };
    assert_eq!(
        err.to_string(),
        "corpus at /tmp/corpus.db has schema version 7, but this codecite supports up to 1; upgrade codecite"
    );
    assert!(err.source().is_none());
}
```

Run: `cargo test -p codecite-core error`
Expected: FAIL to compile (`no variant named Sqlite`).

- [ ] **Step 4: Add the variants**

Follow the existing pattern. Each variant gets a doc comment. `Sqlite` uses `#[error("SQLite error")]` and does not repeat its source. `SchemaTooNew` uses the exact message from the test, and its fields get doc comments like `InvalidCorpus`'s.

Run: `cargo test -p codecite-core error`
Expected: PASS (6 tests).

- [ ] **Step 5: Gates and commit**

Run the Global Constraints gates. Expected: all succeed.

```bash
git add Cargo.toml Cargo.lock deny.toml crates/codecite-core/Cargo.toml crates/codecite-core/src/error.rs
git commit -m "feat(core): add rusqlite and the store error variants (#14)"
```

---

### Task 2: `Store` with numbered migrations

**Files:**
- Create: `crates/codecite-core/migrations/0001_initial.sql`
- Create: `crates/codecite-core/src/store.rs` (code and `#[cfg(test)] mod tests`)
- Modify: `crates/codecite-core/src/lib.rs` (`mod store; pub use store::{Store, SCHEMA_VERSION};`)

**Interfaces:**
- Consumes: `Error::Sqlite`, `Error::SchemaTooNew`, `Error::InvalidCorpus` (Task 1 and #7).
- Produces:
  - `pub const SCHEMA_VERSION: u32`, equal to `MIGRATIONS.len()`.
  - `pub struct Store` (private fields `conn: rusqlite::Connection` and `path: PathBuf`).
  - `impl Store { pub fn open_read_write(path: &Path) -> Result<Store>; pub fn open_read_only(path: &Path) -> Result<Store>; pub fn schema_version(&self) -> Result<u32>; pub fn path(&self) -> &Path }`
  - Later issues add indexing and query methods on `Store`. This task adds no others.

- [ ] **Step 1: Write `0001_initial.sql`**

Copy the `docs/design.md#data-model` SQL verbatim, with the two changes listed in Global Constraints: drop the `chunk_vec` statement and use `content='', contentless_delete=1` on `chunk_fts`. The file has no `BEGIN`/`COMMIT`, because the caller owns the transaction. A one-line header comment says that an FTS row's `rowid` is its `chunk.id`, and that contentless-delete FTS rows are deleted by rowid and can only be updated as a whole row.

- [ ] **Step 2: Write the failing tests**

Test helpers in the `tests` module:
- `fn scratch(name: &str) -> PathBuf` returns a fresh, empty directory `temp_dir()/codecite-store-<pid>-<name>` (removed first if present).
- `fn raw(path: &Path) -> rusqlite::Connection` opens the file directly, so a test can set up a file without going through `Store`.

Each test cleans up its directory at the end.

| Test | Arrange / act | Assert |
| --- | --- | --- |
| `fresh_file_gets_latest_version` | `open_read_write` on a missing `corpus.db` | `schema_version()? == SCHEMA_VERSION`; `PRAGMA journal_mode` returns `"wal"`; `sqlite_master` has tables `meta`, `source`, `document`, `chunk`, `chunk_fts`, `symbol`, `edge` |
| `reopening_is_a_no_op` | open rw, insert a `source` row, drop; open rw again | version unchanged; the `source` row is still there (migration 1 would fail on `CREATE TABLE` if it ran twice) |
| `read_write_refuses_newer_schema_unchanged` | open rw, then via `raw` set `schema_version` to `SCHEMA_VERSION + 1`; close; read the file bytes | `open_read_write` returns `Err(Error::SchemaTooNew { found, supported, .. })` with `found == SCHEMA_VERSION + 1` and `supported == SCHEMA_VERSION`; file bytes unchanged |
| `read_only_refuses_newer_schema` | same file setup | `open_read_only` returns `Err(Error::SchemaTooNew { .. })` |
| `read_only_handle_cannot_write` | open rw, drop; `open_read_only` | `store.conn.execute("INSERT INTO meta VALUES ('k','v')", [])` fails with `rusqlite::Error::SqliteFailure(e, _)` where `e.code == rusqlite::ErrorCode::ReadOnly` |
| `read_only_missing_file_is_invalid_and_not_created` | `open_read_only` on a missing path | `Err(Error::InvalidCorpus { .. })`; `!path.exists()` |
| `read_only_refuses_older_schema` | via `raw`: `CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES ('schema_version','0')` | `open_read_only` returns `Err(Error::InvalidCorpus { reason, .. })` and `reason` contains `"codecite index"` |
| `read_write_refuses_non_sqlite_file_unchanged` | write `b"not a database, just text\n"` to the path | `open_read_write` is `Err`; file bytes unchanged |
| `read_write_refuses_foreign_database_unchanged` | via `raw`: `CREATE TABLE notes(x)`; close; read bytes | `open_read_write` returns `Err(Error::InvalidCorpus { .. })`; file bytes unchanged |
| `concurrent_first_opens_both_succeed` | two `std::thread::spawn`s each call `open_read_write` on the same missing path; join both | both `Ok`; each `schema_version()? == SCHEMA_VERSION` |
| `fts_rows_delete_by_rowid` | open rw; insert an FTS row (rowid 1, `terms = 'validate token validateToken'`); `DELETE FROM chunk_fts WHERE rowid = 1` | the `MATCH 'validate'` count is 1 before the delete and 0 after |
| `every_migration_file_is_registered` | list `concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")` | the `.sql` files are named `0001_*`…`NNNN_*` with no gaps, and `NNNN == SCHEMA_VERSION` |

Run: `cargo test -p codecite-core store`
Expected: FAIL to compile (`store` module and `Store` missing).

- [ ] **Step 3: Implement `store.rs`**

`const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_initial.sql")];`. Index `i` holds version `i + 1`.

Learn: `include_str!` reads the file at compile time and embeds it in the binary as a `&'static str`. The binary needs no migrations directory at run time, and a missing file is a build error, not a run-time one. The path is relative to the source file that calls the macro.

A private helper, `fn read_version(conn: &Connection, path: &Path) -> Result<u32>`:
- If the `meta` table exists, parse its `schema_version` value as `u32`. A missing or unparsable value → `InvalidCorpus { reason: "unreadable schema_version" }`.
- If there is no `meta` table, return 0 when `sqlite_master` is empty. Otherwise return `InvalidCorpus { reason: "not a codecite corpus" }`.

`open_read_write`:
1. Open with `Connection::open` (creates the file if missing). Set `busy_timeout` to 5 s, so a second indexer waits for the first one's lock instead of failing at once.
2. Start a transaction with `conn.transaction_with_behavior(TransactionBehavior::Immediate)`. Read the version inside it. Newer than `SCHEMA_VERSION` → `SchemaTooNew`, with the transaction dropped (rolled back) unwritten. Otherwise, for each missing migration, run `execute_batch(sql)`, then upsert `('schema_version', N)` into `meta`. Commit.
3. Only after the commit, set `PRAGMA journal_mode = WAL` and check it reports `wal`. It can't run inside a transaction. Running it after the version check keeps a refused file's header untouched. It is a no-op on a file already in WAL.

Learn: a `rusqlite::Transaction` borrows the `Connection` mutably and rolls back when dropped unless you call `commit()`. So every early `?` return from a migration leaves the file at its old version. `BEGIN IMMEDIATE` takes the write lock up front. Two indexers racing on a fresh file run one after the other, and the second one reads the version the first one committed.

`open_read_only`:
- A missing path → `InvalidCorpus { reason: "no such file" }`, checked before opening, because SQLite's own error doesn't name the path.
- Open with `Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)`.
- Read the version. Newer → `SchemaTooNew`. Older (including 0) → `InvalidCorpus { reason: format!("schema version {found} is older than {SCHEMA_VERSION}; re-run codecite index to upgrade it") }`. A read-only handle can't migrate, and the design says not to guess at an old schema.

Learn: `OpenFlags::SQLITE_OPEN_READ_ONLY` makes SQLite itself reject every write with `SQLITE_READONLY`. The guarantee comes from the database engine, not from the methods `Store` happens to expose.

Doc comments on every public item, as `missing_docs` requires.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite-core`
Expected: PASS (all store and error tests).

- [ ] **Step 5: Exercise the real file**

Run `cargo test -p codecite-core fresh_file_gets_latest_version` with the cleanup temporarily skipped. Then point the `sqlite3` CLI at the file, if one is installed, and run `PRAGMA journal_mode;` and `SELECT * FROM meta;`. Expected: `wal` and `schema_version|1`. Restore the cleanup. If no `sqlite3` is installed, say so in the PR.

- [ ] **Step 6: Gates and commit**

Run the Global Constraints gates. Expected: all succeed.

```bash
git add crates/codecite-core/migrations/0001_initial.sql crates/codecite-core/src/store.rs crates/codecite-core/src/lib.rs
git commit -m "feat(core): open corpus files with numbered schema migrations (#14)"
```

---

### Task 3: Settle the FTS5 questions in `docs/design.md`

**Files:**
- Modify: `docs/design.md` (`## Data model` SQL and bullets, `## Decisions and open questions`)

- [ ] **Step 1: Update the data model**

In the SQL block, make `chunk_fts` use `content='', contentless_delete=1`. Extend the **Migrations** bullet to cover the rest of the behavior:
- Migrations live in `crates/codecite-core/migrations/`.
- Queries open the file read-only and refuse an older schema instead of migrating it.
- Only indexing opens the file read-write and migrates it.

- [ ] **Step 2: Move the two answers into the decisions table**

Add two rows and delete the two questions:

| Decision | Choice | Why | Revisit if |
| --- | --- | --- | --- |
| Identifier splitting | A stored `terms` column, filled at index time | Simpler. A custom FTS5 tokenizer means C callbacks over FFI, and the workspace forbids `unsafe` | Index size on disk becomes a problem |
| FTS5 updates | Contentless table with `contentless_delete=1`; the bundled SQLite (3.53.2, via `rusqlite`'s `bundled` feature) supports it | Deletes by rowid without storing the text twice. Rows are updated whole, never column by column | A `rusqlite` bump changes the bundled SQLite's FTS5 options |

Update the intro line from "Nine decisions … seven questions" to "Eleven decisions … five questions".

- [ ] **Step 3: Check and commit**

Run: `grep -n "contentless_delete\|Eleven decisions\|custom FTS5 tokenizer?" docs/design.md`
Expected: the SQL line and the decisions row mention `contentless_delete`; the intro line says Eleven; the old question line is gone.

```bash
git add docs/design.md
git commit -m "docs: settle the FTS5 terms and contentless-delete questions (#14)"
```
