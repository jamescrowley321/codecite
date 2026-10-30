//! Writing sources and their documents into the store. Each call is one
//! transaction, so a document is either fully written or not at all.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, Transaction, params};

use super::Store;
use crate::{Keyword, Result, SpecAliases, identifier_terms, text_terms};

/// A source to create or update. A document is matched by its name and
/// revision, since each pinned version is its own source; any other source
/// by its name alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecord<'a> {
    /// The name results cite. A document's versions share it.
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
    /// Whether this is the version of a document that queries cite by
    /// default. Always true for a source that isn't a document.
    pub is_current: bool,
    /// The document that replaces this one, as the manifest names it.
    pub obsoleted_by: Option<&'a str>,
    /// A document that amends this one, as the manifest names it.
    pub updated_by: Option<&'a str>,
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
    /// The file's text, which `read` cites lines of. `None` for a spec,
    /// whose sections and steps are its text.
    pub text: Option<&'a str>,
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
    /// A spec section's number (`3.1.3.7`), or a step's (`13`).
    pub number: Option<&'a str>,
    /// A spec section's own anchor, without the `#`: `IDTokenValidation`.
    pub anchor: Option<&'a str>,
    /// A section's heading.
    pub title: Option<&'a str>,
    /// The index, in [`DocumentRecord::chunks`], of the enclosing section
    /// of a spec section or step. It must be an earlier chunk.
    pub parent: Option<usize>,
    /// The requirement keywords in a spec section or step, stored as
    /// `MUST,SHOULD`. Empty for code.
    pub strength: &'a [Keyword],
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
    /// Creates the source, or updates the stored one's other fields and
    /// `indexed_at`, and returns its ID. Whether it is guidance is left as
    /// it is: [`Store::set_guidance`] and [`Store::clear_guidance`] change
    /// that, so a guidance source being indexed again never answers with no
    /// rules in between. A document version is the stored
    /// document with the same name and revision; any other source is the
    /// stored one with the same name that isn't a document.
    pub fn upsert_source(&mut self, source: &SourceRecord<'_>) -> Result<i64> {
        // An upsert names the unique index it conflicts on, and a partial
        // index is named by its columns plus its `WHERE`.
        let conflict = if source.kind == "document" {
            "(name, revision) WHERE kind = 'document'"
        } else {
            "(name) WHERE kind <> 'document'"
        };
        let id = self.conn.query_row(
            &format!(
                "INSERT INTO source (name, kind, url, revision, is_current, obsoleted_by,
                   updated_by, license_spdx, license_class, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 ON CONFLICT {conflict} DO UPDATE SET
                   kind = excluded.kind, url = excluded.url, revision = excluded.revision,
                   is_current = excluded.is_current, obsoleted_by = excluded.obsoleted_by,
                   updated_by = excluded.updated_by, license_spdx = excluded.license_spdx,
                   license_class = excluded.license_class, indexed_at = excluded.indexed_at
                 RETURNING id"
            ),
            params![
                source.name,
                source.kind,
                source.url,
                source.revision,
                source.is_current,
                source.obsoleted_by,
                source.updated_by,
                source.license_spdx,
                source.license_class,
            ],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Marks the source as no longer guidance and deletes its rule scopes,
    /// in one transaction, so a guidance source the manifest turns into a
    /// plain document stops giving rules.
    pub fn clear_guidance(&mut self, source_id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE source SET guidance = 0 WHERE id = ?1", [source_id])?;
        tx.execute("DELETE FROM rule_scope WHERE source_id = ?1", [source_id])?;
        tx.commit()?;
        Ok(())
    }

    /// The content hash of each document stored for the source, by path.
    pub fn document_hashes(&self, source_id: i64) -> Result<HashMap<String, Vec<u8>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, content_hash FROM document WHERE source_id = ?1")?;
        let rows = stmt.query_map([source_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The language stored for the document at `path` in the source:
    /// `None` when there is no such document, `Some(None)` when it has no
    /// language.
    pub fn document_language(&self, source_id: i64, path: &str) -> Result<Option<Option<String>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT language FROM document WHERE source_id = ?1 AND path = ?2",
                params![source_id, path],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Whether the document at `path` in the source stores its text: false
    /// for a spec or guidance, whose chunks are their text, and when there
    /// is no such document.
    pub fn document_has_text(&self, source_id: i64, path: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT text IS NOT NULL FROM document WHERE source_id = ?1 AND path = ?2",
                params![source_id, path],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
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
            "INSERT INTO document (source_id, path, language, content_hash, size_bytes, dirty, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (source_id, path) DO UPDATE SET
               language = excluded.language, content_hash = excluded.content_hash,
               size_bytes = excluded.size_bytes, dirty = excluded.dirty, text = excluded.text
             RETURNING id",
            params![
                source_id,
                document.path,
                document.language,
                document.content_hash,
                // SQLite integers are i64; no file comes near i64::MAX bytes.
                i64::try_from(document.size_bytes).unwrap_or(i64::MAX),
                document.dirty,
                document.text,
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

    /// Deletes the document at `path` in the source. Its chunks, their
    /// full-text rows, its symbols and the edges it is evidence for go with
    /// it, in the same statement.
    pub fn delete_document(&mut self, source_id: i64, path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM document WHERE source_id = ?1 AND path = ?2",
            params![source_id, path],
        )?;
        Ok(())
    }

    /// Deletes every source whose ID is not in `keep`, with everything
    /// stored for it, in one transaction. Returns the deleted sources as
    /// citations name them, sorted: `app`, or `rfc9449@sha256:1a2b3c4d5e6f`
    /// for a document version.
    pub fn delete_sources_except(&mut self, keep: &[i64]) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        let mut sources: Vec<(i64, String)> = {
            let mut stmt = tx.prepare(
                "SELECT id, CASE kind WHEN 'document' THEN name || '@sha256:' || substr(revision, 1, 12)
                   ELSE name END AS label
                 FROM source ORDER BY label",
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        sources.retain(|(id, _)| !keep.contains(id));
        let mut delete = tx.prepare("DELETE FROM source WHERE id = ?1")?;
        for (id, _) in &sources {
            // `ON DELETE CASCADE` takes the documents, and they take
            // their chunks, symbols and edges.
            delete.execute([id])?;
        }
        drop(delete);
        tx.commit()?;
        Ok(sources.into_iter().map(|(_, label)| label).collect())
    }

    /// Records when the corpus was last indexed, in `meta.indexed_at`, and
    /// the manifest's `max_age_days`, so queries can refuse a stale index
    /// without reading the manifest.
    pub fn set_indexed_at(&mut self, max_age_days: u32) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value)
             VALUES ('indexed_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    ('max_age_days', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [max_age_days.to_string()],
        )?;
        Ok(())
    }

    /// Replaces the stored document aliases with `aliases`, so `read` can
    /// resolve `OIDC Core §3.1` without the manifest.
    pub fn set_aliases(&mut self, aliases: &SpecAliases) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM alias", [])?;
        let mut insert = tx.prepare("INSERT INTO alias (alias, name) VALUES (?1, ?2)")?;
        for (alias, name) in aliases.iter() {
            insert.execute([alias, name])?;
        }
        drop(insert);
        tx.commit()?;
        Ok(())
    }
}

/// Deletes a document's chunks, their full-text rows, its symbols and the
/// edges it is evidence for, keeping the document row.
fn clear_document(tx: &Transaction<'_>, document_id: i64) -> Result<()> {
    // The `chunk_fts_delete` and `spec_fts_delete` triggers delete each
    // chunk's full-text row.
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

/// Inserts a document's symbols, chunks and full-text rows: every chunk's
/// into `chunk_fts`, and a numbered one's, a spec section or step or a
/// rule, into the stemmed `spec_fts` as well. Code ranks against
/// `chunk_fts`, whose BM25 statistics count spec chunks, so they stay in
/// it.
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
        "INSERT INTO chunk (document_id, start_line, end_line, kind, symbol_id, number, anchor,
           title, parent_id, strength, text, terms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    let mut fts = tx.prepare_cached(
        "INSERT INTO chunk_fts (rowid, symbol, path, terms) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut spec_fts = tx.prepare_cached(
        "INSERT INTO spec_fts (rowid, symbol, path, terms) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let path_terms = text_terms(document.path);
    // Chunk IDs as they are assigned, so a later chunk can name its parent.
    let mut ids: Vec<i64> = Vec::with_capacity(document.chunks.len());
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
        debug_assert!(c.parent.is_none_or(|p| p < ids.len()), "parent after child");
        let parent_id = c.parent.and_then(|p| ids.get(p).copied());
        let strength = (!c.strength.is_empty()).then(|| {
            let words: Vec<&str> = c.strength.iter().map(|k| k.as_str()).collect();
            words.join(",")
        });
        let chunk_id = chunk.insert(params![
            document_id,
            c.start_line,
            c.end_line,
            c.kind,
            c.symbol.as_ref().map(|s| &s.id),
            c.number,
            c.anchor,
            c.title,
            parent_id,
            strength,
            c.text,
            terms,
        ])?;
        ids.push(chunk_id);
        let symbol_terms = c
            .symbol
            .as_ref()
            .map_or_else(String::new, |s| identifier_terms(s.display_name));
        fts.execute(params![chunk_id, symbol_terms, path_terms, terms])?;
        if c.number.is_some() {
            spec_fts.execute(params![chunk_id, symbol_terms, path_terms, terms])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// A spec chunk as (kind, number, anchor, parent's number, strength).
    type SpecRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );

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
            is_current: true,
            obsoleted_by: None,
            updated_by: None,
        }
    }

    /// One pinned version of the document `name`.
    fn version<'a>(name: &'a str, sha256: &'a str, is_current: bool) -> SourceRecord<'a> {
        SourceRecord {
            name,
            kind: "document",
            url: "https://www.rfc-editor.org/rfc/rfc9449.html",
            revision: sha256,
            license_spdx: Some("IETF Trust"),
            license_class: "quote",
            is_current,
            obsoleted_by: None,
            updated_by: None,
        }
    }

    fn document<'a>(path: &'a str, text: &'a str) -> DocumentRecord<'a> {
        DocumentRecord {
            path,
            language: Some("rust"),
            content_hash: text.as_bytes(),
            size_bytes: text.len() as u64,
            dirty: false,
            text: Some(text),
            chunks: vec![ChunkRecord {
                kind: "function",
                start_line: 1,
                end_line: 1,
                text,
                number: None,
                anchor: None,
                title: None,
                parent: None,
                strength: &[],
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
        matches_in(store, "chunk_fts", query)
    }

    /// How many rows of the full-text `table` match `query`.
    fn matches_in(store: &Store, table: &str, query: &str) -> i64 {
        store
            .conn
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {table} MATCH ?1"),
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
        let stored: String = store
            .conn
            .query_row("SELECT text FROM document", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, text);
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
        let app = store.upsert_source(&source("app")).unwrap();
        assert_eq!(store.delete_sources_except(&[app]).unwrap(), ["old"]);
        assert_eq!(count(&store, "SELECT count(*) FROM source"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM document"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM symbol"), 1);
        assert_eq!(matches(&store, "path:old"), 0);
        assert!(store.delete_sources_except(&[app]).unwrap().is_empty());
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
        store.set_indexed_at(7).unwrap();
        let meta = |key: &str| -> String {
            store
                .conn
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        let at = meta("indexed_at");
        assert_eq!(at.len(), "2026-10-20T06:00:00Z".len(), "{at}");
        assert_eq!(meta("max_age_days"), "7");
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
            // The lookups `ON DELETE` runs for each deleted chunk and symbol.
            ("SELECT id FROM chunk WHERE parent_id = 1", "chunk_parent"),
            ("SELECT id FROM chunk WHERE symbol_id = 'x'", "chunk_symbol"),
            (
                "SELECT id FROM document WHERE source_id = 1",
                "sqlite_autoindex_document_1",
            ),
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

    #[test]
    fn each_document_version_is_its_own_source() {
        let (mut store, dir) = store("versions");
        let old = "1".repeat(64);
        let new = "2".repeat(64);
        let first = store
            .upsert_source(&version("rfc9449", &old, false))
            .unwrap();
        let second = store
            .upsert_source(&version("rfc9449", &new, true))
            .unwrap();
        assert_ne!(first, second);
        // The same version again is the same row, with its fields updated.
        let mut obsoleted = version("rfc9449", &old, false);
        obsoleted.obsoleted_by = Some("rfc9999");
        assert_eq!(store.upsert_source(&obsoleted).unwrap(), first);
        let rows: Vec<(String, bool, Option<String>)> = {
            let mut stmt = store
                .conn
                .prepare("SELECT revision, is_current, obsoleted_by FROM source ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            rows,
            [
                (old.clone(), false, Some("rfc9999".to_owned())),
                (new.clone(), true, None)
            ]
        );

        // A local source keeps its row when its revision moves.
        let app = store.upsert_source(&source("app")).unwrap();
        let mut moved = source("app");
        moved.revision = "fedcba9876543210fedcba9876543210fedcba98";
        assert_eq!(store.upsert_source(&moved).unwrap(), app);

        assert_eq!(
            store.delete_sources_except(&[second]).unwrap(),
            ["app".to_owned(), format!("rfc9449@sha256:{}", &old[..12])]
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_aliases_replaces_the_stored_ones() {
        let (mut store, dir) = store("aliases");
        let first: SpecAliases = [("OIDC Core", "openid-connect-core-1_0"), ("Core", "x")]
            .into_iter()
            .collect();
        store.set_aliases(&first).unwrap();
        let second: SpecAliases = [("Core", "openid-connect-core-1_0")].into_iter().collect();
        store.set_aliases(&second).unwrap();
        let rows: Vec<(String, String)> = {
            let mut stmt = store.conn.prepare("SELECT alias, name FROM alias").unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            rows,
            [("core".to_owned(), "openid-connect-core-1_0".to_owned())]
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn spec_chunks_store_their_number_parent_and_strength() {
        let (mut store, dir) = store("spec");
        let sha = "1".repeat(64);
        let id = store
            .upsert_source(&version("rfc9449", &sha, true))
            .unwrap();
        let chunk = |kind, number, parent, text, strength| ChunkRecord {
            kind,
            start_line: 1,
            end_line: 1,
            text,
            number: Some(number),
            anchor: (kind == "section").then_some("section-4.3"),
            title: (kind == "section").then_some("Checking DPoP Proofs"),
            parent,
            strength,
            symbol: None,
        };
        let doc = DocumentRecord {
            path: "rfc9449",
            language: None,
            content_hash: sha.as_bytes(),
            size_bytes: 10,
            dirty: false,
            text: None,
            chunks: vec![
                chunk(
                    "section",
                    "4.3",
                    None,
                    "4.3. Checking DPoP Proofs",
                    &[Keyword::Must, Keyword::May],
                ),
                chunk("step", "1", Some(0), "1. One DPoP header.", &[]),
            ],
        };
        assert_eq!(store.write_document(id, &doc).unwrap(), 2);

        let rows: Vec<SpecRow> = {
            let mut stmt = store
                .conn
                .prepare(
                    "SELECT c.kind, c.number, c.anchor, p.number, c.strength
                     FROM chunk c LEFT JOIN chunk p ON p.id = c.parent_id ORDER BY c.id",
                )
                .unwrap();
            stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
        };
        assert_eq!(
            rows,
            [
                (
                    "section".to_owned(),
                    "4.3".to_owned(),
                    Some("section-4.3".to_owned()),
                    None,
                    Some("MUST,MAY".to_owned())
                ),
                (
                    "step".to_owned(),
                    "1".to_owned(),
                    None,
                    Some("4.3".to_owned()),
                    None
                ),
            ]
        );
        // Spec chunks go in the stemmed table, so "checked" finds
        // "Checking", and in the unstemmed one, which doesn't.
        assert_eq!(matches_in(&store, "spec_fts", "dpop"), 2);
        assert_eq!(matches_in(&store, "spec_fts", "checked"), 1);
        assert_eq!(matches(&store, "dpop"), 2);
        assert_eq!(matches(&store, "checked"), 0);

        // Deleting the source takes the document, both chunks and their
        // full-text rows with it.
        assert_eq!(store.delete_sources_except(&[]).unwrap().len(), 1);
        assert_eq!(count(&store, "SELECT count(*) FROM document"), 0);
        assert_eq!(count(&store, "SELECT count(*) FROM chunk"), 0);
        assert_eq!(matches_in(&store, "spec_fts", "dpop"), 0);
        assert_eq!(matches(&store, "dpop"), 0);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }
}
