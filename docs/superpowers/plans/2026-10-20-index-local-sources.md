# Index Local Sources Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #21: `codecite index <corpus>` reads `corpora/<corpus>.toml` (or `--manifest <path>`), walks each `local` source (#19), chunks every file whose content changed (#20), splits identifiers into terms (#18) and writes documents, chunks, symbols and full-text rows into the corpus file (#14), one transaction per document. Unchanged documents are skipped, deleted ones removed, and the run prints a summary.

**Architecture:** Three layers, one task each.

1. `codecite-core` owns the schema, so it owns the writes. A new `store/write.rs` adds `Store` methods (`upsert_source`, `document_hashes`, `write_document`, `set_dirty`, `delete_document`, `delete_sources_except`, `set_indexed_at`) that take plain record structs (`SourceRecord`, `DocumentRecord`, `ChunkRecord`, `SymbolRecord`), so core still depends on no other codecite crate. `write_document` computes the search terms itself with a new `text_terms` (the `identifier_terms` of every word in a text), so the query side (#26) can use the same function. Migration `0002_incremental_index.sql` adds `document.dirty` (#26 shows it on code results, and only the index run knows it) and indexes on `chunk(document_id)`, `symbol(document_id)` and `edge(evidence_doc)`.
2. `codecite-index` gets `index_corpus(&mut Store, &Manifest, &WalkOptions) -> Result<IndexSummary>`. For each local source: walk it, upsert the source row (HEAD as `revision`), and for each kept file read the bytes, hash them with sha256, skip an unchanged hash (updating only `dirty`), else chunk and write. Stored documents not kept this run are deleted, then every source the run did not index. `IndexSummary` implements `Display` as the printed summary.
3. The `codecite` binary gets a `clap` subcommand, `index`, that validates the corpus name, loads the manifest, opens `<data dir>/codecite/<corpus>.db` (the `directories` crate) and prints the summary. Errors get `anyhow` context at each step.

**Tech Stack:** Rust 1.99.1 (edition 2024). New dependencies in `[workspace.dependencies]`: `sha2` 0.11 (MIT OR Apache-2.0) for content hashes, in `codecite-index`; `directories` 6 (MIT OR Apache-2.0) for the data directory, in `codecite`. `directories` pulls in `option-ext` 0.2, which is MPL-2.0, so `deny.toml` gets a license exception for that one crate. `rusqlite` becomes a dev-dependency of `codecite-index` (its tests read the corpus file), `tempfile` a dev-dependency of `codecite`. Prototyped: `cargo deny check` passes with no duplicate versions.

**Spec:** Issue #21; `docs/design.md#data-model`, `#indexing-and-retrieval` (step 5 and Freshness), `#corpus-manifests-and-licensing` (local sources), `#interfaces`.

## Global Constraints

- Each document's document row, chunks, symbols and full-text rows are written in one transaction (`write_document`); deleting a document is one transaction too. A failed write leaves the old document as it was.
- Incremental: the content hash is the sha256 of the file's bytes. A stored document with the same hash is not rewritten; its `dirty` flag is still updated, since committing a file changes `dirty` without changing its bytes.
- A stored document whose file is not kept this run (deleted, newly ignored, excluded, binary, oversized, non-UTF-8 content, ...) is deleted with its chunks, full-text rows, symbols and edges.
- The full-text table is contentless with `contentless_delete=1`: its rows are deleted by rowid, one at a time, and a row's rowid is its chunk's `id`.
- Every source the run did not index is deleted from the store: a source removed from the manifest, or a local source now marked `use = "excluded"`. Git and document sources are not indexed yet (#29, #25); they are listed in the summary as not indexed and have no rows.
- A local source without `name` is named after its checkout directory. Two indexed sources with one name are an error (`Error::DuplicateSource`).
- Files whose contents are not UTF-8 are skipped with a new reason, `SkipReason::NonUtf8Content`; the walk never produces it.
- The corpus file is `<data dir>/codecite/<corpus>.db` (`~/.local/share/codecite/<corpus>.db` on Linux), never in a repository. A corpus name is ASCII letters, digits, `-`, `_`, `.`, not starting with `.` (it becomes a file name, so `../x` is refused), and must equal the manifest's `name`.
- The read-write store sets `PRAGMA synchronous = NORMAL`: in WAL mode a commit then costs no fsync, and a power cut can lose recent commits but never corrupts the file. The index commits once per document.
- The summary prints documents written, unchanged and removed, chunks written, skipped files by reason, sources not indexed or removed, and elapsed time.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- Only the changed document is rewritten: `reindexing_rewrites_only_the_changed_document` (Task 2) checks the other documents keep their chunk IDs, the summary counts 1 written and 2 unchanged, the old symbol no longer matches and the dirty flags.
- Deleted files lose their chunks and full-text rows: `a_deleted_file_loses_its_chunks`, `a_file_that_becomes_skipped_is_removed` (Task 2), `delete_document_removes_everything_stored_for_it` (Task 1).
- Transactions: `a_failed_write_leaves_the_old_document` (Task 1) forces a primary-key failure mid-document and checks the old rows survive.
- Re-indexing stays linear: `document_lookups_use_an_index` (Task 1) checks the query plans. Without the indexes the prototype took 53 s to index 2,934 files (ralph-orchestrator and pi, 45,861 chunks); with them, 8 s, and a re-run with nothing changed 0.07 s for this repository.
- The corpus file is never in the repository and the name can't escape the data directory: `index_writes_the_corpus_file_and_reindexes_incrementally`, `index_errors_name_the_problem` (Task 3, end to end through the binary with `XDG_DATA_HOME` set; Linux only, since other platforms ignore it).
- Known limits, not bugs: a chunker change in a later codecite does not re-chunk unchanged files (no chunker version is stored yet); SQLite may reuse a deleted chunk's ID for a new chunk, so IDs are not stable across rewrites of a document; the summary counts a file deleted between the walk and the read as `deleted`.

Prototyped in full before planning (`/var/tmp/index-proto`): every test below passes, clippy, fmt and deny are clean on 1.99.1, and the release binary indexed this repository and two larger checkouts (2,934 files) twice, the second run with every document unchanged.

---

### Task 1: Write documents into the store (`codecite-core`)

**Files:**
- Create: `crates/codecite-core/migrations/0002_incremental_index.sql`
- Create: `crates/codecite-core/src/store/write.rs`
- Modify: `crates/codecite-core/src/store.rs` (register the migration, `mod write`, `synchronous = NORMAL`)
- Modify: `crates/codecite-core/src/terms.rs` (`text_terms`)
- Modify: `crates/codecite-core/src/lib.rs` (re-exports)

- [ ] **Step 1: Write the failing tests for `text_terms`**

Append to the `tests` module in `crates/codecite-core/src/terms.rs`:

```rust
    #[test]
    fn text_terms_split_each_word() {
        assert_eq!(
            text_terms("fn validateToken(id_token: &str) -> bool {"),
            "fn validate token validateToken id token id_token str bool"
        );
        assert_eq!(text_terms("back-channel"), "back channel");
        assert_eq!(text_terms("src/client.rs"), "src client rs");
    }

    #[test]
    fn text_terms_skip_punctuation_only_words() {
        assert_eq!(text_terms(""), "");
        assert_eq!(text_terms("__ -> {}"), "");
        assert_eq!(text_terms("__init__()"), "init __init__");
    }
```

- [ ] **Step 2: Write the failing store tests**

Create `crates/codecite-core/src/store/write.rs` holding only this test module for now (the implementation goes above it in Step 4):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// A fresh store in its own scratch directory.
    fn store(name: &str) -> (Store, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("codecite-write-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (Store::open_read_write(&dir.join("corpus.db")).unwrap(), dir)
    }

    fn source(name: &str) -> SourceRecord<'_> {
        SourceRecord {
            name,
            kind: "local",
            url: "/src/app",
            revision: "0123456789abcdef0123456789abcdef01234567",
            license_spdx: Some("MIT"),
            license_class: "adapt",
        }
    }

    fn document<'a>(path: &'a str, text: &'a str) -> DocumentRecord<'a> {
        DocumentRecord {
            path,
            language: Some("rust"),
            content_hash: text.as_bytes(),
            size_bytes: text.len() as u64,
            dirty: false,
            chunks: vec![ChunkRecord {
                kind: "function",
                start_line: 1,
                end_line: 1,
                text,
                title: None,
                symbol: Some(SymbolRecord {
                    id: format!("codecite . app . {path}/validateToken()."),
                    display_name: "validateToken",
                    kind: "function",
                }),
            }],
        }
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store.conn.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    fn matches(store: &Store, query: &str) -> i64 {
        store
            .conn
            .query_row(
                "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH ?1",
                [query],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn upsert_source_keeps_the_id() {
        let (mut store, dir) = store("upsert");
        let id = store.upsert_source(&source("app")).unwrap();
        let mut moved = source("app");
        moved.revision = "fedcba9876543210fedcba9876543210fedcba98";
        assert_eq!(store.upsert_source(&moved).unwrap(), id);
        assert_ne!(store.upsert_source(&source("other")).unwrap(), id);
        let revision: String = store
            .conn
            .query_row("SELECT revision FROM source WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(revision, moved.revision);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_document_stores_chunks_symbols_and_search_rows() {
        let (mut store, dir) = store("write");
        let id = store.upsert_source(&source("app")).unwrap();
        let text = "fn validateToken() {}";
        assert_eq!(
            store
                .write_document(id, &document("src/lib.rs", text))
                .unwrap(),
            1
        );

        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM symbol"), 1);
        assert_eq!(matches(&store, "symbol:validate"), 1);
        assert_eq!(matches(&store, "path:lib"), 1);
        assert_eq!(matches(&store, "terms:token"), 1);
        let hashes = store.document_hashes(id).unwrap();
        assert_eq!(hashes["src/lib.rs"], text.as_bytes());
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rewriting_a_document_replaces_its_rows() {
        let (mut store, dir) = store("rewrite");
        let id = store.upsert_source(&source("app")).unwrap();
        store
            .write_document(id, &document("src/lib.rs", "fn validateToken() {}"))
            .unwrap();
        let mut doc = document("src/lib.rs", "fn refreshSession() {}");
        doc.chunks[0].symbol.as_mut().unwrap().display_name = "refreshSession";
        store.write_document(id, &doc).unwrap();

        assert_eq!(count(&store, "SELECT count(*) FROM document"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM symbol"), 1);
        assert_eq!(matches(&store, "token"), 0);
        assert_eq!(matches(&store, "refresh"), 1);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_the_old_document() {
        let (mut store, dir) = store("atomic");
        let id = store.upsert_source(&source("app")).unwrap();
        store
            .write_document(id, &document("src/lib.rs", "fn validateToken() {}"))
            .unwrap();
        // Two chunks with one symbol ID break the symbol primary key.
        let mut doc = document("src/lib.rs", "fn refreshSession() {}");
        doc.chunks.push(doc.chunks[0].clone());
        assert!(store.write_document(id, &doc).is_err());

        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 1);
        assert_eq!(matches(&store, "token"), 1);
        assert_eq!(matches(&store, "refresh"), 0);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_document_removes_everything_stored_for_it() {
        let (mut store, dir) = store("delete");
        let id = store.upsert_source(&source("app")).unwrap();
        store
            .write_document(id, &document("src/lib.rs", "fn validateToken() {}"))
            .unwrap();
        store
            .write_document(id, &document("src/other.rs", "fn validateToken() {}"))
            .unwrap();
        store.delete_document(id, "src/lib.rs").unwrap();

        assert_eq!(count(&store, "SELECT count(*) FROM document"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM symbol"), 1);
        assert_eq!(matches(&store, "path:lib"), 0);
        assert_eq!(matches(&store, "path:other"), 1);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_sources_except_drops_the_others() {
        let (mut store, dir) = store("sources");
        for name in ["app", "old"] {
            let id = store.upsert_source(&source(name)).unwrap();
            store
                .write_document(
                    id,
                    &document(&format!("{name}.rs"), "fn validateToken() {}"),
                )
                .unwrap();
        }
        assert_eq!(store.delete_sources_except(&["app"]).unwrap(), ["old"]);
        assert_eq!(count(&store, "SELECT count(*) FROM source"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 1);
        assert_eq!(matches(&store, "path:old"), 0);
        assert!(store.delete_sources_except(&["app"]).unwrap().is_empty());
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_dirty_and_indexed_at() {
        let (mut store, dir) = store("dirty");
        let id = store.upsert_source(&source("app")).unwrap();
        store
            .write_document(id, &document("src/lib.rs", "fn validateToken() {}"))
            .unwrap();
        store.set_dirty(id, "src/lib.rs", true).unwrap();
        assert_eq!(count(&store, "SELECT dirty FROM document"), 1);
        store.set_indexed_at().unwrap();
        let at: String = store
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'indexed_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(at.len(), "2026-10-20T06:00:00Z".len(), "{at}");
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn document_lookups_use_an_index() {
        let (store, dir) = store("plan");
        for (sql, index) in [
            (
                "SELECT id FROM chunk WHERE document_id = 1",
                "chunk_document",
            ),
            (
                "DELETE FROM symbol WHERE document_id = 1",
                "symbol_document",
            ),
            ("DELETE FROM edge WHERE evidence_doc = 1", "edge_evidence"),
        ] {
            let plan: Vec<String> = store
                .conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap()
                .query_map([], |row| row.get(3))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(
                plan.iter().any(|step| step.contains(index)),
                "{sql}: {plan:?}"
            );
        }
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }
}
```

Create the migration `crates/codecite-core/migrations/0002_incremental_index.sql`:

```sql
-- Re-indexing replaces a document's chunks and symbols, so both are looked up by document.

-- Whether a local source's file differed from HEAD (modified, staged or untracked) when it was indexed.
ALTER TABLE document ADD COLUMN dirty INTEGER NOT NULL DEFAULT 0;

CREATE INDEX chunk_document ON chunk(document_id);
CREATE INDEX symbol_document ON symbol(document_id);
CREATE INDEX edge_evidence ON edge(evidence_doc);
```

In `crates/codecite-core/src/store.rs`, register it and declare the module:

```rust
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_incremental_index.sql"),
];
```

```rust
use crate::{Error, Result};

mod write;
pub use write::{ChunkRecord, DocumentRecord, SourceRecord, SymbolRecord};
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p codecite-core`
Expected: compile errors: `text_terms`, `SourceRecord`, `upsert_source` and the rest are not defined.

- [ ] **Step 4: Implement**

Add `text_terms` to `crates/codecite-core/src/terms.rs`, after `identifier_terms`:

```rust
/// Returns the search terms for a run of text: the [`identifier_terms`] of
/// each word in it, separated by spaces.
///
/// A word is a run of letters, digits and underscores that holds at least
/// one letter or digit. Hyphens and every other character separate words, so
/// `back-channel` gives the terms of `back` and `channel`.
pub fn text_terms(text: &str) -> String {
    let mut terms = String::new();
    let words = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| word.chars().any(char::is_alphanumeric));
    for word in words {
        if !terms.is_empty() {
            terms.push(' ');
        }
        terms.push_str(&identifier_terms(word));
    }
    terms
}
```

> Learn: `str::split` takes a closure as the separator test, and `filter` drops the empty and punctuation-only pieces it leaves, so no index arithmetic is needed. `char::is_alphanumeric` passed by name works where a closure `|c| c.is_alphanumeric()` would.

Put the implementation above the test module in `crates/codecite-core/src/store/write.rs`:

```rust
//! Writing sources and their documents into the store. Each call is one
//! transaction, so a document is either fully written or not at all.

use std::collections::HashMap;

use rusqlite::{Transaction, params};

use super::Store;
use crate::{Result, identifier_terms, text_terms};

/// A source to create or update, matched by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecord<'a> {
    /// The name results cite; unique in the corpus.
    pub name: &'a str,
    /// `local`, `git` or `document`.
    pub kind: &'a str,
    /// The repository URL, local path or document URL.
    pub url: &'a str,
    /// The commit SHA, or a document's sha256.
    pub revision: &'a str,
    /// The license as the manifest writes it.
    pub license_spdx: Option<&'a str>,
    /// `adapt`, `approach_only`, `quote` or `excluded`.
    pub license_class: &'a str,
}

/// A document and every chunk in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRecord<'a> {
    /// The path from the source's root, `/`-separated.
    pub path: &'a str,
    /// The language name, when one was detected.
    pub language: Option<&'a str>,
    /// The sha256 of the file's bytes.
    pub content_hash: &'a [u8],
    /// The file's size in bytes.
    pub size_bytes: u64,
    /// Whether the file differs from the source's revision.
    pub dirty: bool,
    /// The chunks, in order.
    pub chunks: Vec<ChunkRecord<'a>>,
}

/// One chunk of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRecord<'a> {
    /// `function`, `class`, `section`, `step` or `window`.
    pub kind: &'a str,
    /// The first line, counting from 1.
    pub start_line: u32,
    /// The last line, inclusive.
    pub end_line: u32,
    /// The chunk's text.
    pub text: &'a str,
    /// A section's heading.
    pub title: Option<&'a str>,
    /// The definition the chunk holds.
    pub symbol: Option<SymbolRecord<'a>>,
}

/// A definition, stored in `symbol`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRecord<'a> {
    /// The SCIP symbol string.
    pub id: String,
    /// The name as written.
    pub display_name: &'a str,
    /// What kind of definition it is.
    pub kind: &'a str,
}

impl Store {
    /// Creates the source named `source.name`, or updates its other fields
    /// and `indexed_at`, and returns its ID.
    pub fn upsert_source(&mut self, source: &SourceRecord<'_>) -> Result<i64> {
        let id = self.conn.query_row(
            "INSERT INTO source (name, kind, url, revision, license_spdx, license_class, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT (name) DO UPDATE SET
               kind = excluded.kind, url = excluded.url, revision = excluded.revision,
               license_spdx = excluded.license_spdx, license_class = excluded.license_class,
               indexed_at = excluded.indexed_at
             RETURNING id",
            params![
                source.name,
                source.kind,
                source.url,
                source.revision,
                source.license_spdx,
                source.license_class,
            ],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// The content hash of each document stored for the source, by path.
    pub fn document_hashes(&self, source_id: i64) -> Result<HashMap<String, Vec<u8>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, content_hash FROM document WHERE source_id = ?1")?;
        let rows = stmt.query_map([source_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces the document at `document.path` in the source, with its
    /// chunks, symbols and full-text rows, in one transaction. Returns how
    /// many chunks were written.
    pub fn write_document(
        &mut self,
        source_id: i64,
        document: &DocumentRecord<'_>,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let document_id: i64 = tx.query_row(
            "INSERT INTO document (source_id, path, language, content_hash, size_bytes, dirty)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (source_id, path) DO UPDATE SET
               language = excluded.language, content_hash = excluded.content_hash,
               size_bytes = excluded.size_bytes, dirty = excluded.dirty
             RETURNING id",
            params![
                source_id,
                document.path,
                document.language,
                document.content_hash,
                // SQLite integers are i64; no file comes near i64::MAX bytes.
                i64::try_from(document.size_bytes).unwrap_or(i64::MAX),
                document.dirty,
            ],
            |row| row.get(0),
        )?;
        clear_document(&tx, document_id)?;
        insert_chunks(&tx, document_id, document)?;
        tx.commit()?;
        Ok(document.chunks.len())
    }

    /// Sets `dirty` on a stored document whose content is unchanged.
    pub fn set_dirty(&mut self, source_id: i64, path: &str, dirty: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE document SET dirty = ?3 WHERE source_id = ?1 AND path = ?2",
            params![source_id, path, dirty],
        )?;
        Ok(())
    }

    /// Deletes the document at `path` in the source, with everything stored
    /// for it, in one transaction.
    pub fn delete_document(&mut self, source_id: i64, path: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let ids = document_ids(
            &tx,
            "source_id = ?1 AND path = ?2",
            params![source_id, path],
        )?;
        delete_documents(&tx, &ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes every source whose name is not in `keep`, with all its
    /// documents, in one transaction. Returns the deleted sources' names,
    /// sorted.
    pub fn delete_sources_except(&mut self, keep: &[&str]) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        let mut names: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, name FROM source ORDER BY name")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        names.retain(|(_, name)| !keep.contains(&name.as_str()));
        for (id, _) in &names {
            let ids = document_ids(&tx, "source_id = ?1", params![id])?;
            delete_documents(&tx, &ids)?;
            tx.execute("DELETE FROM source WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(names.into_iter().map(|(_, name)| name).collect())
    }

    /// Records when the corpus was last indexed, in `meta.indexed_at`.
    pub fn set_indexed_at(&mut self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value)
             VALUES ('indexed_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        Ok(())
    }
}

/// The IDs of the documents matching `filter`, a `WHERE` clause.
fn document_ids(
    tx: &Transaction<'_>,
    filter: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<i64>> {
    let mut stmt = tx.prepare(&format!("SELECT id FROM document WHERE {filter}"))?;
    let ids = stmt.query_map(params, |row| row.get(0))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

/// Deletes each document and everything stored for it.
fn delete_documents(tx: &Transaction<'_>, ids: &[i64]) -> Result<()> {
    let mut delete = tx.prepare_cached("DELETE FROM document WHERE id = ?1")?;
    for &id in ids {
        clear_document(tx, id)?;
        delete.execute([id])?;
    }
    Ok(())
}

/// Deletes a document's chunks, their full-text rows, its symbols and the
/// edges it is evidence for, keeping the document row.
fn clear_document(tx: &Transaction<'_>, document_id: i64) -> Result<()> {
    // A contentless table deletes by rowid, one row at a time.
    let mut chunk_ids = tx.prepare_cached("SELECT id FROM chunk WHERE document_id = ?1")?;
    let mut delete_fts = tx.prepare_cached("DELETE FROM chunk_fts WHERE rowid = ?1")?;
    let ids = chunk_ids.query_map([document_id], |row| row.get::<_, i64>(0))?;
    for id in ids {
        delete_fts.execute([id?])?;
    }
    for table_filter in [
        "chunk WHERE document_id = ?1",
        "symbol WHERE document_id = ?1",
        "edge WHERE evidence_doc = ?1",
    ] {
        tx.prepare_cached(&format!("DELETE FROM {table_filter}"))?
            .execute([document_id])?;
    }
    Ok(())
}

/// Inserts a document's symbols, chunks and full-text rows.
fn insert_chunks(
    tx: &Transaction<'_>,
    document_id: i64,
    document: &DocumentRecord<'_>,
) -> Result<()> {
    let mut symbol = tx.prepare_cached(
        "INSERT INTO symbol (id, display_name, kind, document_id, start_line, end_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut chunk = tx.prepare_cached(
        "INSERT INTO chunk (document_id, start_line, end_line, kind, symbol_id, title, text, terms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    let mut fts = tx.prepare_cached(
        "INSERT INTO chunk_fts (rowid, symbol, path, terms) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let path_terms = text_terms(document.path);
    for c in &document.chunks {
        if let Some(s) = &c.symbol {
            symbol.execute(params![
                s.id,
                s.display_name,
                s.kind,
                document_id,
                c.start_line,
                c.end_line
            ])?;
        }
        let terms = text_terms(c.text);
        let chunk_id = chunk.insert(params![
            document_id,
            c.start_line,
            c.end_line,
            c.kind,
            c.symbol.as_ref().map(|s| &s.id),
            c.title,
            c.text,
            terms,
        ])?;
        let symbol_terms = c
            .symbol
            .as_ref()
            .map_or_else(String::new, |s| identifier_terms(s.display_name));
        fts.execute(params![chunk_id, symbol_terms, path_terms, terms])?;
    }
    Ok(())
}
```

> Learn: `self.conn.transaction()` returns a `Transaction` that borrows the connection mutably (so these methods take `&mut self`) and rolls back when dropped, so every `?` between it and `tx.commit()` undoes the partial document. `prepare_cached` compiles a statement once per connection and hands back the cached one after that; it borrows `tx`, which is why the statements live inside helper functions that end before `commit`. `Statement::insert` runs an `INSERT` and returns the new rowid, which becomes the full-text row's rowid. `params![...]` builds the parameter list from values of different types; `Option<&str>` binds as NULL when `None`. `INSERT ... ON CONFLICT (...) DO UPDATE ... RETURNING id` is SQLite's upsert, giving back the row's ID whether it was inserted or updated. rusqlite has no `ToSql` for `u64` (SQLite integers are `i64`), hence the `i64::try_from`.

In `crates/codecite-core/src/store.rs`, after `enable_wal(&conn, path)?;` in `open_read_write`:

```rust
        // In WAL mode, NORMAL syncs at checkpoints rather than at every
        // commit. A power cut can lose the last commits but never corrupts
        // the file, and an index run commits once per document.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
```

In `crates/codecite-core/src/lib.rs`:

```rust
mod store;
pub use store::{
    ChunkRecord, DocumentRecord, SCHEMA_VERSION, SourceRecord, Store, SymbolRecord,
};

mod terms;
pub use terms::{identifier_terms, split_identifier, text_terms};
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p codecite-core`
Expected: all pass, including `every_migration_file_is_registered` (it checks the new file's `0002_` prefix and that `SCHEMA_VERSION` is now 2).

- [ ] **Step 6: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

```bash
git add crates/codecite-core
git commit -m "feat(core): write documents, chunks and search rows into the store (#21)"
```

---

### Task 2: Index a corpus's local sources (`codecite-index`)

**Files:**
- Create: `crates/codecite-index/src/index.rs`
- Move: `crates/codecite-index/src/walk/test_repo.rs` → `crates/codecite-index/src/test_repo.rs` (the index tests use it too)
- Modify: `crates/codecite-index/src/walk.rs` (`SkipReason::NonUtf8Content`, `SkipReason::label`, test_repo path), `crates/codecite-index/src/walk/git.rs` (test_repo path)
- Modify: `crates/codecite-index/src/manifest.rs` (`LicenseClass::as_str`)
- Modify: `crates/codecite-index/src/error.rs` (`ReadFile`, `DuplicateSource`)
- Modify: `crates/codecite-index/src/lib.rs`, `crates/codecite-index/Cargo.toml`, `Cargo.toml`, `Cargo.lock`

- [ ] **Step 1: Dependencies and the shared test repository**

In the root `Cargo.toml` `[workspace.dependencies]`, after `globset`:

```toml
sha2 = "0.11"
```

In `crates/codecite-index/Cargo.toml`: `sha2.workspace = true` under `[dependencies]` (after `globset`), and `rusqlite.workspace = true` under `[dev-dependencies]`.

Move the test repository so both modules can use it:

```bash
git mv crates/codecite-index/src/walk/test_repo.rs crates/codecite-index/src/test_repo.rs
```

Change its first line to `//! A throwaway git repository for the walk's and the index's tests.`. In `walk.rs`, delete `#[cfg(test)]` and `mod test_repo;` under `mod git;`, and in both `walk.rs` and `walk/git.rs` change `use crate::walk::test_repo::Repo;` to `use crate::test_repo::Repo;`.

- [ ] **Step 2: Write the failing tests**

Create `crates/codecite-index/src/index.rs` holding only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_repo::Repo;
    use rusqlite::Connection;
    use std::path::PathBuf;

    fn local(repo: &Repo, name: Option<&str>) -> Source {
        Source::Local(LocalSource {
            name: name.map(str::to_owned),
            path: repo.path().to_owned(),
            license: Some("MIT".into()),
            license_class: LicenseClass::Adapt,
            include: Vec::new(),
            exclude: Vec::new(),
        })
    }

    fn manifest(sources: Vec<Source>) -> Manifest {
        Manifest {
            name: "test".into(),
            max_age_days: 30,
            sources,
        }
    }

    /// A checkout with a Rust file, a Markdown file and a text file.
    fn fixture() -> Repo {
        let repo = Repo::new();
        repo.write(
            "src/lib.rs",
            "pub fn validate_token() {}\n\npub struct Client;\n",
        );
        repo.write("README.md", "# App\n\nValidates the token.\n");
        repo.write("notes.txt", "refresh sessions daily\n");
        repo.commit_all(&["src/lib.rs", "README.md", "notes.txt"]);
        repo
    }

    /// The corpus file for one test, deleted with the directory.
    struct Corpus {
        dir: tempfile::TempDir,
    }

    impl Corpus {
        fn new() -> Self {
            Corpus {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self) -> PathBuf {
            self.dir.path().join("corpus.db")
        }

        fn index(&self, manifest: &Manifest) -> IndexSummary {
            let mut store = Store::open_read_write(&self.path()).unwrap();
            index_corpus(&mut store, manifest, &WalkOptions::default()).unwrap()
        }

        fn sql(&self) -> Connection {
            Connection::open(self.path()).unwrap()
        }

        /// Each document's chunk IDs, by path.
        fn chunk_ids(&self) -> BTreeMap<String, Vec<i64>> {
            let conn = self.sql();
            let mut stmt = conn
                .prepare(
                    "SELECT d.path, c.id FROM chunk c JOIN document d ON d.id = c.document_id
                     ORDER BY c.id",
                )
                .unwrap();
            let mut ids: BTreeMap<String, Vec<i64>> = BTreeMap::new();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap();
            for row in rows {
                let (path, id) = row.unwrap();
                ids.entry(path).or_default().push(id);
            }
            ids
        }

        fn matches(&self, query: &str) -> i64 {
            self.sql()
                .query_row(
                    "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH ?1",
                    [query],
                    |row| row.get(0),
                )
                .unwrap()
        }
    }

    #[test]
    fn indexes_a_local_source() {
        let repo = fixture();
        let corpus = Corpus::new();
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (3, 0, 0)
        );
        assert_eq!(summary.chunks, 4);
        let ids = corpus.chunk_ids();
        assert_eq!(
            ids.keys().collect::<Vec<_>>(),
            ["README.md", "notes.txt", "src/lib.rs"]
        );
        assert_eq!(corpus.matches("symbol:validate"), 1);
        assert_eq!(corpus.matches("token"), 2);
        let (name, kind, revision, class): (String, String, String, String) = corpus
            .sql()
            .query_row(
                "SELECT name, kind, revision, license_class FROM source",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), kind.as_str(), class.as_str()),
            ("app", "local", "adapt")
        );
        assert_eq!(revision.len(), 40);
        let symbol: String = corpus
            .sql()
            .query_row(
                "SELECT id FROM symbol WHERE display_name = 'Client'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(symbol, "codecite . app . src/`lib.rs`/Client#");
    }

    #[test]
    fn reindexing_rewrites_only_the_changed_document() {
        let repo = fixture();
        let corpus = Corpus::new();
        let manifest = manifest(vec![local(&repo, Some("app"))]);
        corpus.index(&manifest);
        let before = corpus.chunk_ids();

        repo.write("src/lib.rs", "pub fn refresh_session() {}\n");
        let summary = corpus.index(&manifest);

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (1, 2, 0)
        );
        assert_eq!(summary.chunks, 1);
        let after = corpus.chunk_ids();
        assert_eq!(after["README.md"], before["README.md"]);
        assert_eq!(after["notes.txt"], before["notes.txt"]);
        // SQLite may reuse the freed chunk IDs, so check the content instead.
        assert_eq!(after["src/lib.rs"].len(), 1);
        assert_eq!(corpus.matches("symbol:validate"), 0);
        assert_eq!(corpus.matches("symbol:refresh"), 1);
        let dirty: Vec<(String, bool)> = {
            let conn = corpus.sql();
            let mut stmt = conn
                .prepare("SELECT path, dirty FROM document ORDER BY path")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            dirty,
            [
                ("README.md".to_owned(), false),
                ("notes.txt".to_owned(), false),
                ("src/lib.rs".to_owned(), true)
            ]
        );
    }

    #[test]
    fn a_deleted_file_loses_its_chunks() {
        let repo = fixture();
        let corpus = Corpus::new();
        let manifest = manifest(vec![local(&repo, Some("app"))]);
        corpus.index(&manifest);
        assert_eq!(corpus.matches("refresh"), 1);

        std::fs::remove_file(repo.path().join("notes.txt")).unwrap();
        let summary = corpus.index(&manifest);

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (0, 2, 1)
        );
        assert_eq!(summary.skipped[&SkipReason::Deleted], 1);
        assert!(!corpus.chunk_ids().contains_key("notes.txt"));
        assert_eq!(corpus.matches("refresh"), 0);
    }

    #[test]
    fn a_file_that_becomes_skipped_is_removed() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        repo.write("notes.txt", b"binary\0now\n");
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));
        assert_eq!(summary.removed, 1);
        assert_eq!(summary.skipped[&SkipReason::Binary], 1);

        repo.write("notes.txt", b"caf\xe9\n");
        repo.write("README.md", "# App\n");
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));
        assert_eq!(summary.skipped[&SkipReason::NonUtf8Content], 1);
        assert_eq!((summary.written, summary.removed), (1, 0));
        assert_eq!(
            corpus.chunk_ids().keys().collect::<Vec<_>>(),
            ["README.md", "src/lib.rs"]
        );
    }

    #[test]
    fn sources_the_run_does_not_index_are_removed() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        let mut excluded = local(&repo, Some("app"));
        if let Source::Local(l) = &mut excluded {
            l.license_class = LicenseClass::Excluded;
        }
        let summary = corpus.index(&manifest(vec![excluded]));
        assert_eq!(summary.removed_sources, ["app"]);
        assert_eq!(
            summary.skipped_sources,
            [("app".to_owned(), "use = \"excluded\"")]
        );
        assert!(corpus.chunk_ids().is_empty());
        assert_eq!(corpus.matches("token"), 0);
    }

    #[test]
    fn the_name_defaults_to_the_directory_and_must_be_unique() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, None)]));
        let dir = repo.path().canonicalize().unwrap();
        let name: String = corpus
            .sql()
            .query_row("SELECT name FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, dir.file_name().unwrap().to_str().unwrap());

        let mut store = Store::open_read_write(&corpus.path()).unwrap();
        let twice = manifest(vec![local(&repo, Some("app")), local(&repo, Some("app"))]);
        let err = index_corpus(&mut store, &twice, &WalkOptions::default()).unwrap_err();
        assert!(
            matches!(&err, Error::DuplicateSource { name } if name == "app"),
            "{err:?}"
        );
    }

    #[test]
    fn summary_prints_each_count() {
        let summary = IndexSummary {
            written: 2,
            unchanged: 5,
            removed: 1,
            chunks: 9,
            skipped: BTreeMap::from([(SkipReason::Ignored, 3), (SkipReason::Binary, 1)]),
            skipped_sources: vec![("rfc9449".into(), "document sources are not indexed yet")],
            removed_sources: vec!["old".into()],
            elapsed: Duration::from_millis(1250),
        };
        assert_eq!(
            summary.to_string(),
            "documents: 2 written, 5 unchanged, 1 removed\n\
             chunks: 9 written\n\
             skipped: 3 ignored, 1 binary\n\
             source rfc9449 not indexed: document sources are not indexed yet\n\
             source old removed: the run no longer indexes it\n\
             elapsed: 1.25s"
        );
    }
}
```

In `crates/codecite-index/src/lib.rs`, add before `mod manifest;`:

```rust
mod index;
pub use index::{IndexSummary, index_corpus};
```

and before `mod walk;`:

```rust
#[cfg(test)]
mod test_repo;
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p codecite-index`
Expected: compile errors: `IndexSummary`, `index_corpus`, `SkipReason::NonUtf8Content` and `Error::DuplicateSource` are not defined.

- [ ] **Step 4: Implement**

In `crates/codecite-index/src/walk.rs`, add the variant at the end of `SkipReason` and a label for summaries after the enum:

```rust
    /// The path is not valid UTF-8, so no citation can name it.
    NonUtf8Path,
    /// The contents are not valid UTF-8. Found when indexing reads the
    /// whole file, never by the walk.
    NonUtf8Content,
}

impl SkipReason {
    /// A short label for summaries, e.g. `deny-listed`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ignored => "ignored",
            Self::Excluded => "excluded",
            Self::DenyListed => "deny-listed",
            Self::Deleted => "deleted",
            Self::Symlink => "symlink",
            Self::NotAFile => "not a file",
            Self::Oversized => "oversized",
            Self::Binary => "binary",
            Self::NonUtf8Path => "non-UTF-8 path",
            Self::NonUtf8Content => "non-UTF-8 content",
        }
    }
}
```

In `crates/codecite-index/src/manifest.rs`, before `impl Source`:

```rust
impl LicenseClass {
    /// The value written in the manifest and stored in `source.license_class`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adapt => "adapt",
            Self::ApproachOnly => "approach_only",
            Self::Quote => "quote",
            Self::Excluded => "excluded",
        }
    }
}
```

In `crates/codecite-index/src/error.rs`, after the `Chunker` variant:

```rust
    /// A walked file could not be read.
    #[error("reading {}", path.display())]
    ReadFile {
        /// The file's full path.
        path: std::path::PathBuf,
        /// Why reading failed.
        source: std::io::Error,
    },

    /// Two local sources in the manifest have the same name.
    #[error("two sources are named `{name}`; give one of them a different `name`")]
    DuplicateSource {
        /// The shared name.
        name: String,
    },
