# Index Spec Documents with Their Pinned Versions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #25: `codecite index` writes `document` sources. Each fetched document (#22) is split into sections and steps (#23, #24) and stored as its own source row per pinned version, with `is_current`, `obsoleted_by` and `updated_by` from the manifest and `license_class` from its `use`. Its section and step text goes into the full-text index like code chunks. Re-indexing an unchanged pin writes nothing.

**Architecture:** One migration (`0003_document_versions.sql`) changes two things. `source.name` stops being unique on its own: a document version is unique by `(name, revision)`, any other source by `name`, through two partial unique indexes that `Store::upsert_source` names as its conflict target. Every foreign key also gets `ON DELETE CASCADE`, which SQLite can only add by rebuilding the tables. A trigger deletes a chunk's contentless FTS row, and new indexes on `chunk(parent_id)` and `chunk(symbol_id)` keep every cascade off full scans. `Store` turns `foreign_keys` off while it migrates and on afterwards. It checks `pragma_foreign_key_check` before committing a migration. Deleting a source or document is then one `DELETE`. In `codecite-index`, `index_corpus` keeps the bytes it fetched, indexes the local sources as before, and then calls `index_document` for each document. That upserts the version's source row. If the version isn't stored yet, it runs `split_spec` and writes a `section` chunk per section, followed by a `step` chunk per step whose parent is that section. `delete_sources_except` now takes the IDs the run kept, so an unpinned version is deleted and reported as `rfc9449@sha256:<12 hex>`.

**Tech Stack:** Rust 1.99.1 (edition 2024), rusqlite 0.40 (bundled SQLite), no new dependencies; `cargo deny check` is unchanged. Prototyped: every gate passes (223 passed, 6 ignored), and the real CLI indexes the seven real spec pages (output in Task 2, Step 6).

**Spec:** Issue #25; `docs/design.md#data-model` and `docs/design.md#indexing-and-retrieval` (step 5), which Task 3 updates; `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` (Versions). The decisions that the issue and design.md leave open are recorded in the loop's DEC-015 and restated in the Global Constraints.

## Global Constraints

- A document version is a `source` row: `kind = "document"`, `name` from the manifest (shared by every version), `revision` set to the pinned sha256, and `is_current`, `obsoleted_by` and `updated_by` from the manifest. `license_spdx` and `license_class` come from `license` and `use`. The relations stored are the manifest's alone: the page's own header is not merged in, so an unchanged pin is never parsed again.
- A document source holds one `document`, stored with `path` set to the source's name, `language` NULL, `content_hash` the sha256 of the bytes and `dirty` false. Its chunks come in section order, each section followed by its steps. A section chunk has `kind = "section"`, `number` (`3.1.3.7`), `anchor`, `title`, `parent_id` set to the enclosing section's chunk, and `strength`. A step chunk has `kind = "step"`, `number` (`13`), its section's chunk as `parent_id`, and `strength`. `strength` is the keywords joined by `,` (`MUST,SHOULD`), or NULL when there are none. Line ranges count lines of `SpecDocument::text`, the plain-text rendering.
- A version already stored is unchanged by definition, since its revision is its content's hash. It is counted in `unchanged` and not split again; only its source row is updated.
- Errors: a page that isn't UTF-8 (`Error::DocumentNotUtf8`) or that `split_spec` can't read (`Error::Spec`, with the `SpecError` as its source) stops the run and names the document. A local source with a document's name is `Error::DuplicateSource`. A manifest pinning one `(name, sha256)` twice is `ManifestError::DuplicateVersion`.
- Every source the run did not index is deleted with everything stored for it, including a document version that is no longer pinned. Excluded documents are still never fetched, and they are reported in `skipped_sources`.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item and field gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`. Before the PR, also run `/var/tmp/specidx-proto/real-run.sh <worktree>` (Task 2, Step 6) and put its output in the PR body.

## Review Focus

- The migration (`version_3_keeps_the_rows_and_cascades_deletes`): a version 2 file's rows survive the rebuild with their IDs. Deleting the source then empties `document`, `symbol` and `chunk`, and the trigger empties `chunk_fts`. `a_migration_that_leaves_orphans_is_refused_unchanged` checks that `foreign_key_check` refuses a file whose rows point nowhere, leaving it byte-for-byte unchanged. `foreign_keys_are_enforced` checks that the keys are on after opening.
- Versions in SQL (`each_document_version_is_its_own_source`): two versions of `rfc9449` are two rows, the same version upserts into its own row, and a local source keeps its row when its revision moves. That last case is why uniqueness is split by kind and not `UNIQUE (name, revision)` for every kind.
- Every cascade is indexed: `document_lookups_use_an_index` adds `chunk_parent`, `chunk_symbol` and the `document(source_id, path)` autoindex to the query plans it checks. Without an index, each deleted row would scan the child table, which is the 53 s versus 8 s lesson from #21.
- Acceptance (issue #25): `two_versions_of_one_document_are_both_indexed` indexes two versions, one current, and the old one's `obsoleted_by`, then shows unpinning the old version deletes it and its text. `reindexing_an_unchanged_pin_is_a_no_op` asserts 0 written, 1 unchanged and the same chunk IDs. `a_document_is_indexed_as_sections_and_steps` checks every chunk's kind, number, anchor, parent and strength, plus the full-text matches.

---

### Task 1: Store document versions, with cascading deletes (`codecite-core`)

**Files:**
- Create: `crates/codecite-core/migrations/0003_document_versions.sql`
- Modify: `crates/codecite-core/src/store.rs`, `crates/codecite-core/src/store/write.rs`
- Modify: `crates/codecite-index/src/index.rs` (call sites only, so the workspace keeps compiling)

**Interfaces:**
- Produces (for Task 2, #27 and #34): `SourceRecord` gains `is_current: bool`, `obsoleted_by: Option<&str>` and `updated_by: Option<&str>`. `ChunkRecord` gains `number: Option<&str>`, `anchor: Option<&str>`, `parent: Option<usize>` (the index of an earlier chunk in `DocumentRecord::chunks`) and `strength: &[Keyword]`. `Store::delete_sources_except(&mut self, keep: &[i64]) -> Result<Vec<String>>` now takes source IDs and returns the deleted sources' labels, a version labelled `name@sha256:<12 hex>`. `SCHEMA_VERSION` becomes 3.

- [ ] **Step 1: Write the failing tests**

Add these test hunks to `store.rs` and `store/write.rs`:

```diff
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -523,6 +543,112 @@ mod tests {
         fs::remove_dir_all(&dir).unwrap();
     }
 
