# Derived-data versions, application_id and stale pins Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Version the data an index run derives from a document, so a chunker or terms change re-splits only the documents written under an older version; let readers open an older schema whose later migrations are read-compatible; mark codecite's files with `PRAGMA application_id`; and report a manifest pin that the corpus no longer matches (#252).

**Architecture:** The version constants sit where #262 puts them: `TERMS_VERSION` in `codecite-core/src/terms.rs`, `FTS_TOKENIZER` in `codecite-core/src/store/run.rs`, `CHUNKER_VERSION` in `codecite-index/src/chunk.rs`. `Store::write_split_document` stamps a document with the caller's chunker version and this build's terms version; `codecite-index` compares both before skipping an unchanged file. Each migration declares whether this build's readers can read a file that stops before it. Pin staleness is computed from the manifest whose path the index run stores in `meta.manifest_path`; `codecite-index` turns a manifest into pins, and the CLI hands that reader to `corpora` and to the MCP server.

**Tech Stack:** Rust, rusqlite (bundled SQLite), rmcp.

**Spec:** issue #252 and `docs/design.md` (Data model, Indexing and retrieval, Freshness).

## Global Constraints

- Old migrations (0001–0009) are not rewritten.
- `meta` key names: `tokenizer`, `manifest_path`. #248 adds `manifest_hash`.
- Constant names and files match #262 (#248): `TERMS_VERSION` (core `terms.rs`), `CHUNKER_VERSION` (index `chunk.rs`, exported from the crate), `FTS_TOKENIZER` (core `store/run.rs`).
- The migration is numbered next after `main`'s highest. #260 and #248 also claim 0010; whichever lands later renumbers.
- No status words; issues by number.

## Review Focus

- A corpus at the previous schema is opened by `codecite mcp` after an upgrade: it answers, with a warning in `corpora` and on stderr.
- A foreign SQLite file with its own `application_id` and a `meta` table: refused as not a codecite corpus.
- A codecite file from before `application_id` (id 0, `meta` present): accepted and stamped on its next index.
- The manifest moved or deleted since the index run: `stale_pins` is left out, never an error.
- A document indexed at version 0 whose bytes didn't change: re-split by the next run; one already current: left alone.

---

### Task 1: Version fields and application_id in the store

**Files:**
- Create: `crates/codecite-core/migrations/0010_derived_versions.sql`
- Modify: `crates/codecite-core/src/store.rs`, `store/write.rs`, `terms.rs`, `lib.rs`

**Interfaces:**
- Produces: `pub const TERMS_VERSION: u32 = 1`, `pub const FTS_TOKENIZER: &str = "unicode61"`, `pub const APPLICATION_ID: i32`; `Store::write_split_document(source_id, &DocumentRecord, chunker_version: u32)` (`write_document` writes chunker version 0); `Store::stored_documents(source_id) -> Result<HashMap<String, StoredDocument>>` with `StoredDocument { content_hash, chunker_version, terms_version }` and `StoredDocument::is_current(&self, hash: &[u8], chunker_version: u32) -> bool`. `document_hashes` goes.