```

Put the implementation above the test module in `crates/codecite-index/src/index.rs`:

```rust
//! Indexing a corpus: walk each local source, chunk the files whose content
//! changed and write them into the store, one transaction per document.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use codecite_core::{ChunkRecord, DocumentRecord, SourceRecord, Store, SymbolRecord};
use sha2::{Digest, Sha256};

use crate::{
    Chunker, Error, Language, LicenseClass, LocalSource, LocalTree, Manifest, Result, SkipReason,
    Source, WalkOptions, walk_local,
};

/// What an index run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexSummary {
    /// Documents written: new, or with changed content.
    pub written: usize,
    /// Documents left as they were because their content hash is unchanged.
    pub unchanged: usize,
    /// Documents deleted because their file is gone or now skipped.
    pub removed: usize,
    /// Chunks written, over every written document.
    pub chunks: usize,
    /// Files not indexed, counted by reason.
    pub skipped: BTreeMap<SkipReason, usize>,
    /// Manifest sources not indexed, with the reason.
    pub skipped_sources: Vec<(String, &'static str)>,
    /// Sources deleted from the store because the run did not index them.
    pub removed_sources: Vec<String>,
    /// How long the run took.
    pub elapsed: Duration,
}

/// Indexes every local source in `manifest` into `store`.
///
/// A document whose content hash matches the stored one is not rewritten,
/// and a stored document whose file is gone, or is now skipped, is deleted.
/// Git and document sources are reported in
/// [`IndexSummary::skipped_sources`], as are sources marked `excluded`.
/// Every source the run did not index is then deleted from the store.
pub fn index_corpus(
    store: &mut Store,
    manifest: &Manifest,
    options: &WalkOptions,
) -> Result<IndexSummary> {
    let started = Instant::now();
    let mut summary = IndexSummary::default();
    let mut chunker = Chunker::new()?;
    let mut indexed: Vec<String> = Vec::new();
    for source in &manifest.sources {
        let local = match source {
            Source::Local(local) if local.license_class != LicenseClass::Excluded => local,
            Source::Local(local) => {
                let name = local
                    .name
                    .clone()
                    .unwrap_or_else(|| local.path.display().to_string());
                summary.skipped_sources.push((name, "use = \"excluded\""));
                continue;
            }
            Source::Git(git) => {
                let name = git.name.clone().unwrap_or_else(|| git.url.clone());
                summary
                    .skipped_sources
                    .push((name, "git sources are not indexed yet"));
                continue;
            }
            Source::Document(doc) => {
                summary
                    .skipped_sources
                    .push((doc.name.clone(), "document sources are not indexed yet"));
                continue;
            }
        };
        let tree = walk_local(local, options)?;
        let name = source_name(local, &tree);
        if indexed.contains(&name) {
            return Err(Error::DuplicateSource { name });
        }
        index_local(store, &mut chunker, local, &name, &tree, &mut summary)?;
        indexed.push(name);
    }
    let keep: Vec<&str> = indexed.iter().map(String::as_str).collect();
    summary.removed_sources = store.delete_sources_except(&keep)?;
    store.set_indexed_at()?;
    summary.elapsed = started.elapsed();
    Ok(summary)
}