+    #[test]
+    fn version_3_keeps_the_rows_and_cascades_deletes() {
+        let dir = scratch("v2");
+        let path = dir.join("corpus.db");
+        // A version 2 corpus with one source, document, symbol and chunk.
+        raw(&path)
+            .execute_batch(&format!(
+                "{}
+                 {}
+                 INSERT INTO meta (key, value) VALUES ('schema_version', '2');
+                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
+                   VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then');
+                 INSERT INTO document (id, source_id, path, content_hash, dirty)
+                   VALUES (1, 1, 'src/lib.rs', x'00', 1);
+                 INSERT INTO symbol (id, display_name, document_id) VALUES ('s', 'validate', 1);
+                 INSERT INTO chunk (id, document_id, start_line, end_line, kind, symbol_id, text, terms)
+                   VALUES (7, 1, 1, 1, 'function', 's', 'fn validate() {{}}', 'validate');
+                 INSERT INTO chunk_fts (rowid, symbol, path, terms) VALUES (7, 'validate', 'lib', 'validate');",
+                MIGRATIONS[0], MIGRATIONS[1]
+            ))
+            .unwrap();
+
+        let store = Store::open_read_write(&path).unwrap();
+        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
+        let (chunk, dirty): (i64, bool) = store
+            .conn
+            .query_row(
+                "SELECT c.id, d.dirty FROM chunk c JOIN document d ON d.id = c.document_id
+                 JOIN symbol s ON s.id = c.symbol_id JOIN source ON source.id = d.source_id",
+                [],
+                |row| Ok((row.get(0)?, row.get(1)?)),
+            )
+            .unwrap();
+        assert_eq!((chunk, dirty), (7, true));
+        let fts = |conn: &Connection| -> i64 {
+            conn.query_row(
+                "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH 'validate'",
+                [],
+                |row| row.get(0),
+            )
+            .unwrap()
+        };
+        assert_eq!(fts(&store.conn), 1);
+
+        store.conn.execute("DELETE FROM source", []).unwrap();
+        for table in ["document", "symbol", "chunk"] {
+            let rows: i64 = store
+                .conn
+                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
+                    row.get(0)
+                })
+                .unwrap();
+            assert_eq!(rows, 0, "{table}");
+        }
+        assert_eq!(fts(&store.conn), 0);
+        drop(store);
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
+    #[test]
+    fn a_migration_that_leaves_orphans_is_refused_unchanged() {
+        let dir = scratch("orphans");
+        let path = dir.join("corpus.db");
+        raw(&path)
+            .execute_batch(&format!(
+                "PRAGMA foreign_keys = OFF;
+                 {}
+                 {}
+                 INSERT INTO meta (key, value) VALUES ('schema_version', '2');
+                 INSERT INTO document (id, source_id, path, content_hash) VALUES (1, 99, 'a', x'00');",
+                MIGRATIONS[0], MIGRATIONS[1]
+            ))
+            .unwrap();
+        let before = fs::read(&path).unwrap();
+
+        let err = Store::open_read_write(&path).err().unwrap();
+        assert!(
+            matches!(&err, Error::InvalidCorpus { reason, .. } if reason.contains("1 rows")),
+            "{err:?}"
+        );
+        assert_eq!(fs::read(&path).unwrap(), before);
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
+    #[test]
+    fn foreign_keys_are_enforced() {
+        let dir = scratch("fk");
+        let store = Store::open_read_write(&dir.join("corpus.db")).unwrap();
+        let err = store
+            .conn
+            .execute(
+                "INSERT INTO document (source_id, path, content_hash) VALUES (99, 'a', x'00')",
+                [],
+            )
+            .unwrap_err();
+        assert!(
+            matches!(
+                &err,
+                rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ConstraintViolation
+            ),
+            "{err:?}"
+        );
+        drop(store);
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
     #[test]
     fn every_migration_file_is_registered() {
         let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
```

```diff
diff --git a/crates/codecite-core/src/store/write.rs b/crates/codecite-core/src/store/write.rs
--- a/crates/codecite-core/src/store/write.rs
+++ b/crates/codecite-core/src/store/write.rs
@@ -289,6 +315,15 @@ mod tests {
     use std::fs;
     use std::path::PathBuf;
 
+    /// A spec chunk as (kind, number, anchor, parent's number, strength).
+    type SpecRow = (
+        String,
+        String,
+        Option<String>,
+        Option<String>,
+        Option<String>,
+    );
+
     /// A fresh store in its own scratch directory.
     fn store(name: &str) -> (Store, PathBuf) {
         let dir =
@@ -306,6 +341,24 @@ mod tests {
             revision: "0123456789abcdef0123456789abcdef01234567",
             license_spdx: Some("MIT"),
             license_class: "adapt",
+            is_current: true,
+            obsoleted_by: None,
+            updated_by: None,
+        }
+    }
+
+    /// One pinned version of the document `name`.
+    fn version<'a>(name: &'a str, sha256: &'a str, is_current: bool) -> SourceRecord<'a> {
+        SourceRecord {
+            name,
+            kind: "document",
+            url: "https://www.rfc-editor.org/rfc/rfc9449.html",
+            revision: sha256,
+            license_spdx: Some("IETF Trust"),
+            license_class: "quote",
+            is_current,
+            obsoleted_by: None,
+            updated_by: None,
         }
     }
 
@@ -321,7 +374,11 @@ mod tests {
                 start_line: 1,
                 end_line: 1,
                 text,
+                number: None,
+                anchor: None,
                 title: None,
+                parent: None,
+                strength: &[],
                 symbol: Some(SymbolRecord {
                     id: format!("codecite . app . {path}/validateToken()."),
                     display_name: "validateToken",
@@ -460,11 +517,14 @@ mod tests {
                 )
                 .unwrap();
         }
-        assert_eq!(store.delete_sources_except(&["app"]).unwrap(), ["old"]);
+        let app = store.upsert_source(&source("app")).unwrap();
+        assert_eq!(store.delete_sources_except(&[app]).unwrap(), ["old"]);
         assert_eq!(count(&store, "SELECT count(*) FROM source"), 1);
+        assert_eq!(count(&store, "SELECT count(*) FROM document"), 1);
         assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 1);
+        assert_eq!(count(&store, "SELECT count(*) FROM symbol"), 1);
         assert_eq!(matches(&store, "path:old"), 0);
-        assert!(store.delete_sources_except(&["app"]).unwrap().is_empty());
+        assert!(store.delete_sources_except(&[app]).unwrap().is_empty());
         drop(store);
         fs::remove_dir_all(&dir).unwrap();
     }
@@ -505,6 +565,13 @@ mod tests {
                 "symbol_document",
             ),
             ("DELETE FROM edge WHERE evidence_doc = 1", "edge_evidence"),
+            // The lookups `ON DELETE` runs for each deleted chunk and symbol.
+            ("SELECT id FROM chunk WHERE parent_id = 1", "chunk_parent"),
+            ("SELECT id FROM chunk WHERE symbol_id = 'x'", "chunk_symbol"),
+            (
+                "SELECT id FROM document WHERE source_id = 1",
+                "sqlite_autoindex_document_1",
+            ),
         ] {
             let plan: Vec<String> = store
                 .conn
@@ -522,4 +589,142 @@ mod tests {
         drop(store);
         fs::remove_dir_all(&dir).unwrap();
     }
+
+    #[test]
+    fn each_document_version_is_its_own_source() {
+        let (mut store, dir) = store("versions");
+        let old = "1".repeat(64);
+        let new = "2".repeat(64);
+        let first = store
+            .upsert_source(&version("rfc9449", &old, false))
+            .unwrap();
+        let second = store
+            .upsert_source(&version("rfc9449", &new, true))
+            .unwrap();
+        assert_ne!(first, second);
+        // The same version again is the same row, with its fields updated.
+        let mut obsoleted = version("rfc9449", &old, false);
+        obsoleted.obsoleted_by = Some("rfc9999");
+        assert_eq!(store.upsert_source(&obsoleted).unwrap(), first);
+        let rows: Vec<(String, bool, Option<String>)> = {
+            let mut stmt = store
+                .conn
+                .prepare("SELECT revision, is_current, obsoleted_by FROM source ORDER BY id")
+                .unwrap();
+            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
+                .unwrap()
+                .collect::<rusqlite::Result<_>>()
+                .unwrap()
+        };
+        assert_eq!(
+            rows,
+            [
+                (old.clone(), false, Some("rfc9999".to_owned())),
+                (new.clone(), true, None)
+            ]
+        );
+
+        // A local source keeps its row when its revision moves.
+        let app = store.upsert_source(&source("app")).unwrap();
+        let mut moved = source("app");
+        moved.revision = "fedcba9876543210fedcba9876543210fedcba98";
+        assert_eq!(store.upsert_source(&moved).unwrap(), app);
+
+        assert_eq!(
+            store.delete_sources_except(&[second]).unwrap(),
+            ["app".to_owned(), format!("rfc9449@sha256:{}", &old[..12])]
+        );
+        drop(store);
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
+    #[test]
+    fn spec_chunks_store_their_number_parent_and_strength() {
+        let (mut store, dir) = store("spec");
+        let sha = "1".repeat(64);
+        let id = store
+            .upsert_source(&version("rfc9449", &sha, true))
+            .unwrap();
+        let chunk = |kind, number, parent, text, strength| ChunkRecord {
+            kind,
+            start_line: 1,
+            end_line: 1,
+            text,
+            number: Some(number),
+            anchor: (kind == "section").then_some("section-4.3"),
+            title: (kind == "section").then_some("Checking DPoP Proofs"),
+            parent,
+            strength,
+            symbol: None,
+        };
+        let doc = DocumentRecord {
+            path: "rfc9449",
+            language: None,
+            content_hash: sha.as_bytes(),
+            size_bytes: 10,
+            dirty: false,
+            chunks: vec![
+                chunk(
+                    "section",
+                    "4.3",
+                    None,
+                    "4.3. Checking DPoP Proofs",
+                    &[Keyword::Must, Keyword::May],
+                ),
+                chunk("step", "1", Some(0), "1. One DPoP header.", &[]),
+            ],
+        };
+        assert_eq!(store.write_document(id, &doc).unwrap(), 2);
+
+        let rows: Vec<SpecRow> = {
+            let mut stmt = store
+                .conn
+                .prepare(
+                    "SELECT c.kind, c.number, c.anchor, p.number, c.strength
+                     FROM chunk c LEFT JOIN chunk p ON p.id = c.parent_id ORDER BY c.id",
+                )
+                .unwrap();
+            stmt.query_map([], |row| {
+                Ok((
+                    row.get(0)?,
+                    row.get(1)?,
+                    row.get(2)?,
+                    row.get(3)?,
+                    row.get(4)?,
+                ))
+            })
+            .unwrap()
+            .collect::<rusqlite::Result<_>>()
+            .unwrap()
+        };
+        assert_eq!(
+            rows,
+            [
+                (
+                    "section".to_owned(),
+                    "4.3".to_owned(),
+                    Some("section-4.3".to_owned()),
+                    None,
+                    Some("MUST,MAY".to_owned())
+                ),
+                (
+                    "step".to_owned(),
+                    "1".to_owned(),
+                    None,
+                    Some("4.3".to_owned()),
+                    None
+                ),
+            ]
+        );
+        assert_eq!(matches(&store, "dpop"), 2);
+
+        // Deleting the source takes the document, both chunks and their
+        // full-text rows with it.
+        assert_eq!(store.delete_sources_except(&[]).unwrap().len(), 1);
+        assert_eq!(count(&store, "SELECT count(*) FROM document"), 0);
+        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 0);
+        assert_eq!(matches(&store, "dpop"), 0);
+        drop(store);
+        fs::remove_dir_all(&dir).unwrap();
+    }
 }
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite-core`
Expected: compile errors, among them ``struct `write::SourceRecord<'_>` has no field named `is_current` `` and the same for `obsoleted_by`, `updated_by` and `ChunkRecord`'s `strength`.

- [ ] **Step 3: Add the migration**

Create `crates/codecite-core/migrations/0003_document_versions.sql`:

```sql
-- Several pinned versions of one document are separate sources: a document's name is unique per revision, any other source's name on its own.
-- Deleting a source or a document deletes everything stored for it: SQLite can't add ON DELETE CASCADE to a column, so these tables are rebuilt.
-- The store turns foreign keys on only after migrating, so the copies below are not checked against tables not yet rebuilt.

CREATE TABLE new_source (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL,
  kind TEXT NOT NULL,
  url TEXT NOT NULL,
  revision TEXT NOT NULL,
  is_current INTEGER NOT NULL DEFAULT 1,
  obsoleted_by TEXT, updated_by TEXT,
  license_spdx TEXT, license_class TEXT NOT NULL,
  indexed_at TEXT NOT NULL
);
INSERT INTO new_source SELECT id, name, kind, url, revision, is_current, obsoleted_by, updated_by, license_spdx, license_class, indexed_at FROM source;
DROP TABLE source;
ALTER TABLE new_source RENAME TO source;
CREATE UNIQUE INDEX source_name ON source(name) WHERE kind <> 'document';
CREATE UNIQUE INDEX source_version ON source(name, revision) WHERE kind = 'document';

CREATE TABLE new_document (
  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  path TEXT NOT NULL, language TEXT, content_hash BLOB NOT NULL, size_bytes INTEGER,
  dirty INTEGER NOT NULL DEFAULT 0,
  UNIQUE (source_id, path)
);
INSERT INTO new_document SELECT id, source_id, path, language, content_hash, size_bytes, dirty FROM document;
DROP TABLE document;
ALTER TABLE new_document RENAME TO document;

CREATE TABLE new_symbol (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL, kind TEXT,
  document_id INTEGER REFERENCES document(id) ON DELETE CASCADE, start_line INTEGER, end_line INTEGER
);
INSERT INTO new_symbol SELECT id, display_name, kind, document_id, start_line, end_line FROM symbol;
DROP TABLE symbol;
ALTER TABLE new_symbol RENAME TO symbol;
CREATE INDEX symbol_name ON symbol(display_name);
CREATE INDEX symbol_document ON symbol(document_id);

CREATE TABLE new_chunk (
  id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES document(id) ON DELETE CASCADE,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  kind TEXT NOT NULL,
  symbol_id TEXT REFERENCES symbol(id) ON DELETE SET NULL,
  number TEXT, anchor TEXT, title TEXT,
  parent_id INTEGER REFERENCES chunk(id) ON DELETE CASCADE,
  strength TEXT,
  text TEXT NOT NULL,
  terms TEXT NOT NULL
);
INSERT INTO new_chunk SELECT id, document_id, start_line, end_line, kind, symbol_id, number, anchor, title, parent_id, strength, text, terms FROM chunk;
DROP TABLE chunk;
ALTER TABLE new_chunk RENAME TO chunk;
-- Every foreign key is indexed, so a cascading delete looks its children up instead of scanning.
CREATE INDEX chunk_document ON chunk(document_id);
CREATE INDEX chunk_parent ON chunk(parent_id);
CREATE INDEX chunk_symbol ON chunk(symbol_id);

-- A contentless full-text table can't be a foreign key's child, so a trigger deletes a chunk's row, by rowid, with the chunk.
CREATE TRIGGER chunk_fts_delete AFTER DELETE ON chunk BEGIN
  DELETE FROM chunk_fts WHERE rowid = old.id;
END;

CREATE TABLE new_edge (
  src TEXT NOT NULL, dst TEXT NOT NULL,
  kind TEXT NOT NULL,
  provider TEXT NOT NULL,
  confidence INTEGER NOT NULL,
  evidence_doc INTEGER REFERENCES document(id) ON DELETE CASCADE, evidence_line INTEGER,
  PRIMARY KEY (src, dst, kind, provider)
);
INSERT INTO new_edge SELECT src, dst, kind, provider, confidence, evidence_doc, evidence_line FROM edge;
DROP TABLE edge;
ALTER TABLE new_edge RENAME TO edge;
CREATE INDEX edge_dst ON edge(dst, kind);
CREATE INDEX edge_evidence ON edge(evidence_doc);
```

> **Learn: modelling versions in SQL.** "Unique" can be scoped. A *partial index* (`CREATE UNIQUE INDEX … WHERE kind = 'document'`) enforces uniqueness only on the rows its `WHERE` matches. Two partial indexes give two rules in one table: documents are unique by `(name, revision)`, so every pinned version gets its own row, and everything else is unique by `name`. An upsert (`INSERT … ON CONFLICT … DO UPDATE`) has to name the index it conflicts on. For a partial index, you write its columns and its `WHERE`: `ON CONFLICT (name, revision) WHERE kind = 'document'`. That is why `upsert_source` picks its conflict clause by kind.

> **Learn: foreign keys and `ON DELETE CASCADE`.** `REFERENCES source(id)` makes `document.source_id` a foreign key. With `PRAGMA foreign_keys` on, SQLite refuses a row that points at nothing. `ON DELETE CASCADE` says what happens to the children when a parent is deleted: they are deleted too, and so are their own children, so `DELETE FROM source` takes the documents, chunks, symbols and edges with it. `ON DELETE SET NULL` keeps the child and clears the key instead. SQLite can't add these clauses to an existing column, so the migration rebuilds each table: create the new one, copy the rows with their IDs, drop the old one, rename. Every cascade looks up the children by their key, so each foreign key column needs an index. Without one, every deleted parent scans the whole child table. The FTS table can't be a child, which is why a trigger deletes its rows.

- [ ] **Step 4: Register it and turn the keys on**

```diff
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -18,6 +18,7 @@ pub use write::{ChunkRecord, DocumentRecord, SourceRecord, SymbolRecord};
 const MIGRATIONS: &[&str] = &[
     include_str!("../migrations/0001_initial.sql"),
     include_str!("../migrations/0002_incremental_index.sql"),
+    include_str!("../migrations/0003_document_versions.sql"),
 ];
 
 /// The schema version this build creates and reads.
@@ -45,6 +46,11 @@ impl Store {
         // left into the main file. Hold that off until the file is accepted,
         // so a refused one stays as it was.
         conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)?;
+        // A migration that rebuilds a table drops the old one, and with
+        // foreign keys on, dropping a table first deletes its rows, which
+        // its children's keys refuse. The bundled SQLite turns them on by
+        // default, and the pragma can't change inside a transaction.
+        conn.pragma_update(None, "foreign_keys", false)?;
 
         // `BEGIN IMMEDIATE` takes the write lock before reading the version,
         // so two indexers racing on a fresh file migrate it one after the
@@ -66,8 +72,22 @@ impl Store {
                 [version.to_string()],
             )?;
         }
+        // With the keys off, nothing checked the rebuilt tables' rows.
+        let orphans: i64 =
+            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
+                row.get(0)
+            })?;
+        if orphans > 0 {
+            return Err(Error::InvalidCorpus {
+                path: path.to_path_buf(),
+                reason: format!("{orphans} rows refer to rows that don't exist"),
+            });
+        }
         tx.commit()?;
         conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, false)?;
+        // Every write from here on checks foreign keys and runs their
+        // `ON DELETE CASCADE`.
+        conn.pragma_update(None, "foreign_keys", true)?;
 
         // Switching to WAL can't run inside a transaction, and doing it only
         // now leaves a refused file's header untouched.
```

> **Learn: the bundled SQLite's defaults.** The `bundled` feature compiles SQLite with `SQLITE_DEFAULT_FOREIGN_KEYS=1`, so foreign keys were already on, and that is why the old code deleted chunks before their document. A table rebuild has to turn them off, because `DROP TABLE` on a parent first deletes its rows and the children's keys would refuse that. `PRAGMA foreign_keys` is ignored inside a transaction, so `open_read_write` sets it before `BEGIN IMMEDIATE`. Once the migrations commit, it turns the keys back on.

- [ ] **Step 5: Write versions, spec chunk fields and cascading deletes**

```diff
diff --git a/crates/codecite-core/src/store/write.rs b/crates/codecite-core/src/store/write.rs
--- a/crates/codecite-core/src/store/write.rs
+++ b/crates/codecite-core/src/store/write.rs
@@ -6,12 +6,14 @@ use std::collections::HashMap;
 use rusqlite::{Transaction, params};
 
 use super::Store;
-use crate::{Result, identifier_terms, text_terms};
+use crate::{Keyword, Result, identifier_terms, text_terms};
 
-/// A source to create or update, matched by name.
+/// A source to create or update. A document is matched by its name and
+/// revision, since each pinned version is its own source; any other source
+/// by its name alone.
 #[derive(Debug, Clone, PartialEq, Eq)]
 pub struct SourceRecord<'a> {
-    /// The name results cite; unique in the corpus.
+    /// The name results cite. A document's versions share it.
     pub name: &'a str,
     /// `local`, `git` or `document`.
     pub kind: &'a str,
@@ -23,6 +25,13 @@ pub struct SourceRecord<'a> {
     pub license_spdx: Option<&'a str>,
     /// `adapt`, `approach_only`, `quote` or `excluded`.
     pub license_class: &'a str,
+    /// Whether this is the version of a document that queries cite by
+    /// default. Always true for a source that isn't a document.
+    pub is_current: bool,
+    /// The document that replaces this one, as the manifest names it.
+    pub obsoleted_by: Option<&'a str>,
+    /// A document that amends this one, as the manifest names it.
+    pub updated_by: Option<&'a str>,
 }
 
 /// A document and every chunk in it.
@@ -53,8 +62,18 @@ pub struct ChunkRecord<'a> {
     pub end_line: u32,
     /// The chunk's text.
     pub text: &'a str,
+    /// A spec section's number (`3.1.3.7`), or a step's (`13`).
+    pub number: Option<&'a str>,
+    /// A spec section's own anchor, without the `#`: `IDTokenValidation`.
+    pub anchor: Option<&'a str>,
     /// A section's heading.
     pub title: Option<&'a str>,
+    /// The index, in [`DocumentRecord::chunks`], of the enclosing section
+    /// of a spec section or step. It must be an earlier chunk.
+    pub parent: Option<usize>,
+    /// The requirement keywords in a spec section or step, stored as
+    /// `MUST,SHOULD`. Empty for code.
+    pub strength: &'a [Keyword],
     /// The definition the chunk holds.
     pub symbol: Option<SymbolRecord<'a>>,
 }
@@ -71,22 +90,38 @@ pub struct SymbolRecord<'a> {
 }
 
 impl Store {
-    /// Creates the source named `source.name`, or updates its other fields
-    /// and `indexed_at`, and returns its ID.
+    /// Creates the source, or updates the stored one's other fields and
+    /// `indexed_at`, and returns its ID. A document version is the stored
+    /// document with the same name and revision; any other source is the
+    /// stored one with the same name that isn't a document.
     pub fn upsert_source(&mut self, source: &SourceRecord<'_>) -> Result<i64> {
+        // An upsert names the unique index it conflicts on, and a partial
+        // index is named by its columns plus its `WHERE`.
+        let conflict = if source.kind == "document" {
+            "(name, revision) WHERE kind = 'document'"
+        } else {
+            "(name) WHERE kind <> 'document'"
+        };
         let id = self.conn.query_row(
-            "INSERT INTO source (name, kind, url, revision, license_spdx, license_class, indexed_at)
-             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
-             ON CONFLICT (name) DO UPDATE SET
-               kind = excluded.kind, url = excluded.url, revision = excluded.revision,
-               license_spdx = excluded.license_spdx, license_class = excluded.license_class,
-               indexed_at = excluded.indexed_at
-             RETURNING id",
+            &format!(
+                "INSERT INTO source (name, kind, url, revision, is_current, obsoleted_by,
+                   updated_by, license_spdx, license_class, indexed_at)
+                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
+                 ON CONFLICT {conflict} DO UPDATE SET
+                   kind = excluded.kind, url = excluded.url, revision = excluded.revision,
+                   is_current = excluded.is_current, obsoleted_by = excluded.obsoleted_by,
+                   updated_by = excluded.updated_by, license_spdx = excluded.license_spdx,
+                   license_class = excluded.license_class, indexed_at = excluded.indexed_at
+                 RETURNING id"
+            ),
             params![
                 source.name,
                 source.kind,
                 source.url,
                 source.revision,
+                source.is_current,
+                source.obsoleted_by,
+                source.updated_by,
                 source.license_spdx,
                 source.license_class,
             ],
@@ -146,38 +181,42 @@ impl Store {
         Ok(())
     }
 
-    /// Deletes the document at `path` in the source, with everything stored
-    /// for it, in one transaction.
+    /// Deletes the document at `path` in the source. Its chunks, their
+    /// full-text rows, its symbols and the edges it is evidence for go with
+    /// it, in the same statement.
     pub fn delete_document(&mut self, source_id: i64, path: &str) -> Result<()> {
-        let tx = self.conn.transaction()?;
-        let ids = document_ids(
-            &tx,
-            "source_id = ?1 AND path = ?2",
+        self.conn.execute(
+            "DELETE FROM document WHERE source_id = ?1 AND path = ?2",
             params![source_id, path],
         )?;
-        delete_documents(&tx, &ids)?;
-        tx.commit()?;
         Ok(())
     }
 
-    /// Deletes every source whose name is not in `keep`, with all its
-    /// documents, in one transaction. Returns the deleted sources' names,
-    /// sorted.
-    pub fn delete_sources_except(&mut self, keep: &[&str]) -> Result<Vec<String>> {
+    /// Deletes every source whose ID is not in `keep`, with everything
+    /// stored for it, in one transaction. Returns the deleted sources as
+    /// citations name them, sorted: `app`, or `rfc9449@sha256:1a2b3c4d5e6f`
+    /// for a document version.
+    pub fn delete_sources_except(&mut self, keep: &[i64]) -> Result<Vec<String>> {
         let tx = self.conn.transaction()?;
-        let mut names: Vec<(i64, String)> = {
-            let mut stmt = tx.prepare("SELECT id, name FROM source ORDER BY name")?;
+        let mut sources: Vec<(i64, String)> = {
+            let mut stmt = tx.prepare(
+                "SELECT id, CASE kind WHEN 'document' THEN name || '@sha256:' || substr(revision, 1, 12)
+                   ELSE name END AS label
+                 FROM source ORDER BY label",
+            )?;
             let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
             rows.collect::<rusqlite::Result<_>>()?
         };
-        names.retain(|(_, name)| !keep.contains(&name.as_str()));
-        for (id, _) in &names {
-            let ids = document_ids(&tx, "source_id = ?1", params![id])?;
-            delete_documents(&tx, &ids)?;
-            tx.execute("DELETE FROM source WHERE id = ?1", [id])?;
+        sources.retain(|(id, _)| !keep.contains(id));
+        let mut delete = tx.prepare("DELETE FROM source WHERE id = ?1")?;
+        for (id, _) in &sources {
+            // `ON DELETE CASCADE` takes the documents, and they take
+            // their chunks, symbols and edges.
+            delete.execute([id])?;
         }
+        drop(delete);
         tx.commit()?;
-        Ok(names.into_iter().map(|(_, name)| name).collect())
+        Ok(sources.into_iter().map(|(_, label)| label).collect())
     }
 
     /// Records when the corpus was last indexed, in `meta.indexed_at`.
@@ -192,37 +231,10 @@ impl Store {
     }
 }
 
-/// The IDs of the documents matching `filter`, a `WHERE` clause.
-fn document_ids(
-    tx: &Transaction<'_>,
-    filter: &str,
-    params: impl rusqlite::Params,
-) -> Result<Vec<i64>> {
-    let mut stmt = tx.prepare(&format!("SELECT id FROM document WHERE {filter}"))?;
-    let ids = stmt.query_map(params, |row| row.get(0))?;
-    Ok(ids.collect::<rusqlite::Result<_>>()?)
-}
-
-/// Deletes each document and everything stored for it.
-fn delete_documents(tx: &Transaction<'_>, ids: &[i64]) -> Result<()> {
-    let mut delete = tx.prepare_cached("DELETE FROM document WHERE id = ?1")?;
-    for &id in ids {
-        clear_document(tx, id)?;
-        delete.execute([id])?;
-    }
-    Ok(())
-}
-
 /// Deletes a document's chunks, their full-text rows, its symbols and the
 /// edges it is evidence for, keeping the document row.
 fn clear_document(tx: &Transaction<'_>, document_id: i64) -> Result<()> {
-    // A contentless table deletes by rowid, one row at a time.
-    let mut chunk_ids = tx.prepare_cached("SELECT id FROM chunk WHERE document_id = ?1")?;
-    let mut delete_fts = tx.prepare_cached("DELETE FROM chunk_fts WHERE rowid = ?1")?;
-    let ids = chunk_ids.query_map([document_id], |row| row.get::<_, i64>(0))?;
-    for id in ids {
-        delete_fts.execute([id?])?;
-    }
+    // The `chunk_fts_delete` trigger deletes each chunk's full-text row.
     for table_filter in [
         "chunk WHERE document_id = ?1",
         "symbol WHERE document_id = ?1",
@@ -245,13 +257,16 @@ fn insert_chunks(
          VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
     )?;
     let mut chunk = tx.prepare_cached(
-        "INSERT INTO chunk (document_id, start_line, end_line, kind, symbol_id, title, text, terms)
-         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
+        "INSERT INTO chunk (document_id, start_line, end_line, kind, symbol_id, number, anchor,
+           title, parent_id, strength, text, terms)
+         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
     )?;
     let mut fts = tx.prepare_cached(
         "INSERT INTO chunk_fts (rowid, symbol, path, terms) VALUES (?1, ?2, ?3, ?4)",
     )?;
     let path_terms = text_terms(document.path);
+    // Chunk IDs as they are assigned, so a later chunk can name its parent.
+    let mut ids: Vec<i64> = Vec::with_capacity(document.chunks.len());
     for c in &document.chunks {
         if let Some(s) = &c.symbol {
             symbol.execute(params![
@@ -264,16 +279,27 @@ fn insert_chunks(
             ])?;
         }
         let terms = text_terms(c.text);
+        debug_assert!(c.parent.is_none_or(|p| p < ids.len()), "parent after child");
+        let parent_id = c.parent.and_then(|p| ids.get(p).copied());
+        let strength = (!c.strength.is_empty()).then(|| {
+            let words: Vec<&str> = c.strength.iter().map(|k| k.as_str()).collect();
+            words.join(",")
+        });
         let chunk_id = chunk.insert(params![
             document_id,
             c.start_line,
             c.end_line,
             c.kind,
             c.symbol.as_ref().map(|s| &s.id),
+            c.number,
+            c.anchor,
             c.title,
+            parent_id,
+            strength,
             c.text,
             terms,
         ])?;
+        ids.push(chunk_id);
         let symbol_terms = c
             .symbol
             .as_ref()
```

> **Learn: `bool::then` and `Option::is_none_or`.** `cond.then(|| value)` is `Some(value)` when `cond` is true and `None` otherwise, which gives a NULL `strength` for a chunk with no keywords. `then_some(value)` is the eager form. `opt.is_none_or(|p| …)` is true for `None`, or when the closure is true for `Some(p)`.

- [ ] **Step 6: Update the index's call sites**

`index_local` now returns its source ID, and the run keeps a list of the IDs it indexed. Code chunks have none of the new fields:

```diff
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -73,6 +73,7 @@ pub fn index_corpus(
     }
     let mut chunker = Chunker::new()?;
     let mut indexed: Vec<String> = Vec::new();
+    let mut keep: Vec<i64> = Vec::new();
     for source in &manifest.sources {
         let local = match source {
             Source::Local(local) if local.license_class != LicenseClass::Excluded => local,
@@ -106,10 +107,16 @@ pub fn index_corpus(
         if indexed.contains(&name) {
             return Err(Error::DuplicateSource { name });
         }
-        index_local(store, &mut chunker, local, &name, &tree, &mut summary)?;
+        keep.push(index_local(
+            store,
+            &mut chunker,
+            local,
+            &name,
+            &tree,
+            &mut summary,
+        )?);
         indexed.push(name);
     }
-    let keep: Vec<&str> = indexed.iter().map(String::as_str).collect();
     summary.removed_sources = store.delete_sources_except(&keep)?;
     store.set_indexed_at()?;
     summary.elapsed = started.elapsed();
@@ -125,7 +132,7 @@ fn source_name(local: &LocalSource, tree: &LocalTree) -> String {
     })
 }
 
-/// Writes one walked local source into the store.
+/// Writes one walked local source into the store and returns its ID.
 fn index_local(
     store: &mut Store,
     chunker: &mut Chunker,
@@ -133,7 +140,7 @@ fn index_local(
     name: &str,
     tree: &LocalTree,
     summary: &mut IndexSummary,
-) -> Result<()> {
+) -> Result<i64> {
     let url = tree.root.to_string_lossy();
     let source_id = store.upsert_source(&SourceRecord {
         name,
@@ -142,6 +149,9 @@ fn index_local(
         revision: &tree.head,
         license_spdx: local.license.as_deref(),
         license_class: local.license_class.as_str(),
+        is_current: true,
+        obsoleted_by: None,
+        updated_by: None,
     })?;
     for (reason, count) in tree.skip_counts() {
         *summary.skipped.entry(reason).or_default() += count;
@@ -193,7 +203,11 @@ fn index_local(
                     start_line: c.start_line,
                     end_line: c.end_line,
                     text: c.text,
+                    number: None,
+                    anchor: None,
                     title: c.title.as_deref(),
+                    parent: None,
+                    strength: &[],
                     symbol: c.symbol.as_ref().zip(id).map(|(s, id)| SymbolRecord {
                         id,
                         display_name: &s.name,
@@ -211,7 +225,7 @@ fn index_local(
         store.delete_document(source_id, path)?;
         summary.removed += 1;
     }
-    Ok(())
+    Ok(source_id)
 }
 
 /// Reads a walked file, or `None` when it was deleted after the walk.
```

- [ ] **Step 7: Run the tests**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no warnings; all tests pass (218 passed), the existing `documents_are_fetched_once_and_not_indexed_yet` included.

- [ ] **Step 8: Commit**

```bash
git add crates/codecite-core/migrations/0003_document_versions.sql crates/codecite-core/src/store.rs crates/codecite-core/src/store/write.rs crates/codecite-index/src/index.rs
git commit -m "feat(core): store document versions, with cascading deletes (#25)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Index document sources (`codecite-index`)

**Files:**
- Modify: `crates/codecite-index/src/index.rs`, `crates/codecite-index/src/error.rs`, `crates/codecite-index/src/manifest.rs`, `crates/codecite-index/src/fetch.rs`, `crates/codecite-index/src/lib.rs`
- Modify: `crates/codecite/tests/index.rs`

**Interfaces:**
- Consumes: Task 1's `SourceRecord`, `ChunkRecord` and `delete_sources_except`; #24's `split_spec`, `SpecDocument`, `SpecSection`, `SpecStep`.
- Produces: new `Error::DocumentNotUtf8 { name }` and `Error::Spec { name, source: SpecError }`, and `ManifestError::DuplicateVersion { name, sha256 }`. `crate::fetch::hex` becomes `pub(crate)` so tests can compute a page's pin.

- [ ] **Step 1: Write the failing tests**

The index tests now serve the v3 fixture from #23 (`tests/fixtures/spec/rfc-v3.html`), and the CLI test serves the smallest v3 page:

```diff
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -277,12 +394,20 @@ impl fmt::Display for IndexSummary {
 #[cfg(test)]
 mod tests {
     use super::*;
-    use crate::DocumentSource;
     use crate::test_repo::Repo;
     use crate::test_server::Server;
     use rusqlite::Connection;
     use std::path::PathBuf;
 
+    /// A spec chunk as (kind, number, anchor, parent's number, strength).
+    type SpecRow = (
+        String,
+        String,
+        Option<String>,
+        Option<String>,
+        Option<String>,
+    );
+
     fn local(repo: &Repo, name: Option<&str>) -> Source {
         Source::Local(LocalSource {
             name: name.map(str::to_owned),
@@ -299,7 +424,7 @@ mod tests {
             name: "rfc9449".into(),
             url,
             sha256: sha256.into(),
-            license: None,
+            license: Some("IETF Trust".into()),
             license_class: LicenseClass::Quote,
             current: true,
             aliases: Vec::new(),
@@ -308,9 +433,25 @@ mod tests {
         })
     }
 
-    const SPEC: &str = "<h1>RFC 9449</h1>\n";
-    /// The sha256 of [`SPEC`].
-    const SPEC_SHA256: &str = "adcd06b613eb4e660e403fbec3d11011983dec4501489279e191b728e2fe6b2b";
+    /// An RFC page with sections 1, 4, 4.3 (two steps), 4.3.1, A and A.1.
+    const SPEC: &str = include_str!("../tests/fixtures/spec/rfc-v3.html");
+
+    fn sha256(text: &str) -> String {
+        crate::fetch::hex(&Sha256::digest(text))
+    }
+
+    /// An older version of [`SPEC`], served at `/old.html`, and the
+    /// manifest source pinning it, not current.
+    fn old_version(server: &Server) -> Source {
+        let old = SPEC.replace("is a JWT", "is a signed JWT");
+        server.set("/old.html", old.as_str());
+        let mut source = document(server.url("/old.html"), &sha256(&old));
+        if let Source::Document(d) = &mut source {
+            d.current = false;
+            d.obsoleted_by = Some("rfc9449bis".into());
+        }
+        source
+    }
 
     fn manifest(sources: Vec<Source>) -> Manifest {
         Manifest {
@@ -562,26 +703,201 @@ mod tests {
     }
 
     #[test]
-    fn documents_are_fetched_once_and_not_indexed_yet() {
+    fn a_document_is_indexed_as_sections_and_steps() {
         let server = Server::start();
         server.set("/rfc9449.html", SPEC);
         let corpus = Corpus::new();
-        let manifest = manifest(vec![document(server.url("/rfc9449.html"), SPEC_SHA256)]);
+        let manifest = manifest(vec![document(server.url("/rfc9449.html"), &sha256(SPEC))]);
 
-        let first = corpus.index(&manifest);
-        assert_eq!((first.downloaded, first.cached), (1, 0));
+        let summary = corpus.index(&manifest);
+        assert_eq!((summary.downloaded, summary.written), (1, 1));
+        // Six sections, and two steps in §4.3.
+        assert_eq!(summary.chunks, 8);
+        assert!(summary.skipped_sources.is_empty());
+        let source: (String, String, String, bool, String, Option<String>) = corpus
+            .sql()
+            .query_row(
+                "SELECT name, kind, revision, is_current, license_class, license_spdx FROM source",
+                [],
+                |row| {
+                    Ok((
+                        row.get(0)?,
+                        row.get(1)?,
+                        row.get(2)?,
+                        row.get(3)?,
+                        row.get(4)?,
+                        row.get(5)?,
+                    ))
+                },
+            )
+            .unwrap();
         assert_eq!(
-            first.skipped_sources,
-            [(
+            source,
+            (
                 "rfc9449".to_owned(),
-                "document sources are fetched and checked, not indexed yet"
-            )]
+                "document".to_owned(),
+                sha256(SPEC),
+                true,
+                "quote".to_owned(),
+                Some("IETF Trust".to_owned())
+            )
         );
+
+        let chunks: Vec<SpecRow> = {
+            let conn = corpus.sql();
+            let mut stmt = conn
+                .prepare(
+                    "SELECT c.kind, c.number, c.anchor, p.number, c.strength
+                     FROM chunk c LEFT JOIN chunk p ON p.id = c.parent_id ORDER BY c.id",
+                )
+                .unwrap();
+            stmt.query_map([], |row| {
+                Ok((
+                    row.get(0)?,
+                    row.get(1)?,
+                    row.get(2)?,
+                    row.get(3)?,
+                    row.get(4)?,
+                ))
+            })
+            .unwrap()
+            .collect::<rusqlite::Result<_>>()
+            .unwrap()
+        };
+        let row = |kind: &str,
+                   number: &str,
+                   anchor: Option<&str>,
+                   parent: Option<&str>,
+                   strength: Option<&str>| {
+            (
+                kind.to_owned(),
+                number.to_owned(),
+                anchor.map(str::to_owned),
+                parent.map(str::to_owned),
+                strength.map(str::to_owned),
+            )
+        };
+        assert_eq!(
+            chunks,
+            [
+                row("section", "1", Some("section-1"), None, None),
+                row("section", "4", Some("section-4"), None, None),
+                row(
+                    "section",
+                    "4.3",
+                    Some("section-4.3"),
+                    Some("4"),
+                    Some("MUST")
+                ),
+                row("step", "1", None, Some("4.3"), None),
+                row("step", "2", None, Some("4.3"), None),
+                row(
+                    "section",
+                    "4.3.1",
+                    Some("section-4.3.1"),
+                    Some("4.3"),
+                    Some("MAY")
+                ),
+                row("section", "A", Some("appendix-A"), None, None),
+                row("section", "A.1", Some("appendix-A.1"), Some("A"), None),
+            ]
+        );
+        // Spec text is searchable like code: §4.3 and its second step.
+        assert_eq!(corpus.matches("jose"), 2);
+        assert_eq!(corpus.matches("path:rfc9449"), 8);
+    }
+
+    #[test]
+    fn reindexing_an_unchanged_pin_is_a_no_op() {
+        let server = Server::start();
+        server.set("/rfc9449.html", SPEC);
+        let corpus = Corpus::new();
+        let manifest = manifest(vec![document(server.url("/rfc9449.html"), &sha256(SPEC))]);
+        corpus.index(&manifest);
+        let before = corpus.chunk_ids();
+
         let second = corpus.index(&manifest);
         assert_eq!((second.downloaded, second.cached), (0, 1));
+        assert_eq!((second.written, second.unchanged, second.chunks), (0, 1, 0));
+        assert!(second.removed_sources.is_empty());
+        assert_eq!(corpus.chunk_ids(), before);
         assert_eq!(server.requests(), 1);
     }
 
+    #[test]
+    fn two_versions_of_one_document_are_both_indexed() {
+        let server = Server::start();
+        server.set("/rfc9449.html", SPEC);
+        let corpus = Corpus::new();
+        let current = document(server.url("/rfc9449.html"), &sha256(SPEC));
+        let old = old_version(&server);
+        let summary = corpus.index(&manifest(vec![current.clone(), old]));
+        assert_eq!((summary.written, summary.chunks), (2, 16));
+
+        let versions: Vec<(String, bool, Option<String>)> = {
+            let conn = corpus.sql();
+            let mut stmt = conn
+                .prepare(
+                    "SELECT revision, is_current, obsoleted_by FROM source
+                     WHERE name = 'rfc9449' ORDER BY is_current",
+                )
+                .unwrap();
+            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
+                .unwrap()
+                .collect::<rusqlite::Result<_>>()
+                .unwrap()
+        };
+        let old_sha = sha256(&SPEC.replace("is a JWT", "is a signed JWT"));
+        assert_eq!(
+            versions,
+            [
+                (old_sha.clone(), false, Some("rfc9449bis".to_owned())),
+                (sha256(SPEC), true, None)
+            ]
+        );
+        assert_eq!(corpus.matches("signed"), 1);
+
+        // Unpinning the old version deletes it and everything stored for it.
+        let summary = corpus.index(&manifest(vec![current]));
+        assert_eq!(
+            summary.removed_sources,
+            [format!("rfc9449@sha256:{}", &old_sha[..12])]
+        );
+        assert_eq!(corpus.matches("signed"), 0);
+        assert_eq!(corpus.matches("jose"), 2);
+    }
+
+    #[test]
+    fn a_page_that_is_not_a_spec_is_an_error() {
+        let server = Server::start();
+        let page = "<h1>RFC 9449</h1>\n";
+        server.set("/rfc9449.html", page);
+        let corpus = Corpus::new();
+        let manifest = manifest(vec![document(server.url("/rfc9449.html"), &sha256(page))]);
+        let err = corpus.try_index(&manifest).unwrap_err();
+        assert!(
+            matches!(&err, Error::Spec { name, source: crate::SpecError::UnknownFormat } if name == "rfc9449"),
+            "{err:?}"
+        );
+    }
+
+    #[test]
+    fn a_local_source_may_not_share_a_document_name() {
+        let repo = fixture();
+        let server = Server::start();
+        server.set("/rfc9449.html", SPEC);
+        let corpus = Corpus::new();
+        let manifest = manifest(vec![
+            local(&repo, Some("rfc9449")),
+            document(server.url("/rfc9449.html"), &sha256(SPEC)),
+        ]);
+        let err = corpus.try_index(&manifest).unwrap_err();
+        assert!(
+            matches!(&err, Error::DuplicateSource { name } if name == "rfc9449"),
+            "{err:?}"
+        );
+    }
+
     #[test]
     fn a_changed_document_stops_the_run_before_any_write() {
         let repo = fixture();
@@ -590,7 +906,7 @@ mod tests {
         let corpus = Corpus::new();
         let manifest = manifest(vec![
             local(&repo, Some("app")),
-            document(server.url("/rfc9449.html"), SPEC_SHA256),
+            document(server.url("/rfc9449.html"), &sha256(SPEC)),
         ]);
 
         let err = corpus.try_index(&manifest).unwrap_err();
@@ -609,7 +925,7 @@ mod tests {
     fn excluded_documents_are_not_fetched() {
         let server = Server::start();
         let corpus = Corpus::new();
-        let mut doc = document(server.url("/rfc9449.html"), SPEC_SHA256);
+        let mut doc = document(server.url("/rfc9449.html"), &sha256(SPEC));
         if let Source::Document(d) = &mut doc {
             d.license_class = LicenseClass::Excluded;
         }
@@ -631,7 +947,7 @@ mod tests {
             downloaded: 1,
             cached: 2,
             skipped: BTreeMap::from([(SkipReason::Ignored, 3), (SkipReason::Binary, 1)]),
-            skipped_sources: vec![("rfc9449".into(), "document sources are not indexed yet")],
+            skipped_sources: vec![("client".into(), "git sources are not indexed yet")],
             removed_sources: vec!["old".into()],
             elapsed: Duration::from_millis(1250),
         };
@@ -641,7 +957,7 @@ mod tests {
              chunks: 9 written\n\
              fetched: 1 downloaded, 2 from the cache\n\
              skipped: 3 ignored, 1 binary\n\
-             source rfc9449 not indexed: document sources are not indexed yet\n\
+             source client not indexed: git sources are not indexed yet\n\
              source old removed: the run no longer indexes it\n\
              elapsed: 1.25s"
         );
```

```diff
diff --git a/crates/codecite-index/src/manifest.rs b/crates/codecite-index/src/manifest.rs
--- a/crates/codecite-index/src/manifest.rs
+++ b/crates/codecite-index/src/manifest.rs
@@ -611,4 +628,18 @@ updated_by = "openid-connect-core-1_0-errata2"
         .unwrap();
         assert_eq!(m.sources.len(), 2);
     }
+
+    #[test]
+    fn a_version_pinned_twice_is_an_error() {
+        let text = manifest(&format!(
+            "{}{}",
+            document("core", SHA256, Some(true)),
+            document("core", SHA256, Some(false)),
+        ));
+        let err = Manifest::parse(&text).unwrap_err();
+        assert_eq!(
+            err.to_string(),
+            format!("document `core`: sha256 {SHA256} is pinned twice; remove one")
+        );
+    }
 }
```

```diff
diff --git a/crates/codecite/tests/index.rs b/crates/codecite/tests/index.rs
--- a/crates/codecite/tests/index.rs
+++ b/crates/codecite/tests/index.rs
@@ -152,8 +152,12 @@ fn document_manifest(work: &Path, url: &str, sha256: &str) {
 
 #[test]
 fn documents_are_pinned_by_hash_and_cached() {
-    const SPEC: &[u8] = b"<h1>RFC 9449</h1>\n";
-    const SPEC_SHA256: &str = "adcd06b613eb4e660e403fbec3d11011983dec4501489279e191b728e2fe6b2b";
+    // The smallest xml2rfc v3 page: one numbered section.
+    const SPEC: &[u8] = b"<section id=\"section-1\"><h2><a href=\"#section-1\" \
+        class=\"section-number selfRef\">1. </a><a href=\"#name-introduction\" \
+        class=\"section-name selfRef\">Introduction</a></h2>\n\
+        <p>A client MUST send a DPoP proof.</p></section>\n";
+    const SPEC_SHA256: &str = "14fc279128304abd07e3ae5da78db59f24f3916bd17e4579545e26eeeb29aef9";
     let old_pin = "0".repeat(64);
     let tmp = tempfile::tempdir().unwrap();
     let work = tmp.path().join("work");
@@ -185,8 +189,16 @@ fn documents_are_pinned_by_hash_and_cached() {
         indexed.contains("fetched: 0 downloaded, 1 from the cache"),
         "{indexed}"
     );
+    assert!(
+        indexed.contains("documents: 1 written, 0 unchanged, 0 removed"),
+        "{indexed}"
+    );
     let pinned = stdout(&codecite(&work, tmp.path(), &["index", "specs"]));
     assert!(pinned.contains("1 from the cache"), "{pinned}");
+    assert!(
+        pinned.contains("documents: 0 written, 1 unchanged, 0 removed"),
+        "{pinned}"
+    );
     assert!(
         tmp.path()
             .join("cache/codecite/documents")
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite-index --lib`
Expected: compile errors: ``no variant named `Spec` found for enum `error::Error` ``, ``function `hex` is private`` and ``cannot find struct, variant or union type `DocumentSource` ``.

- [ ] **Step 3: Add the errors and the manifest check**

```diff
diff --git a/crates/codecite-index/src/error.rs b/crates/codecite-index/src/error.rs
--- a/crates/codecite-index/src/error.rs
+++ b/crates/codecite-index/src/error.rs
@@ -38,12 +38,30 @@ pub enum Error {
         source: std::io::Error,
     },
 
-    /// Two local sources in the manifest have the same name.
+    /// Two sources in the manifest have the same name, and they aren't
+    /// versions of one document.
     #[error("two sources are named `{name}`; give one of them a different `name`")]
     DuplicateSource {
         /// The shared name.
         name: String,
     },
+
+    /// A document source's bytes are not UTF-8, so they can't be HTML a
+    /// spec is split from.
+    #[error("document `{name}` is not UTF-8 text")]
+    DocumentNotUtf8 {
+        /// The document's name.
+        name: String,
+    },
+
+    /// A document source is not a spec page in a layout codecite reads.
+    #[error("document `{name}` can't be split into sections")]
+    Spec {
+        /// The document's name.
+        name: String,
+        /// Why splitting failed.
+        source: crate::SpecError,
+    },
 }
 
 /// `Result` with [`Error`] as the default error type.
```

```diff
diff --git a/crates/codecite-index/src/manifest.rs b/crates/codecite-index/src/manifest.rs
--- a/crates/codecite-index/src/manifest.rs
+++ b/crates/codecite-index/src/manifest.rs
@@ -1,7 +1,7 @@
 //! Corpus manifests: the committed TOML file that says what a corpus indexes,
 //! at which commit or hash, and how agents may use each source.
 
-use std::collections::BTreeMap;
+use std::collections::{BTreeMap, HashSet};
 use std::path::{Path, PathBuf};
 
 use serde::Deserialize;
@@ -199,6 +199,15 @@ pub enum ManifestError {
         /// The document's name.
         name: String,
     },
+
+    /// One version of a document is pinned twice.
+    #[error("document `{name}`: sha256 {sha256} is pinned twice; remove one")]
+    DuplicateVersion {
+        /// The document's name.
+        name: String,
+        /// The repeated pin.
+        sha256: String,
+    },
 }
 
 /// The top level before each source is parsed on its own.
@@ -227,7 +236,7 @@ impl Manifest {
             .enumerate()
             .map(|(i, table)| parse_source(i, table, home.as_deref()))
             .collect::<Result<Vec<_>, _>>()?;
-        check_current(&sources)?;
+        check_versions(&sources)?;
         Ok(Manifest {
             name: raw.name,
             max_age_days: raw.max_age_days,
@@ -285,12 +294,20 @@ pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
     s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
 }
 
-/// Checks that each document name has exactly one version with
-/// `current = true`. Names are checked in order, so the first error is stable.
-fn check_current(sources: &[Source]) -> Result<(), ManifestError> {
+/// Checks that no document version is pinned twice and that each document
+/// name has exactly one version with `current = true`. Names are checked in
+/// order, so the first error is stable.
+fn check_versions(sources: &[Source]) -> Result<(), ManifestError> {
     let mut current: BTreeMap<&str, usize> = BTreeMap::new();
+    let mut pins = HashSet::new();
     for source in sources {
         if let Source::Document(doc) = source {
+            if !pins.insert((&doc.name, &doc.sha256)) {
+                return Err(ManifestError::DuplicateVersion {
+                    name: doc.name.clone(),
+                    sha256: doc.sha256.clone(),
+                });
+            }
             *current.entry(&doc.name).or_default() += usize::from(doc.current);
         }
     }
```

```diff
diff --git a/crates/codecite-index/src/fetch.rs b/crates/codecite-index/src/fetch.rs
--- a/crates/codecite-index/src/fetch.rs
+++ b/crates/codecite-index/src/fetch.rs
@@ -268,7 +268,7 @@ impl Download {
 }
 
 /// Lowercase hex, the form the manifest pins.
-fn hex(bytes: &[u8]) -> String {
+pub(crate) fn hex(bytes: &[u8]) -> String {
     use std::fmt::Write as _;
     bytes.iter().fold(String::new(), |mut s, b| {
         let _ = write!(s, "{b:02x}");
```

- [ ] **Step 4: Index each document version**

```diff
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -1,5 +1,6 @@
 //! Indexing a corpus: walk each local source, chunk the files whose content
-//! changed and write them into the store, one transaction per document.
+//! changed, split each new document version into its sections and steps,
+//! and write them into the store, one transaction per document.
 
 use std::collections::{BTreeMap, HashSet};
 use std::fmt;
@@ -11,8 +12,8 @@ use codecite_core::{ChunkRecord, DocumentRecord, SourceRecord, Store, SymbolReco
 use sha2::{Digest, Sha256};
 
 use crate::{
-    Chunker, Error, Fetcher, Language, LicenseClass, LocalSource, LocalTree, Manifest, Result,
-    SkipReason, Source, WalkOptions, walk_local,
+    Chunker, DocumentSource, Error, Fetcher, Language, LicenseClass, LocalSource, LocalTree,
+    Manifest, Result, SkipReason, Source, SpecDocument, WalkOptions, split_spec, walk_local,
 };
 
 /// What an index run did.
@@ -40,18 +41,19 @@ pub struct IndexSummary {
     pub elapsed: Duration,
 }
 
-/// Fetches every document source in `manifest` and indexes every local
-/// source into `store`.
+/// Fetches every document source in `manifest` and indexes it and every
+/// local source into `store`.
 ///
 /// Documents are fetched first, from `fetcher`'s cache when they are there,
 /// so a document whose bytes don't match its pinned `sha256` stops the run
-/// before anything is written. Documents are not indexed yet.
+/// before anything is written. Each pinned version of a document is its own
+/// source, split into section and step chunks when it is first indexed.
 ///
 /// A file whose content hash matches the stored one is not rewritten, and a
-/// stored file that is gone, or is now skipped, is deleted. Git and document
-/// sources are reported in [`IndexSummary::skipped_sources`], as are sources
-/// marked `excluded`. Every source the run did not index is then deleted
-/// from the store.
+/// stored file that is gone, or is now skipped, is deleted. Git sources are
+/// reported in [`IndexSummary::skipped_sources`], as are sources marked
+/// `excluded`. Every source the run did not index, a document version no
+/// longer pinned included, is then deleted from the store.
 pub fn index_corpus(
     store: &mut Store,
     manifest: &Manifest,
@@ -60,15 +62,18 @@ pub fn index_corpus(
 ) -> Result<IndexSummary> {
     let started = Instant::now();
     let mut summary = IndexSummary::default();
+    let mut documents = Vec::new();
     for source in &manifest.sources {
         if let Source::Document(doc) = source
             && doc.license_class != LicenseClass::Excluded
         {
-            if fetcher.fetch_pinned(doc)?.from_cache {
+            let fetched = fetcher.fetch_pinned(doc)?;
+            if fetched.from_cache {
                 summary.cached += 1;
             } else {
                 summary.downloaded += 1;
             }
+            documents.push((doc, fetched.bytes));
         }
     }
     let mut chunker = Chunker::new()?;
@@ -93,12 +98,11 @@ pub fn index_corpus(
                 continue;
             }
             Source::Document(doc) => {
-                let reason = if doc.license_class == LicenseClass::Excluded {
-                    "use = \"excluded\""
-                } else {
-                    "document sources are fetched and checked, not indexed yet"
-                };
-                summary.skipped_sources.push((doc.name.clone(), reason));
+                if doc.license_class == LicenseClass::Excluded {
+                    summary
+                        .skipped_sources
+                        .push((doc.name.clone(), "use = \"excluded\""));
+                }
                 continue;
             }
         };
@@ -117,6 +121,15 @@ pub fn index_corpus(
         )?);
         indexed.push(name);
     }
+    for (doc, bytes) in &documents {
+        // A document's versions share its name, but no other source may.
+        if indexed.contains(&doc.name) {
+            return Err(Error::DuplicateSource {
+                name: doc.name.clone(),
+            });
+        }
+        keep.push(index_document(store, doc, bytes, &mut summary)?);
+    }
     summary.removed_sources = store.delete_sources_except(&keep)?;
     store.set_indexed_at()?;
     summary.elapsed = started.elapsed();
@@ -228,6 +241,110 @@ fn index_local(
     Ok(source_id)
 }
 
+/// Writes one pinned document version into the store and returns its
+/// source's ID.
+///
+/// The source row is updated on every run, since `current` and the
+/// relations live in the manifest. The document is split and written only
+/// when it isn't stored yet: its revision is its content's hash, so a
+/// stored version can't have changed.
+fn index_document(
+    store: &mut Store,
+    doc: &DocumentSource,
+    bytes: &[u8],
+    summary: &mut IndexSummary,
+) -> Result<i64> {
+    let source_id = store.upsert_source(&SourceRecord {
+        name: &doc.name,
+        kind: "document",
+        url: &doc.url,
+        revision: &doc.sha256,
+        license_spdx: doc.license.as_deref(),
+        license_class: doc.license_class.as_str(),
+        is_current: doc.current,
+        obsoleted_by: doc.obsoleted_by.as_deref(),
+        updated_by: doc.updated_by.as_deref(),
+    })?;
+    // A document source holds one document, stored under its name.
+    let path = doc.name.as_str();
+    let hash = Sha256::digest(bytes);
+    if store
+        .document_hashes(source_id)?
+        .get(path)
+        .is_some_and(|old| old[..] == hash[..])
+    {
+        summary.unchanged += 1;
+        return Ok(source_id);
+    }
+
+    let html = std::str::from_utf8(bytes).map_err(|_| Error::DocumentNotUtf8 {
+        name: doc.name.clone(),
+    })?;
+    let spec = split_spec(html).map_err(|source| Error::Spec {
+        name: doc.name.clone(),
+        source,
+    })?;
+    let step_numbers: Vec<Vec<String>> = spec
+        .sections
+        .iter()
+        .map(|s| s.steps.iter().map(|step| step.number.to_string()).collect())
+        .collect();
+    let document = DocumentRecord {
+        path,
+        language: None,
+        content_hash: hash.as_slice(),
+        size_bytes: bytes.len() as u64,
+        dirty: false,
+        chunks: spec_chunks(&spec, &step_numbers),
+    };
+    summary.chunks += store.write_document(source_id, &document)?;
+    summary.written += 1;
+    Ok(source_id)
+}
+
+/// Each section's chunk followed by its steps' chunks. A section names its
+/// enclosing section's chunk as its parent, and a step its section's.
+fn spec_chunks<'a>(
+    spec: &'a SpecDocument,
+    step_numbers: &'a [Vec<String>],
+) -> Vec<ChunkRecord<'a>> {
+    let mut chunks = Vec::new();
+    // The chunk index of each section, by section index. A parent comes
+    // before its subsections, so its index is always known here.
+    let mut section_chunks = Vec::with_capacity(spec.sections.len());
+    for (section, numbers) in spec.sections.iter().zip(step_numbers) {
+        let index = chunks.len();
+        section_chunks.push(index);
+        chunks.push(ChunkRecord {
+            kind: "section",
+            start_line: section.start_line,
+            end_line: section.end_line,
+            text: &section.text,
+            number: Some(&section.number),
+            anchor: Some(&section.anchor),
+            title: Some(&section.title),
+            parent: section.parent.and_then(|p| section_chunks.get(p).copied()),
+            strength: &section.strength,
+            symbol: None,
+        });
+        for (step, number) in section.steps.iter().zip(numbers) {
+            chunks.push(ChunkRecord {
+                kind: "step",
+                start_line: step.start_line,
+                end_line: step.end_line,
+                text: &step.text,
+                number: Some(number),
+                anchor: None,
+                title: None,
+                parent: Some(index),
+                strength: &step.strength,
+                symbol: None,
+            });
+        }
+    }
+    chunks
+}
+
 /// Reads a walked file, or `None` when it was deleted after the walk.
 fn read(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
     match std::fs::read(root.join(path)) {
```

```diff
diff --git a/crates/codecite-index/src/lib.rs b/crates/codecite-index/src/lib.rs
--- a/crates/codecite-index/src/lib.rs
+++ b/crates/codecite-index/src/lib.rs
@@ -6,7 +6,8 @@
 //! headings, anything else into 60-line windows. [`Fetcher`] downloads document
 //! sources and checks them against their pinned sha256, caching them by hash.
 //! [`index_corpus`] fetches a corpus's documents, runs the other steps over
-//! its local sources and writes the result into its store.
+//! its local sources, splits each document into spec sections and steps,
+//! and writes the result into its store.
 
 mod chunk;
 pub use chunk::{
```

> **Learn: borrowing into records.** `ChunkRecord<'a>` holds `&'a str`, so every string it points at must outlive it. A step's number is a `u32`, so `index_document` first builds `step_numbers: Vec<Vec<String>>`, and `spec_chunks` borrows from that and from `spec`, both of which live until `write_document` returns. `section.parent.and_then(|p| section_chunks.get(p).copied())` looks a parent up without the panic an index (`[p]`) could raise.

- [ ] **Step 5: Run the gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all green; 223 passed, 6 ignored; `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 6: Index the real pages through the CLI**

Run: `cargo build -p codecite && /var/tmp/specidx-proto/real-run.sh "$PWD"`. It serves the seven pinned pages from #24's prototype (`/var/tmp/steps-proto/docs`) on 127.0.0.1:8765 and indexes them twice. It then pins an older RFC 9449 (one word changed) and unpins it again.
Expected output, as the prototype printed it. The sections and steps match #24's counts:

```text
== first run
indexed corpus `specs` into /var/tmp/specidx-proto/real/home/data/codecite/specs.db
documents: 7 written, 0 unchanged, 0 removed
chunks: 658 written
fetched: 7 downloaded, 0 from the cache
elapsed: 0.77s
== second run
indexed corpus `specs` into /var/tmp/specidx-proto/real/home/data/codecite/specs.db
documents: 0 written, 7 unchanged, 0 removed
chunks: 0 written
fetched: 0 downloaded, 7 from the cache
elapsed: 0.17s
== older rfc9449 pinned
indexed corpus `specs` into /var/tmp/specidx-proto/real/home/data/codecite/specs.db
documents: 1 written, 7 unchanged, 0 removed
chunks: 60 written
fetched: 1 downloaded, 7 from the cache
elapsed: 0.26s
== older rfc9449 unpinned
indexed corpus `specs` into /var/tmp/specidx-proto/real/home/data/codecite/specs.db
documents: 0 written, 7 unchanged, 0 removed
chunks: 0 written
fetched: 0 downloaded, 7 from the cache
source rfc9449@sha256:24c8f9a22f2d removed: the run no longer indexes it
elapsed: 0.17s
== sections and steps per source
('openid-connect-backchannel-1_0', 1, None, 'quote', 24, 11)
('openid-connect-core-1_0', 1, None, 'quote', 182, 98)
('rfc6749', 1, 'rfc9999', 'quote', 116, 0)
('rfc8252', 1, None, 'quote', 32, 5)
('rfc9126', 1, None, 'quote', 23, 6)
('rfc9449', 1, None, 'quote', 48, 12)
('rfc9700', 1, None, 'quote', 75, 26)
== Core 3.1.3.7 step 13
[('13', '3.1.3.7', 'SHOULD', '13. If the auth_time Claim was requested, either through a s')]
== integrity [('ok',)] orphans 0
```

- [ ] **Step 7: Commit**

```bash
git add crates/codecite-index/src/index.rs crates/codecite-index/src/error.rs crates/codecite-index/src/manifest.rs crates/codecite-index/src/fetch.rs crates/codecite-index/src/lib.rs crates/codecite/tests/index.rs
git commit -m "feat(index): index spec documents with their pinned versions (#25)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Describe document versions in the design

**Files:**
- Modify: `docs/design.md`

- [ ] **Step 1: Update the data model and the write step**

```diff
diff --git a/docs/design.md b/docs/design.md
index 942b35d..6130fce 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -79,7 +79,7 @@ Each corpus is one SQLite file in WAL mode, holding sources, chunks, a full-text
 CREATE TABLE meta     (key TEXT PRIMARY KEY, value TEXT);          -- schema_version, indexed_at, manifest_hash
 
 CREATE TABLE source (
-  id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE,
+  id INTEGER PRIMARY KEY, name TEXT NOT NULL,                       -- a document's versions share its name
   kind TEXT NOT NULL,                                               -- local | git | document
   url TEXT NOT NULL,                                                -- repo URL, local path or document URL
   revision TEXT NOT NULL,                                           -- commit SHA, or sha256 of a document
@@ -88,21 +88,23 @@ CREATE TABLE source (
   license_spdx TEXT, license_class TEXT NOT NULL,                   -- adapt | approach_only | quote | excluded
   indexed_at TEXT NOT NULL
 );
+CREATE UNIQUE INDEX source_name ON source(name) WHERE kind <> 'document';
+CREATE UNIQUE INDEX source_version ON source(name, revision) WHERE kind = 'document';
 
 CREATE TABLE document (
-  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id),
+  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
   path TEXT NOT NULL, language TEXT, content_hash BLOB NOT NULL, size_bytes INTEGER,  -- content_hash: sha256 of the bytes
   dirty INTEGER NOT NULL DEFAULT 0,                                 -- a local file that differed from HEAD
   UNIQUE (source_id, path)
 );
 
 CREATE TABLE chunk (
-  id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES document(id),
+  id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES document(id) ON DELETE CASCADE,
   start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
   kind TEXT NOT NULL,                                               -- function | class | section | step | window
-  symbol_id TEXT REFERENCES symbol(id),
-  number TEXT, anchor TEXT, title TEXT,                             -- spec sections and steps: "3.1.3.7", "IDTokenValidation"
-  parent_id INTEGER REFERENCES chunk(id),                           -- enclosing section of a section or step
+  symbol_id TEXT REFERENCES symbol(id) ON DELETE SET NULL,
+  number TEXT, anchor TEXT, title TEXT,                             -- a section's "3.1.3.7", "IDTokenValidation"; a step's "13"
+  parent_id INTEGER REFERENCES chunk(id) ON DELETE CASCADE,         -- enclosing section of a section or step
   strength TEXT,                                                    -- requirement keywords found, e.g. "MUST,SHOULD"
   text TEXT NOT NULL,
   terms TEXT NOT NULL                                               -- identifiers split for search
@@ -118,7 +120,7 @@ CREATE VIRTUAL TABLE chunk_vec USING vec0(chunk_id INTEGER PRIMARY KEY, embeddin
 CREATE TABLE symbol (
   id TEXT PRIMARY KEY,                                              -- SCIP symbol string
   display_name TEXT NOT NULL, kind TEXT,
-  document_id INTEGER REFERENCES document(id), start_line INTEGER, end_line INTEGER
+  document_id INTEGER REFERENCES document(id) ON DELETE CASCADE, start_line INTEGER, end_line INTEGER
 );
 
 CREATE TABLE edge (
@@ -126,16 +128,24 @@ CREATE TABLE edge (
   kind TEXT NOT NULL,                                               -- defines | references | calls | imports | inherits | documents
   provider TEXT NOT NULL,                                           -- treesitter | scip | lsp | codegraph | graphify
   confidence INTEGER NOT NULL,                                      -- 3 exact, 2 extracted, 1 inferred, 0 ambiguous
-  evidence_doc INTEGER REFERENCES document(id), evidence_line INTEGER,
+  evidence_doc INTEGER REFERENCES document(id) ON DELETE CASCADE, evidence_line INTEGER,
   PRIMARY KEY (src, dst, kind, provider)
 );
 CREATE INDEX edge_dst ON edge(dst, kind);
 CREATE INDEX symbol_name ON symbol(display_name);
-CREATE INDEX chunk_document ON chunk(document_id);                  -- re-indexing replaces a document's rows
+CREATE INDEX chunk_document ON chunk(document_id);                  -- every foreign key is indexed, so a cascade never scans
+CREATE INDEX chunk_parent ON chunk(parent_id);
+CREATE INDEX chunk_symbol ON chunk(symbol_id);
 CREATE INDEX symbol_document ON symbol(document_id);
 CREATE INDEX edge_evidence ON edge(evidence_doc);
+CREATE TRIGGER chunk_fts_delete AFTER DELETE ON chunk BEGIN
+  DELETE FROM chunk_fts WHERE rowid = old.id;                       -- a contentless FTS table can't be a foreign key's child
+END;
 ```
 
+- **Document versions** are separate sources: each pinned version of a document is its own `source` row, with the document's name, its `sha256` as `revision`, and `is_current`, `obsoleted_by` and `updated_by` from the manifest. A version is unique by name and revision, any other source by name. Results name a version as citations do, `rfc9449@sha256:1a2b3c4d5e6f`.
+- **Deletes cascade.** Every connection that writes turns `PRAGMA foreign_keys` on, so deleting a source deletes its documents, and deleting a document its chunks, symbols and the edges it is evidence for; the trigger takes each chunk's full-text row with it. Migrations run with the keys off, since rebuilding a table to change its keys drops the old one, and check every key (`PRAGMA foreign_key_check`) before they commit.
+
 - **Symbol IDs** are SCIP symbol strings, so precise indexers drop in without remapping. Symbols found only by tree-sitter get SCIP-shaped local IDs, `codecite . <source> . <path segments>/<descriptors>` (e.g. `` codecite . identity-model . src/`client.rs`/Client#validate(). ``), with a SCIP disambiguator on a later same-named function in the file (`new(+1).`); they are replaced when a SCIP index for the same source arrives.
 - **Edges from different providers are kept as separate rows.** Queries collapse them to the highest confidence and list every provider that agrees.
 - **One file per corpus** keeps corpora portable and independently rebuildable. Cross-corpus queries use `ATTACH`. The file lives in the user's data directory, never in a repository: `<data dir>/codecite/<corpus>.db`, which is `~/.local/share/codecite/<corpus>.db` on Linux. A corpus name is ASCII letters, digits, `-`, `_` and `.`, not starting with `.`, and must match the manifest's `name`.
@@ -180,9 +190,9 @@ BM25 over code-aware terms does most of the work; vectors and graph signals refi
 
 1. **Fetch:** `git clone --filter=blob:none` at the manifest's pinned commit, read a local working tree, or fetch a document by URL and check it against its pinned `sha256`. Every document is fetched before anything is written, so changed bytes stop the run with an error naming the document and both hashes, and the store is untouched until the pin is updated on purpose. Fetched documents are cached outside the repo, in `<cache dir>/codecite/documents/<sha256>` (`~/.cache` on Linux): a document whose pinned hash is in the cache is never fetched again, so indexing works offline after the first run. A cache file whose bytes no longer match its name is deleted and fetched again. A download is plain HTTP(S) GET (`ureq`, rustls with Mozilla's roots, the standard proxy variables), capped at 16 MiB and 60 seconds; the hash, not the transport or an ETag, is what pins the bytes. A document marked `excluded` is never fetched.
 2. **Walk:** list the files git sees (`git ls-files`, so `.gitignore`, `.git/info/exclude` and the global excludes apply and tracked files are never ignored), apply the manifest's `include`/`exclude` globs, and skip deny-listed secrets, symlinks, files over 1 MiB and binaries (a NUL byte in the first 8000 bytes, git's rule; extensions are never trusted). Every skipped path is reported with its reason; an ignored directory counts once. A path under a symlinked directory is skipped as a symlink. The repository's config is not trusted, since it can name programs for git to run: the walk turns `core.fsmonitor` off, allows git no transport (an empty `GIT_ALLOW_PROTOCOL`, so a partial clone never fetches a missing object from its promisor remote), and never runs `git status`; `dirty` compares HEAD's tree, the index and `git hash-object --no-filters` of the kept files only.
-3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174). Spec HTML comes in three layouts: xml2rfc v3 RFCs (RFC 8650 and later), the older RFC HTML on rfc-editor.org, and the xml2rfc-generated OpenID pages. `scraper` parses it (html5ever repairs malformed HTML as a browser does), and one iterative walk renders it to plain text: a line per paragraph, list item or table row, `<pre>` kept as written, and page headers, footers and ¶ links dropped. A section runs from its heading to the next heading, numbered or not, so its line range holds its own text and not its subsections'. Front matter, the table of contents, the references and unnumbered parts such as the authors' addresses are not sections. A step is an item of a numbered `<ol>` that sits in no other list, or in an older RFC's plain text a paragraph numbered `1.`, `2.`, … at one indent; it keeps the number the document writes, lists nested in it stay part of its text, and a later list in the same section that reuses a number gives no step for it. Requirement keywords are found in core (`requirement_keywords`), so `cite` can scan a claim the same way: whole uppercase words only, with `MUST NOT`, `SHALL NOT`, `SHOULD NOT` and `NOT RECOMMENDED` read as one keyword across any whitespace. An RFC's "Obsoletes", "Updates", "Obsoleted by" and "Updated by" header fields are read from its HTML (the v3 identifiers block, or the older plain-text header) as `rfc<N>` names. html5ever's parse is quadratic in nesting depth, which is fine for the pinned pages the owner chose but slow for a hostile one.
+3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174). Spec HTML comes in three layouts: xml2rfc v3 RFCs (RFC 8650 and later), the older RFC HTML on rfc-editor.org, and the xml2rfc-generated OpenID pages. `scraper` parses it (html5ever repairs malformed HTML as a browser does), and one iterative walk renders it to plain text, whose lines a spec chunk's line range counts: a line per paragraph, list item or table row, `<pre>` kept as written, and page headers, footers and ¶ links dropped. A section runs from its heading to the next heading, numbered or not, so its line range holds its own text and not its subsections'. Front matter, the table of contents, the references and unnumbered parts such as the authors' addresses are not sections. A step is an item of a numbered `<ol>` that sits in no other list, or in an older RFC's plain text a paragraph numbered `1.`, `2.`, … at one indent; it keeps the number the document writes, lists nested in it stay part of its text, and a later list in the same section that reuses a number gives no step for it. Requirement keywords are found in core (`requirement_keywords`), so `cite` can scan a claim the same way: whole uppercase words only, with `MUST NOT`, `SHALL NOT`, `SHOULD NOT` and `NOT RECOMMENDED` read as one keyword across any whitespace. An RFC's "Obsoletes", "Updates", "Obsoleted by" and "Updated by" header fields are read from its HTML (the v3 identifiers block, or the older plain-text header) as `rfc<N>` names. html5ever's parse is quadratic in nesting depth, which is fine for the pinned pages the owner chose but slow for a hostile one.
 4. **Terms:** split camelCase, snake_case and kebab-case identifiers into lowercase parts and keep the original, so `validateToken` matches "validate token" and the exact name. Any character that is not a letter or digit separates parts, an acronym ends before the capital that starts the next word (`HTTPServer` → `http server`), and digits stay with the part before them (`sha256`, `rfc9449`).
-5. **Write:** chunks, full-text rows and symbols for one document in a single transaction. A document whose sha256 matches the stored one is not rewritten; only its `dirty` flag is updated. A stored document whose file is gone, or is now skipped, is deleted with everything stored for it, and so is every source the run did not index (one removed from the manifest, or marked `excluded`). A chunk's `terms` and its full-text `terms` column hold the identifier terms of every word in its text; the `symbol` column holds its symbol name's terms and the `path` column its path's. A local source without a `name` is named after its checkout directory, and two sources with one name are an error. The run ends with a summary: documents written, unchanged and removed, chunks written, skipped files by reason, and the elapsed time.
+5. **Write:** chunks, full-text rows and symbols for one document in a single transaction. A document whose sha256 matches the stored one is not rewritten; only its `dirty` flag is updated. A document source holds one document, stored under the source's name: a `section` chunk per section and a `step` chunk per step, each step after its section and naming it as its parent, and each with its number, the section's anchor and title, and its requirement keywords as `strength` (`MUST,SHOULD`). A version already stored is never split again, since its revision is its content's hash; a page that isn't UTF-8 or in a known layout stops the run with an error naming the document. A stored document whose file is gone, or is now skipped, is deleted with everything stored for it, and so is every source the run did not index (one removed from the manifest, or marked `excluded`). A chunk's `terms` and its full-text `terms` column hold the identifier terms of every word in its text; the `symbol` column holds its symbol name's terms and the `path` column its path's. A local source without a `name` is named after its checkout directory, and two sources with one name are an error, unless they are versions of one document; a manifest that pins one version twice is refused. The run ends with a summary: documents written, unchanged and removed, chunks written, skipped files by reason, and the elapsed time.
 6. **Providers:** SCIP, CodeGraph and Graphify run after the base pass and add symbols and edges.
 7. **Embed (optional):** a small local model through `fastembed`, 384 dimensions, CPU only.
```

- [ ] **Step 2: Commit**

```bash
git add docs/design.md
git commit -m "docs: describe document versions and cascading deletes (#25)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