- [ ] Write failing tests in `store.rs`: `version_11_stamps_stored_documents_and_records_the_tokenizer`, `version_8_refills_the_full_text_table_in_its_own_transaction` (an upgrade whose index run then fails still answers, with the right totals), `a_fresh_file_has_the_application_id`, `a_foreign_application_id_is_refused_unchanged`, `a_codecite_file_without_an_application_id_is_accepted_and_stamped`; in `write.rs` `written_documents_carry_the_current_versions`.
- [ ] Migration adds `document.chunker_version` and `document.terms_version` (`INTEGER NOT NULL DEFAULT 0`), sets both to 1 for existing rows, and inserts `meta.tokenizer = 'unicode61'`.
- [ ] `open_read_write` sets `PRAGMA application_id` inside the migration transaction. `read_version` refuses a non-zero foreign id and falls back to the `meta` check for id 0.
- [ ] `fill_fts`'s copy of the FTS rule goes: `write::fts_row` is the one rule, used by `insert_chunks` and by version 8's refill, which stays inside the migration transaction so a failed index run never leaves `chunk_fts` empty (review on #264).
- [ ] `write_document` stamps both versions. Run `cargo test -p codecite-core`, then commit.

### Task 2: Readers open a read-compatible older schema

**Files:** Modify `crates/codecite-core/src/store.rs`, `store/corpora.rs`.

**Interfaces:**
- Produces: `struct Migration { sql: &'static str, read_compatible: bool }`; `Store::schema_warning(&self) -> Option<&str>`; `CorpusInfo.warning: Option<String>`.

- [ ] Failing tests: `read_only_opens_a_read_compatible_older_schema_with_a_warning` (a file one compatible migration behind opens; the warning names both versions and `codecite index`), `read_only_refuses_an_older_schema_behind_an_incompatible_migration` (the existing v0 test still refuses), `corpora_carry_the_schema_warning`.
- [ ] Implement it, run the tests and commit.

### Task 3: Index runs re-split documents whose version is behind

**Files:** Modify `crates/codecite-index/src/index.rs`, `chunk.rs` (`pub const CHUNKER_VERSION: u32 = 1`), `lib.rs`.

- [ ] Failing test `a_version_bump_resplits_only_the_documents_behind_it`: index a local source of two files, a document and a guidance file; set `chunker_version = 0` on one file and on the document; reindex. `written == 2`, `unchanged == 2`, and the two have the current versions again. The guidance file gets `terms_version = 0` the same way.
- [ ] `index_tree`, `index_document` and `index_guidance` skip only when `StoredDocument::is_current`. Run the tests and commit.

### Task 4: Stale pins in the freshness output

**Files:** Modify `crates/codecite-core/src/store/corpora.rs`, `store/write.rs`, `crates/codecite-index/src/index.rs` (or `manifest.rs`), `crates/codecite-mcp/src/server.rs`, `crates/codecite/src/main.rs`.

**Interfaces:**
- Produces: `pub struct ManifestPin { pub source: String, pub revision: String }` (`Pin` is taken by citations); `pub type PinReader = fn(&Path) -> Option<Vec<ManifestPin>>`; `Store::set_manifest_path(&mut self, &Path)`; `Store::stale_pins(&self, &[Pin]) -> Result<Vec<StalePin>>` with `StalePin { source, pinned, stored: Vec<String> }`; `corpora(dir, Option<PinReader>)`; `codecite_index::pins(&Manifest) -> Vec<ManifestPin>` and `codecite_index::manifest_pins(&Path) -> Option<Vec<ManifestPin>>`; `Server::with_pins(self, PinReader)`, `serve_stdio(data_dir, Option<PinReader>)`.
- `CorpusInfo` adds `stale_pins` (left out when the manifest can't be read) and `pins_stale: bool` next to `stale`.

- [ ] Failing tests: core `a_pin_the_corpus_lacks_is_stale` (git name with another revision, a document version not stored, a source not stored at all), `corpora_report_stale_pins_from_the_stored_manifest`, `a_missing_manifest_leaves_pins_out`; index `manifest_pins_name_git_documents_and_pinned_guidance` (local and excluded sources give none).
- [ ] CLI `index` stores the canonical manifest path; `corpora` and `mcp` pass `codecite_index::manifest_pins`. Run the tests and commit.

### Task 5: Docs

- [ ] `docs/design.md`: Data model (`document` version columns, `meta` keys), the Migrations bullet (read-compatible flag, application_id, version marking replacing the `content_hash = x''` trick), Indexing step 5, Freshness (pin staleness), `corpora` output. `docs/usage.md`: the `corpora` example and fields.
- [ ] `cargo fmt --check`, clippy with `-D warnings`, `cargo test --workspace --locked`; commit.