/// The manifest's `name`, else the checkout directory's name.
fn source_name(local: &LocalSource, tree: &LocalTree) -> String {
    local.name.clone().unwrap_or_else(|| {
        tree.root
            .file_name()
            .map_or_else(|| "local".to_owned(), |n| n.to_string_lossy().into_owned())
    })
}

/// Writes one walked local source into the store.
fn index_local(
    store: &mut Store,
    chunker: &mut Chunker,
    local: &LocalSource,
    name: &str,
    tree: &LocalTree,
    summary: &mut IndexSummary,
) -> Result<()> {
    let url = tree.root.to_string_lossy();
    let source_id = store.upsert_source(&SourceRecord {
        name,
        kind: "local",
        url: &url,
        revision: &tree.head,
        license_spdx: local.license.as_deref(),
        license_class: local.license_class.as_str(),
    })?;
    for (reason, count) in tree.skip_counts() {
        *summary.skipped.entry(reason).or_default() += count;
    }

    let stored = store.document_hashes(source_id)?;
    let mut kept = HashSet::new();
    for file in &tree.files {
        let bytes = match read(&tree.root, &file.path)? {
            Some(bytes) => bytes,
            None => {
                *summary.skipped.entry(SkipReason::Deleted).or_default() += 1;
                continue;
            }
        };
        let hash = Sha256::digest(&bytes);
        if stored
            .get(&file.path)
            .is_some_and(|old| old[..] == hash[..])
        {
            store.set_dirty(source_id, &file.path, file.dirty)?;
            kept.insert(file.path.as_str());
            summary.unchanged += 1;
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            *summary
                .skipped
                .entry(SkipReason::NonUtf8Content)
                .or_default() += 1;
            continue;
        };
        let chunks = chunker.chunk(&file.path, &text);
        let symbol_ids: Vec<Option<String>> = chunks
            .iter()
            .map(|c| c.symbol.as_ref().map(|s| s.scip_id(name, &file.path)))
            .collect();
        let document = DocumentRecord {
            path: &file.path,
            language: Language::detect(&file.path, &text).map(Language::name),
            content_hash: hash.as_slice(),
            size_bytes: text.len() as u64,
            dirty: file.dirty,
            chunks: chunks
                .iter()
                .zip(symbol_ids)
                .map(|(c, id)| ChunkRecord {
                    kind: c.kind.as_str(),
                    start_line: c.start_line,
                    end_line: c.end_line,
                    text: c.text,
                    title: c.title.as_deref(),
                    symbol: c.symbol.as_ref().zip(id).map(|(s, id)| SymbolRecord {
                        id,
                        display_name: &s.name,
                        kind: s.kind.as_str(),
                    }),
                })
                .collect(),
        };
        summary.chunks += store.write_document(source_id, &document)?;
        summary.written += 1;
        kept.insert(file.path.as_str());
    }

    for path in stored.keys().filter(|p| !kept.contains(p.as_str())) {
        store.delete_document(source_id, path)?;
        summary.removed += 1;
    }
    Ok(())
}

