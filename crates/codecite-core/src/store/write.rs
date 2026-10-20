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