/// Reads a walked file, or `None` when it was deleted after the walk.
fn read(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    match std::fs::read(root.join(path)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::ReadFile {
            path: root.join(path),
            source,
        }),
    }
}

impl fmt::Display for IndexSummary {
    /// The summary `codecite index` prints, one fact per line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "documents: {} written, {} unchanged, {} removed",
            self.written, self.unchanged, self.removed
        )?;
        writeln!(f, "chunks: {} written", self.chunks)?;
        if !self.skipped.is_empty() {
            let parts: Vec<String> = self
                .skipped
                .iter()
                .map(|(reason, count)| format!("{count} {}", reason.label()))
                .collect();
            writeln!(f, "skipped: {}", parts.join(", "))?;
        }
        for (name, reason) in &self.skipped_sources {
            writeln!(f, "source {name} not indexed: {reason}")?;
        }
        for name in &self.removed_sources {
            writeln!(f, "source {name} removed: the run no longer indexes it")?;
        }
        write!(f, "elapsed: {:.2}s", self.elapsed.as_secs_f64())
    }
}
```

> Learn: `Sha256::digest(&bytes)` hashes in one call and returns a fixed-size array; `hash.as_slice()` and `old[..] == hash[..]` compare it with the stored `Vec<u8>` as byte slices. `let Ok(text) = String::from_utf8(bytes) else { ... continue; };` takes ownership of the bytes without copying when they are valid UTF-8. `chunker.chunk` returns chunks that borrow `text`, and `ChunkRecord` borrows from those chunks, so the whole `DocumentRecord` is built and written before `text` is dropped at the end of the loop body; the borrow checker enforces that order. `Option::zip` pairs a symbol with its ID only when both exist. `impl fmt::Display` lets the binary `println!("{summary}")` and the test compare `summary.to_string()`.

Update the crate docs in `lib.rs`: end the first paragraph with "[`index_corpus`] runs those steps over a corpus's local sources and writes the result into its store."

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p codecite-index`
Expected: all pass, the walk tests included.

- [ ] **Step 6: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`

```bash
git add Cargo.toml Cargo.lock crates/codecite-index
git commit -m "feat(index): index a corpus's local sources incrementally (#21)"
```

---

### Task 3: The `codecite index` command

**Files:**
- Modify: `crates/codecite/src/main.rs`
- Create: `crates/codecite/tests/index.rs`
- Modify: `crates/codecite/Cargo.toml`, `Cargo.toml`, `Cargo.lock`, `deny.toml`
- Modify: `docs/design.md`, `README.md`

- [ ] **Step 1: Dependencies and the license exception**

In the root `Cargo.toml` `[workspace.dependencies]`, after `anyhow`:

```toml
directories = "6"
```

In `crates/codecite/Cargo.toml`: `directories.workspace = true` under `[dependencies]` (after `clap`), and a new section:

```toml
[dev-dependencies]
tempfile.workspace = true
```

In `deny.toml`, under `[licenses]`, after `unused-allowed-license = "allow"`:

```toml
exceptions = [{ crate = "option-ext", allow = ["MPL-2.0"] }]
```

`option-ext` (via `directories` → `dirs-sys`) is MPL-2.0, a file-level copyleft; using it unmodified as a dependency puts no obligation on codecite's own code. The exception names the one crate, so no other MPL-2.0 crate gets in unreviewed.

Run: `cargo deny check`
Expected: `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 2: Write the failing tests**

Create `crates/codecite/tests/index.rs`:

```rust
//! `codecite index` end to end: a real git checkout, a manifest, and the
//! corpus file under a temporary `XDG_DATA_HOME`. Linux only, since other
//! platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=codecite",
            "-c",
            "user.email=codecite@example.com",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// Runs the binary in `cwd` with its data directory under `home`.
fn codecite(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn index_writes_the_corpus_file_and_reindexes_incrementally() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("app");
    fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("src/lib.rs"), "pub fn validate_token() {}\n").unwrap();
    fs::write(repo.join("README.md"), "# App\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    let work = tmp.path().join("work");
    fs::create_dir_all(work.join("corpora")).unwrap();
    fs::write(
        work.join("corpora/demo.toml"),
        format!(
            "name = \"demo\"\n\n[[source]]\nkind = \"local\"\npath = {:?}\nuse = \"adapt\"\n",
            repo.to_str().unwrap()
        ),
    )
    .unwrap();

    let first = stdout(&codecite(&work, tmp.path(), &["index", "demo"]));
    assert!(
        first.contains("documents: 2 written, 0 unchanged, 0 removed"),
        "{first}"
    );
    assert!(tmp.path().join("data/codecite/demo.db").is_file());
    assert!(!work.join("corpora/demo.db").exists());

    fs::write(repo.join("README.md"), "# App, changed\n").unwrap();
    let second = stdout(&codecite(&work, tmp.path(), &["index", "demo"]));
    assert!(
        second.contains("documents: 1 written, 1 unchanged, 0 removed"),
        "{second}"
    );
}

#[test]
fn index_errors_name_the_problem() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = codecite(tmp.path(), tmp.path(), &["index", "nope"]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        stderr.contains("reading manifest corpora/nope.toml"),
        "{stderr}"
    );

    let escape = codecite(tmp.path(), tmp.path(), &["index", "../etc"]);
    assert!(!escape.status.success());
    assert!(String::from_utf8_lossy(&escape.stderr).contains("invalid corpus name"));

    fs::write(tmp.path().join("other.toml"), "name = \"other\"\n").unwrap();
    let mismatch = codecite(
        tmp.path(),
        tmp.path(),
        &["index", "demo", "--manifest", "other.toml"],
    );
    assert!(!mismatch.status.success());
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("is for corpus `other`"));
}
```

Add to the `tests` module in `crates/codecite/src/main.rs` (it gains `use super::*;`, and its `use anyhow::Context as _;` goes, since `super::*` brings it):

```rust
    #[test]
    fn corpus_names_are_plain_file_names() {
        for good in ["identity", "oidc-impls", "rfc_9449", "v1.2"] {
            assert!(check_corpus_name(good).is_ok(), "{good}");
        }
        for bad in ["", ".hidden", "..", "../x", "a/b", "a\\b", "名前", "a b"] {
            assert!(check_corpus_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn index_requires_a_corpus() {
        assert!(Cli::try_parse_from(["codecite", "index"]).is_err());
        let cli = Cli::try_parse_from(["codecite", "index", "identity", "--manifest", "m.toml"])
            .unwrap();
        let Command::Index { corpus, manifest } = cli.command;
        assert_eq!(corpus, "identity");
        assert_eq!(manifest, Some(PathBuf::from("m.toml")));
    }
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p codecite`
Expected: compile errors in `main.rs` (`check_corpus_name`, `Command` not defined); once those exist, `tests/index.rs` fails because `index` is not a subcommand.

- [ ] **Step 4: Implement**

Replace `crates/codecite/src/main.rs` with:

```rust
//! The `codecite` command-line tool.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand};
use codecite_core::Store;
use codecite_index::{Manifest, WalkOptions, index_corpus};

/// Command-line arguments.
#[derive(Parser)]
#[command(version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The subcommands. Later milestones add more here.
#[derive(Subcommand)]
enum Command {
    /// Index a corpus's sources into its corpus file.
    Index {
        /// The corpus to index, as named in its manifest.
        corpus: String,
        /// The manifest to read [default: corpora/<CORPUS>.toml].
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Index { corpus, manifest } => index(&corpus, manifest),
    }
}

/// `codecite index`: reads the manifest, indexes it into the corpus file in
/// the user's data directory, and prints a summary.
fn index(corpus: &str, manifest: Option<PathBuf>) -> anyhow::Result<()> {
    check_corpus_name(corpus)?;
    let manifest_path =
        manifest.unwrap_or_else(|| Path::new("corpora").join(format!("{corpus}.toml")));
    let manifest = Manifest::load(&manifest_path)
        .with_context(|| format!("reading manifest {}", manifest_path.display()))?;
    if manifest.name != corpus {
        bail!(
            "manifest {} is for corpus `{}`, not `{corpus}`",
            manifest_path.display(),
            manifest.name
        );
    }

    let path = corpus_path(corpus)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut store = Store::open_read_write(&path)
        .with_context(|| format!("opening corpus file {}", path.display()))?;
    let summary = index_corpus(&mut store, &manifest, &WalkOptions::default())
        .with_context(|| format!("indexing corpus `{corpus}`"))?;
    println!("indexed corpus `{corpus}` into {}", path.display());
    println!("{summary}");
    Ok(())
}

/// A corpus name becomes a file name, so it may hold only ASCII letters,
/// digits, `-`, `_` and `.`, and may not start with `.`.
fn check_corpus_name(corpus: &str) -> anyhow::Result<()> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if corpus.is_empty() || corpus.starts_with('.') || !corpus.chars().all(allowed) {
        bail!(
            "invalid corpus name {corpus:?}: use ASCII letters, digits, `-`, `_` and `.`, not starting with `.`"
        );
    }
    Ok(())
}

/// Where the corpus file lives: `<data dir>/codecite/<corpus>.db`, e.g.
/// `~/.local/share/codecite/identity.db` on Linux. Never in the repository.
fn corpus_path(corpus: &str) -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "codecite")
        .context("no home directory to keep corpus files in")?;
    Ok(dirs.data_dir().join(format!("{corpus}.db")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> anyhow::Result<()> {
        let err = codecite_core::Error::from(std::io::Error::other("disk on fire"));
        Err(err).context("opening corpus /tmp/corpus")
    }

    #[test]
    fn context_chain_prints_each_cause_once() {
        let err = open().unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "opening corpus /tmp/corpus: I/O error: disk on fire"
        );
        assert!(err.downcast_ref::<codecite_core::Error>().is_some());
    }

    #[test]
    fn corpus_names_are_plain_file_names() {
        for good in ["identity", "oidc-impls", "rfc_9449", "v1.2"] {
            assert!(check_corpus_name(good).is_ok(), "{good}");
        }
        for bad in ["", ".hidden", "..", "../x", "a/b", "a\\b", "名前", "a b"] {
            assert!(check_corpus_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn index_requires_a_corpus() {
        assert!(Cli::try_parse_from(["codecite", "index"]).is_err());
        let cli =
            Cli::try_parse_from(["codecite", "index", "identity", "--manifest", "m.toml"]).unwrap();
        let Command::Index { corpus, manifest } = cli.command;
        assert_eq!(corpus, "identity");
        assert_eq!(manifest, Some(PathBuf::from("m.toml")));
    }
}
```

> Learn: `#[derive(Subcommand)]` on an enum makes each variant a subcommand and its fields the arguments; `#[command(subcommand)]` on the `Cli` field wires it in, and the doc comments become `--help` text. `let Command::Index { corpus, manifest } = cli.command;` is an irrefutable pattern while the enum has one variant; the `match` in `main` is what grows with each new command. In the binary, errors are `anyhow::Error`: `.with_context(|| format!(...))` wraps any error with what was being done, and `{err:#}` (which `main` returning `Err` uses) prints the chain, e.g. `indexing corpus `demo`: two sources are named `app`; ...`. `bail!` returns early with a formatted error. Libraries keep typed `thiserror` enums so callers can match on them; only the binary turns them into context chains.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p codecite`
Expected: all pass, `tests/cli.rs` included.

- [ ] **Step 6: Try it on a real checkout**

```bash
cargo build --release -p codecite
mkdir -p /var/tmp/idx-run/corpora
printf 'name = "self"\n[[source]]\nkind = "local"\npath = "%s"\nuse = "adapt"\n' "$PWD" > /var/tmp/idx-run/corpora/self.toml
(cd /var/tmp/idx-run && XDG_DATA_HOME=/var/tmp/idx-run/data "$OLDPWD/target/release/codecite" index self)
(cd /var/tmp/idx-run && XDG_DATA_HOME=/var/tmp/idx-run/data "$OLDPWD/target/release/codecite" index self)
```

Expected: the first run writes every document, the second reports them all unchanged with 0 chunks written. Then an adversarial input: `codecite index ../x` fails with `invalid corpus name`.

- [ ] **Step 7: Update the docs**

In `docs/design.md`:

- Data model: add `dirty INTEGER NOT NULL DEFAULT 0, -- a local file that differed from HEAD` to `document` (and mark `content_hash` as the sha256 of the bytes), and the three new indexes after `symbol_name`:

```sql
CREATE INDEX chunk_document ON chunk(document_id);                  -- re-indexing replaces a document's rows
CREATE INDEX symbol_document ON symbol(document_id);
CREATE INDEX edge_evidence ON edge(evidence_doc);
```

- Data model, the "One file per corpus" bullet: append "The file lives in the user's data directory, never in a repository: `<data dir>/codecite/<corpus>.db`, which is `~/.local/share/codecite/<corpus>.db` on Linux. A corpus name is ASCII letters, digits, `-`, `_` and `.`, not starting with `.`, and must match the manifest's `name`."
- Indexing pipeline step 5, replace with: "**Write:** chunks, full-text rows and symbols for one document in a single transaction. A document whose sha256 matches the stored one is not rewritten; only its `dirty` flag is updated. A stored document whose file is gone, or is now skipped, is deleted with everything stored for it, and so is every source the run did not index (one removed from the manifest, or marked `excluded`). A chunk's `terms` and its full-text `terms` column hold the identifier terms of every word in its text; the `symbol` column holds its symbol name's terms and the `path` column its path's. A local source without a `name` is named after its checkout directory, and two sources with one name are an error. The run ends with a summary: documents written, unchanged and removed, chunks written, skipped files by reason, and the elapsed time."
- Interfaces table: `codecite index <corpus> [--manifest <path>]` — "Fetch, index and run providers; incremental by default. Reads `corpora/<corpus>.toml` unless `--manifest` names another file".

In `README.md`, replace the Status line with: "Pre-alpha. `codecite index <corpus>` indexes the local git checkouts a corpus manifest (`corpora/<corpus>.toml`) lists into `~/.local/share/codecite/<corpus>.db` on Linux; searching it comes next."

- [ ] **Step 8: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`

```bash
git add Cargo.toml Cargo.lock deny.toml crates/codecite docs/design.md README.md
git commit -m "feat: index local sources with codecite index (#21)"
```
