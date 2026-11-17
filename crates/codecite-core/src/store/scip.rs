//! Writing a source's SCIP layer: the precise symbols, occurrences and
//! edges a SCIP index gives, over the tree-sitter symbols an index run
//! wrote. Each write first undoes the previous one, so the layer always
//! matches the index it was last given.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{OptionalExtension, Transaction, params};

use super::Store;
use super::read::file_lines;
use super::search::name_key;
use crate::Result;

/// How sure a SCIP row is: exact (3), the top of the confidence ladder.
pub const SCIP_CONFIDENCE: u8 = 3;

/// The SCIP role bit that marks a definition.
pub const ROLE_DEFINITION: u32 = 0x1;

/// One file of a SCIP index, as codecite reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScipDocument {
    /// The path from the source's root, `/`-separated.
    pub path: String,
    /// The global symbols defined in the file, in the order the index
    /// gives them.
    pub definitions: Vec<ScipDefinition>,
    /// Every occurrence of a global symbol, definitions included.
    pub occurrences: Vec<ScipOccurrence>,
}

/// A symbol a SCIP index defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScipDefinition {
    /// The SCIP symbol string.
    pub symbol: String,
    /// The name as written: the index's display name, else the last
    /// descriptor's name.
    pub display_name: String,
    /// What kind of definition it is: `function`, `method`, `struct`,
    /// `field`, `module`, …
    pub kind: String,
    /// The line the defining name is on, counting from 1.
    pub line: u32,
    /// The definition's first line: its enclosing range's when the index
    /// gives one, else `line`.
    pub start_line: u32,
    /// Its last line, inclusive.
    pub end_line: u32,
}

/// A place a symbol occurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScipOccurrence {
    /// The SCIP symbol string.
    pub symbol: String,
    /// The first line, counting from 1.
    pub start_line: u32,
    /// The last line, inclusive.
    pub end_line: u32,
    /// The SCIP `SymbolRole` bits: 1 definition, 2 import, 4 write, 8
    /// read, 16 generated, 32 test.
    pub roles: u32,
}

/// What writing a source's SCIP layer did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScipSummary {
    /// SCIP documents written.
    pub documents: usize,
    /// SCIP documents whose path the source didn't index, skipped.
    pub missing_documents: usize,
    /// Tree-sitter symbols given their SCIP ID.
    pub matched: usize,
    /// SCIP definitions no tree-sitter symbol held, written as symbols of
    /// their own.
    pub added: usize,
    /// Definitions skipped because their ID was already stored: a second
    /// definition of one symbol, or another source's.
    pub conflicts: usize,
    /// IDs taken from another source's SCIP-only symbol by a definition that
    /// sits on a tree-sitter symbol of the same name here.
    pub taken_over: usize,
    /// The other sources that already held one of those IDs, sorted.
    pub conflicting_sources: Vec<String>,
    /// Occurrences written, definitions included.
    pub occurrences: usize,
    /// Definitions and occurrences past the end of their file, skipped.
    pub past_end: usize,
    /// `calls` and `references` edges written.
    pub edges: usize,
}

impl Store {
    /// Replaces the SCIP layer of the source with `documents`, in one
    /// transaction.
    ///
    /// The previous layer is undone first, as [`Store::clear_scip`] does.
    /// Then each definition replaces the innermost tree-sitter symbol in the
    /// same file with the same name whose lines hold the definition's line:
    /// the symbol takes the SCIP ID and keeps its lines and kind, and its
    /// chunk follows it. A definition no tree-sitter symbol holds becomes a
    /// symbol of its own. Each occurrence is stored with the innermost
    /// definition holding it, and a reference inside a definition gives an
    /// edge from that definition: `calls` to a method or function,
    /// `references` to anything else, with the first such line as evidence.
    /// Documents the source doesn't have, and lines past a file's end, are
    /// counted and skipped.
    pub fn write_scip(
        &mut self,
        source_id: i64,
        documents: &[ScipDocument],
    ) -> Result<ScipSummary> {
        let tx = self.conn.transaction()?;
        undo_scip(&tx, source_id)?;
        let stored = stored_documents(&tx, source_id)?;
        let mut summary = ScipSummary::default();
        // By path, so which of two definitions of one ID wins never depends
        // on the order the indexer wrote its files in.
        let mut found: Vec<(&ScipDocument, i64, u32)> = Vec::new();
        for doc in documents {
            match stored.get(doc.path.as_str()) {
                Some(&(id, lines)) => found.push((doc, id, lines)),
                None => summary.missing_documents += 1,
            }
        }
        found.sort_by(|a, b| a.0.path.cmp(&b.0.path));
        summary.documents = found.len();
        for &(doc, document_id, lines) in &found {
            write_definitions(&tx, source_id, document_id, lines, doc, &mut summary)?;
        }
        for &(doc, document_id, lines) in &found {
            write_occurrences(&tx, document_id, lines, doc, &mut summary)?;
        }
        tx.commit()?;
        Ok(summary)
    }

    /// Undoes every source's SCIP layer, in one transaction, so an index run
    /// applies each source's index to a store where no other source, one the
    /// manifest no longer lists included, still holds a SCIP ID.
    pub fn clear_all_scip(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        let sources: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM source ORDER BY id")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for source_id in sources {
            undo_scip(&tx, source_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Undoes the source's SCIP layer, in one transaction: its occurrences,
    /// its SCIP edges and the symbols only SCIP found are deleted, and every
    /// symbol a SCIP ID replaced gets its tree-sitter ID back. An index run
    /// calls it for a source that no longer names a SCIP index.
    pub fn clear_scip(&mut self, source_id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        undo_scip(&tx, source_id)?;
        tx.commit()?;
        Ok(())
    }
}

/// Whether a SCIP symbol is a method or function: its last descriptor is a
/// method, `name().` or `name(+1).`.
pub(super) fn is_callable(symbol: &str) -> bool {
    symbol.ends_with(").")
}

/// The source's code documents with their text, as `(ID, line count)` by
/// path.
fn stored_documents(tx: &Transaction<'_>, source_id: i64) -> Result<HashMap<String, (i64, u32)>> {
    let mut stmt = tx
        .prepare("SELECT id, path, text FROM document WHERE source_id = ?1 AND text IS NOT NULL")?;
    let rows = stmt.query_map([source_id], |row| {
        let text: String = row.get(2)?;
        let lines = u32::try_from(file_lines(&text).len()).unwrap_or(u32::MAX);
        Ok((row.get(1)?, (row.get(0)?, lines)))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Deletes the source's SCIP occurrences, edges and SCIP-only symbols, and
/// gives each replaced symbol its tree-sitter ID back.
fn undo_scip(tx: &Transaction<'_>, source_id: i64) -> Result<()> {
    const DOCS: &str = "(SELECT id FROM document WHERE source_id = ?1)";
    tx.execute(
        &format!("DELETE FROM occurrence WHERE provider = 'scip' AND document_id IN {DOCS}"),
        [source_id],
    )?;
    tx.execute(
        &format!("DELETE FROM edge WHERE provider = 'scip' AND evidence_doc IN {DOCS}"),
        [source_id],
    )?;
    tx.execute(
        &format!(
            "DELETE FROM symbol WHERE provider = 'scip' AND local_id IS NULL
               AND document_id IN {DOCS}"
        ),
        [source_id],
    )?;
    let replaced: Vec<(String, String)> = {
        let mut stmt = tx.prepare(&format!(
            "SELECT id, local_id FROM symbol WHERE provider = 'scip' AND local_id IS NOT NULL
               AND document_id IN {DOCS} ORDER BY id"
        ))?;
        let rows = stmt.query_map([source_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (id, local_id) in &replaced {
        rename_symbol(tx, id, local_id, "treesitter", None)?;
    }
    Ok(())
}

/// Gives the symbol `from` the ID `to`, moving its chunk with it: a new
/// row with the same name, kind and lines, then the chunk, then the old
/// row's delete, so no chunk ever names a missing symbol.
fn rename_symbol(
    tx: &Transaction<'_>,
    from: &str,
    to: &str,
    provider: &str,
    local_id: Option<&str>,
) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO symbol (id, display_name, kind, document_id, start_line, end_line, name_key,
                             provider, local_id)
         SELECT ?2, display_name, kind, document_id, start_line, end_line, name_key, ?3, ?4
         FROM symbol WHERE id = ?1",
    )?
    .execute(params![from, to, provider, local_id])?;
    tx.prepare_cached("UPDATE chunk SET symbol_id = ?2 WHERE symbol_id = ?1")?
        .execute(params![from, to])?;
    tx.prepare_cached("DELETE FROM symbol WHERE id = ?1")?
        .execute([from])?;
    Ok(())
}

/// Writes one document's definitions: each replaces the tree-sitter
/// symbol that holds it, or is added.
fn write_definitions(
    tx: &Transaction<'_>,
    source_id: i64,
    document_id: i64,
    lines: u32,
    doc: &ScipDocument,
    summary: &mut ScipSummary,
) -> Result<()> {
    let mut definitions: Vec<&ScipDefinition> = doc.definitions.iter().collect();
    definitions.sort_by(|a, b| (a.line, &a.symbol).cmp(&(b.line, &b.symbol)));
    for def in definitions {
        if def.line == 0 || def.line > lines {
            summary.past_end += 1;
            continue;
        }
        let holder: Option<String> = tx
            .prepare_cached(
                "SELECT id FROM symbol
                 WHERE document_id = ?1 AND provider = 'treesitter' AND display_name = ?2
                   AND start_line <= ?3 AND end_line >= ?3
                 ORDER BY end_line - start_line, start_line DESC, id LIMIT 1",
            )?
            .query_row(params![document_id, def.display_name, def.line], |row| {
                row.get(0)
            })
            .optional()?;
        // Who holds the ID already: whether only SCIP found it there, and
        // in which source.
        let taken: Option<(bool, i64, String)> = tx
            .prepare_cached(
                "SELECT sym.local_id IS NULL AND sym.provider = 'scip', s.id, s.name
                 FROM symbol sym
                 JOIN document d ON d.id = sym.document_id
                 JOIN source s ON s.id = d.source_id
                 WHERE sym.id = ?1",
            )?
            .query_row([&def.symbol], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        if let Some((scip_only, other, name)) = taken {
            // A definition that sits on a tree-sitter symbol of the same
            // name wins over another source's that SCIP alone found there,
            // whatever order the manifest lists them in; otherwise the
            // first keeps the ID.
            if scip_only && other != source_id && holder.is_some() {
                // The losing source's references stay, but no longer inside
                // the definition it lost: its edges from it go, and its
                // occurrences lose it as their enclosing definition.
                const LOSER: &str = "(SELECT id FROM document WHERE source_id = ?2)";
                tx.prepare_cached("DELETE FROM symbol WHERE id = ?1")?
                    .execute([&def.symbol])?;
                tx.prepare_cached(&format!(
                    "DELETE FROM edge WHERE src = ?1 AND provider = 'scip'
                       AND evidence_doc IN {LOSER}"
                ))?
                .execute(params![def.symbol, other])?;
                tx.prepare_cached(&format!(
                    "UPDATE occurrence SET enclosing = NULL WHERE enclosing = ?1
                       AND document_id IN {LOSER}"
                ))?
                .execute(params![def.symbol, other])?;
                summary.taken_over += 1;
            } else {
                summary.conflicts += 1;
                if other != source_id && !summary.conflicting_sources.contains(&name) {
                    summary.conflicting_sources.push(name);
                    summary.conflicting_sources.sort();
                }
                continue;
            }
        }
        if let Some(local_id) = holder {
            rename_symbol(tx, &local_id, &def.symbol, "scip", Some(&local_id))?;
            summary.matched += 1;
        } else {
            let start = def.start_line.clamp(1, def.line);
            let end = def.end_line.clamp(def.line, lines);
            tx.prepare_cached(
                "INSERT INTO symbol (id, display_name, kind, document_id, start_line, end_line,
                                     name_key, provider)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'scip')",
            )?
            .execute(params![
                def.symbol,
                def.display_name,
                def.kind,
                document_id,
                start,
                end,
                name_key(&def.display_name)
            ])?;
            summary.added += 1;
        }
    }
    Ok(())
}

/// Writes one document's occurrences, each with the innermost definition
/// holding it, and the edges its references give.
fn write_occurrences(
    tx: &Transaction<'_>,
    document_id: i64,
    lines: u32,
    doc: &ScipDocument,
    summary: &mut ScipSummary,
) -> Result<()> {
    // Every definition in the file, SCIP's and tree-sitter's.
    let ranges: Vec<(String, u32, u32)> = {
        let mut stmt = tx.prepare_cached(
            "SELECT id, start_line, end_line FROM symbol WHERE document_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([document_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut occurrences: Vec<&ScipOccurrence> = doc.occurrences.iter().collect();
    occurrences.sort_by(|a, b| {
        (a.start_line, a.end_line, &a.symbol, a.roles).cmp(&(
            b.start_line,
            b.end_line,
            &b.symbol,
            b.roles,
        ))
    });
    // The first site of each edge, so its evidence is the earliest line.
    let mut edges: BTreeMap<(&str, &str), u32> = BTreeMap::new();
    let mut insert = tx.prepare_cached(
        "INSERT INTO occurrence (symbol_id, document_id, start_line, end_line, roles, enclosing,
                                 provider, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'scip', ?7)",
    )?;
    for occ in occurrences {
        if occ.start_line == 0 || occ.start_line > lines {
            summary.past_end += 1;
            continue;
        }
        let end = occ.end_line.clamp(occ.start_line, lines);
        let enclosing = ranges
            .iter()
            // A definition is inside the next definition out; a reference,
            // a recursive call included, is inside the innermost one.
            .filter(|(id, start, end)| {
                (occ.roles & ROLE_DEFINITION == 0 || *id != occ.symbol)
                    && *start <= occ.start_line
                    && occ.start_line <= *end
            })
            .min_by(|a, b| {
                (a.2.saturating_sub(a.1), std::cmp::Reverse(a.1), &a.0).cmp(&(
                    b.2.saturating_sub(b.1),
                    std::cmp::Reverse(b.1),
                    &b.0,
                ))
            })
            .map(|(id, _, _)| id.as_str());
        insert.execute(params![
            occ.symbol,
            document_id,
            occ.start_line,
            end,
            occ.roles,
            enclosing,
            SCIP_CONFIDENCE
        ])?;
        summary.occurrences += 1;
        if occ.roles & ROLE_DEFINITION == 0
            && let Some(from) = enclosing
        {
            edges.entry((from, &occ.symbol)).or_insert(occ.start_line);
        }
    }
    let mut edge = tx.prepare_cached(
        "INSERT OR IGNORE INTO edge (src, dst, kind, provider, confidence, evidence_doc,
                                     evidence_line)
         VALUES (?1, ?2, ?3, 'scip', ?4, ?5, ?6)",
    )?;
    for ((from, to), line) in edges {
        let kind = if is_callable(to) {
            "calls"
        } else {
            "references"
        };
        summary.edges +=
            edge.execute(params![from, to, kind, SCIP_CONFIDENCE, document_id, line])?;
    }
    Ok(())
}

/// A store holding a small Rust crate as an index run writes it, and the
/// SCIP index rust-analyzer gives for it, for the SCIP and graph tests.
#[cfg(test)]
pub(super) mod fixture {
    use super::*;
    use crate::{ChunkRecord, DocumentRecord, SourceRecord, SymbolRecord};

    pub const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    pub const LIB: &str = "//! A tiny crate.\n\npub mod client;\n\nuse client::Client;\n\n\
/// Builds a client and validates a token with it.\npub fn login(token: &str) -> bool {\n    \
let client = Client::new();\n    client.validate(token)\n}\n\n/// Logs in twice.\n\
pub fn relogin(token: &str) -> bool {\n    login(token) && login(token)\n}\n\n\
fn helper() {}\n\nfn check() {}\n";
    pub const CLIENT: &str = "/// A client.\npub struct Client {\n    pub issuer: String,\n}\n\n\
impl Client {\n    /// A client with no issuer.\n    pub fn new() -> Self {\n        Client {\n            \
issuer: String::new(),\n        }\n    }\n\n    /// Whether the token is usable.\n    \
pub fn validate(&self, token: &str) -> bool {\n        check(token) && !self.issuer.is_empty()\n    \
}\n}\n\nfn check(token: &str) -> bool {\n    !token.is_empty()\n}\n";

    pub const PKG: &str = "rust-analyzer cargo app 0.1.0 ";

    /// A SCIP ID in the crate: `scip("login().")`.
    pub fn scip(descriptors: &str) -> String {
        format!("{PKG}{descriptors}")
    }

    /// The tree-sitter ID an index run gives a definition.
    pub fn local(path: &str, descriptors: &str) -> String {
        let file = path.rsplit('/').next().unwrap();
        format!("codecite . app . src/`{file}`/{descriptors}")
    }

    /// One tree-sitter definition: `(descriptors, name, kind, lines)`.
    type Def<'a> = (&'a str, &'a str, &'a str, (u32, u32));

    fn document<'a>(path: &'a str, text: &'a str, defs: &[Def<'a>]) -> DocumentRecord<'a> {
        let lines: Vec<&str> = text.lines().collect();
        DocumentRecord {
            path,
            language: Some("rust"),
            content_hash: path.as_bytes(),
            size_bytes: text.len() as u64,
            dirty: false,
            text: Some(text),
            chunks: defs
                .iter()
                .map(|&(descriptors, name, kind, (start, end))| ChunkRecord {
                    kind: if kind == "struct" {
                        "class"
                    } else {
                        "function"
                    },
                    start_line: start,
                    end_line: end,
                    text: Box::leak(
                        lines[start as usize - 1..end as usize]
                            .join("\n")
                            .into_boxed_str(),
                    ),
                    number: None,
                    anchor: None,
                    title: None,
                    parent: None,
                    strength: &[],
                    symbol: Some(SymbolRecord {
                        id: local(path, descriptors),
                        display_name: name,
                        kind,
                    }),
                })
                .collect(),
        }
    }

    /// The crate as tree-sitter alone indexes it, and its source's ID.
    pub fn app() -> (tempfile::TempDir, Store, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let id = store
            .upsert_source(&SourceRecord {
                name: "app",
                kind: "local",
                url: "/src/app",
                revision: HEAD,
                license_spdx: Some("MIT"),
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        store
            .write_document(
                id,
                &document(
                    "src/lib.rs",
                    LIB,
                    &[
                        ("login().", "login", "function", (7, 11)),
                        ("relogin().", "relogin", "function", (13, 16)),
                        ("helper().", "helper", "function", (18, 18)),
                        ("check().", "check", "function", (20, 20)),
                    ],
                ),
            )
            .unwrap();
        store
            .write_document(
                id,
                &document(
                    "src/client.rs",
                    CLIENT,
                    &[
                        ("Client#", "Client", "struct", (1, 4)),
                        ("Client#new().", "new", "method", (7, 12)),
                        ("Client#validate().", "validate", "method", (14, 17)),
                        ("check().", "check", "function", (20, 22)),
                    ],
                ),
            )
            .unwrap();
        store.set_indexed_at(30).unwrap();
        (dir, store, id)
    }

    fn def(symbol: &str, name: &str, kind: &str, line: u32, lines: (u32, u32)) -> ScipDefinition {
        ScipDefinition {
            symbol: scip(symbol),
            display_name: name.into(),
            kind: kind.into(),
            line,
            start_line: lines.0,
            end_line: lines.1,
        }
    }

    /// An occurrence of a symbol in the crate (`app`) or outside it.
    fn at(symbol: &str, line: u32, roles: u32) -> ScipOccurrence {
        let symbol = if symbol.starts_with("rust-analyzer ") {
            symbol.to_owned()
        } else {
            scip(symbol)
        };
        ScipOccurrence {
            symbol,
            start_line: line,
            end_line: line,
            roles,
        }
    }

    pub const STRING_NEW: &str = "rust-analyzer cargo alloc https://github.com/rust-lang/rust/library/alloc string/impl#[String]new().";
    pub const IS_EMPTY: &str = "rust-analyzer cargo core https://github.com/rust-lang/rust/library/core str/impl#[str]is_empty().";

    /// The crate's SCIP index, as rust-analyzer writes it (lines from 1).
    pub fn index() -> Vec<ScipDocument> {
        let d = ROLE_DEFINITION;
        let mut lib = ScipDocument {
            path: "src/lib.rs".into(),
            definitions: vec![
                def("crate/", "crate", "module", 1, (1, 21)),
                def("login().", "login", "function", 8, (7, 11)),
                def("relogin().", "relogin", "function", 14, (13, 16)),
            ],
            occurrences: vec![
                at("crate/", 1, d),
                at("client/", 3, 0),
                at("client/", 5, 0),
                at("client/Client#", 5, 2),
                at("login().", 8, d),
                at("client/Client#", 9, 8),
                at("client/impl#[Client]new().", 9, 0),
                at("client/impl#[Client]validate().", 10, 0),
                at("relogin().", 14, d),
                at("login().", 15, 0),
                at("login().", 15, 0),
            ],
        };
        lib.occurrences.reverse();
        let client = ScipDocument {
            path: "src/client.rs".into(),
            definitions: vec![
                def("client/", "client", "module", 1, (1, 22)),
                def("client/Client#", "Client", "struct", 2, (1, 4)),
                def("client/Client#issuer.", "issuer", "field", 3, (3, 3)),
                def("client/impl#[Client]new().", "new", "method", 8, (7, 12)),
                def(
                    "client/impl#[Client]validate().",
                    "validate",
                    "method",
                    15,
                    (14, 17),
                ),
                def("client/check().", "check", "function", 20, (20, 22)),
            ],
            occurrences: vec![
                at("client/", 1, d),
                at("client/Client#", 2, d),
                at("client/Client#issuer.", 3, d),
                at("client/Client#", 6, 0),
                at("client/impl#[Client]new().", 8, d),
                at("client/Client#", 9, 0),
                at("client/Client#issuer.", 10, 4),
                at(STRING_NEW, 10, 0),
                at("client/impl#[Client]validate().", 15, d),
                at("client/check().", 16, 0),
                at("client/Client#issuer.", 16, 8),
                at(IS_EMPTY, 21, 0),
                at("client/check().", 20, d),
            ],
        };
        vec![lib, client]
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{app, index, local, scip};
    use super::*;
    use crate::Error;

    /// Every row a SCIP layer touches, sorted, to compare two stores.
    fn rows(store: &Store) -> Vec<String> {
        let mut out = Vec::new();
        for sql in [
            "SELECT id || '|' || display_name || '|' || coalesce(kind, '') || '|' || document_id
               || '|' || start_line || '-' || end_line || '|' || provider || '|'
               || coalesce(local_id, '') FROM symbol",
            "SELECT id || '|' || coalesce(symbol_id, '') FROM chunk",
            "SELECT symbol_id || '|' || document_id || '|' || start_line || '|' || roles || '|'
               || coalesce(enclosing, '') FROM occurrence",
            "SELECT src || '|' || dst || '|' || kind || '|' || provider || '|' || confidence
               || '|' || evidence_doc || '|' || evidence_line FROM edge",
        ] {
            let mut stmt = store.conn.prepare(sql).unwrap();
            let mut table: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            table.sort();
            out.extend(table);
            out.push("--".into());
        }
        out
    }

    fn symbol_of_chunk(store: &Store, start: u32, path: &str) -> Option<String> {
        store
            .conn
            .query_row(
                "SELECT c.symbol_id FROM chunk c JOIN document d ON d.id = c.document_id
                 WHERE d.path = ?1 AND c.start_line = ?2",
                params![path, start],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn definitions_replace_the_tree_sitter_symbols_that_hold_them() {
        let (_dir, mut store, id) = app();
        let summary = store.write_scip(id, &index()).unwrap();
        assert_eq!(summary.documents, 2);
        // login, relogin, Client, new, validate and client.rs's check.
        assert_eq!(summary.matched, 6);
        // The two modules and the field.
        assert_eq!(summary.added, 3);
        assert_eq!(summary.conflicts, 0);
        assert_eq!(summary.past_end, 0);
        assert_eq!(
            symbol_of_chunk(&store, 7, "src/lib.rs"),
            Some(scip("login()."))
        );
        assert_eq!(
            symbol_of_chunk(&store, 14, "src/client.rs"),
            Some(scip("client/impl#[Client]validate()."))
        );
        // lib.rs's own `check` isn't the one SCIP defines.
        assert_eq!(
            symbol_of_chunk(&store, 20, "src/lib.rs"),
            Some(local("src/lib.rs", "check()."))
        );
        let (lines, provider, local_id): (String, String, Option<String>) = store
            .conn
            .query_row(
                "SELECT start_line || '-' || end_line, provider, local_id FROM symbol
                 WHERE id = ?1",
                [scip("login().")],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(lines, "7-11", "the chunk's lines, doc comment included");
        assert_eq!(provider, "scip");
        assert_eq!(local_id, Some(local("src/lib.rs", "login().")));
        let field: (String, String) = store
            .conn
            .query_row(
                "SELECT kind, start_line || '-' || end_line FROM symbol WHERE id = ?1",
                [scip("client/Client#issuer.")],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(field, ("field".into(), "3-3".into()));
    }

    #[test]
    fn references_give_calls_and_references_edges_from_their_enclosing_definition() {
        let (_dir, mut store, id) = app();
        let summary = store.write_scip(id, &index()).unwrap();
        let edges = |src: &str| -> Vec<(String, String, u32)> {
            let mut stmt = store
                .conn
                .prepare(
                    "SELECT dst, kind, evidence_line FROM edge WHERE src = ?1 AND provider = 'scip'
                     AND confidence = 3 ORDER BY dst",
                )
                .unwrap();
            stmt.query_map([src], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            edges(&scip("login().")),
            [
                (scip("client/Client#"), "references".into(), 9),
                (scip("client/impl#[Client]new()."), "calls".into(), 9),
                (scip("client/impl#[Client]validate()."), "calls".into(), 10),
            ]
        );
        // Two calls on one line are one edge.
        assert_eq!(
            edges(&scip("relogin().")),
            [(scip("login()."), "calls".into(), 15)]
        );
        // A top-level `use` is inside the crate's module; an `impl` line
        // inside the file's.
        assert_eq!(
            edges(&scip("crate/")),
            [
                (scip("client/"), "references".into(), 3),
                (scip("client/Client#"), "references".into(), 5),
            ]
        );
        assert_eq!(
            edges(&scip("client/check().")),
            [(fixture::IS_EMPTY.into(), "calls".into(), 21)]
        );
        assert_eq!(summary.edges, 13);
        // Every occurrence is kept, the two on line 15 included.
        assert_eq!(summary.occurrences, 24);
        let enclosing: Option<String> = store
            .conn
            .query_row(
                "SELECT enclosing FROM occurrence WHERE symbol_id = ?1 AND roles = 4",
                [scip("client/Client#issuer.")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(enclosing, Some(scip("client/impl#[Client]new().")));
    }

    #[test]
    fn a_recursive_call_is_an_edge_from_the_function_itself() {
        let (_dir, mut store, id) = app();
        let mut docs = index();
        // login calls itself on line 10, inside its own lines and the
        // crate module's.
        docs[0].occurrences.push(ScipOccurrence {
            symbol: scip("login()."),
            start_line: 10,
            end_line: 10,
            roles: 0,
        });
        store.write_scip(id, &docs).unwrap();
        let enclosing: Option<String> = store
            .conn
            .query_row(
                "SELECT enclosing FROM occurrence WHERE symbol_id = ?1 AND start_line = 10",
                [scip("login().")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(enclosing, Some(scip("login().")));
        let from_module: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM edge WHERE src = ?1 AND dst = ?2",
                [scip("crate/"), scip("login().")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(from_module, 0, "the module doesn't call login");
        let to_itself: String = store
            .conn
            .query_row(
                "SELECT kind FROM edge WHERE src = ?1 AND dst = ?1",
                [scip("login().")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(to_itself, "calls");
        // A definition is still inside the next definition out.
        let defined_in: Option<String> = store
            .conn
            .query_row(
                "SELECT enclosing FROM occurrence WHERE symbol_id = ?1 AND roles = 1",
                [scip("login().")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(defined_in, Some(scip("crate/")));
    }

    #[test]
    fn every_symbol_keeps_its_name_key() {
        let (_dir, mut store, id) = app();
        store.write_scip(id, &index()).unwrap();
        let keys = |store: &Store| -> Vec<(String, Option<String>)> {
            let mut stmt = store
                .conn
                .prepare("SELECT display_name, name_key FROM symbol ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        for (name, key) in keys(&store) {
            assert_eq!(key, Some(super::super::search::name_key(&name)), "{name}");
        }
        store.clear_scip(id).unwrap();
        for (name, key) in keys(&store) {
            assert_eq!(key, Some(super::super::search::name_key(&name)), "{name}");
        }
    }

    #[test]
    fn writing_again_undoes_the_last_layer_first() {
        let (_dir, mut store, id) = app();
        let before = rows(&store);
        store.write_scip(id, &index()).unwrap();
        let once = rows(&store);
        store.write_scip(id, &index()).unwrap();
        assert_eq!(rows(&store), once, "the same index writes the same rows");

        // An index without `login` gives login its tree-sitter ID back.
        let mut changed = index();
        changed[0]
            .definitions
            .retain(|d| d.symbol != scip("login()."));
        changed[0]
            .occurrences
            .retain(|o| o.symbol != scip("login()."));
        store.write_scip(id, &changed).unwrap();
        assert_eq!(
            symbol_of_chunk(&store, 7, "src/lib.rs"),
            Some(local("src/lib.rs", "login()."))
        );

        store.clear_scip(id).unwrap();
        assert_eq!(rows(&store), before, "clearing restores tree-sitter's rows");
    }

    #[test]
    fn rewriting_a_document_drops_its_scip_rows() {
        let (_dir, mut store, id) = app();
        store.write_scip(id, &index()).unwrap();
        // An index run rewrites a changed file before the SCIP pass.
        store.delete_document(id, "src/client.rs").unwrap();
        let left: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM occurrence o JOIN document d ON d.id = o.document_id
                 WHERE d.path = 'src/client.rs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        let summary = store.write_scip(id, &index()).unwrap();
        assert_eq!(summary.missing_documents, 1);
        assert_eq!(summary.documents, 1);
    }

    #[test]
    fn lines_past_the_end_and_taken_ids_are_counted_not_written() {
        let (_dir, mut store, id) = app();
        let mut docs = index();
        docs[0].occurrences.push(ScipOccurrence {
            symbol: scip("login()."),
            start_line: 99,
            end_line: 99,
            roles: 0,
        });
        docs[0].definitions.push(ScipDefinition {
            symbol: scip("gone()."),
            display_name: "gone".into(),
            kind: "function".into(),
            line: 0,
            start_line: 0,
            end_line: 0,
        });
        // A second definition of `login`, in another file.
        docs[1].definitions.push(ScipDefinition {
            symbol: scip("login()."),
            display_name: "login".into(),
            kind: "function".into(),
            line: 21,
            start_line: 21,
            end_line: 21,
        });
        docs.push(ScipDocument {
            path: "src/missing.rs".into(),
            ..ScipDocument::default()
        });
        let summary = store.write_scip(id, &docs).unwrap();
        assert_eq!(summary.past_end, 2);
        assert_eq!(summary.conflicts, 1);
        assert_eq!(summary.missing_documents, 1);
        let far: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM occurrence WHERE start_line = 99",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(far, 0);
    }

    #[test]
    fn definition_reports_scip_and_finds_dotted_names_in_scip_ids() {
        let (_dir, mut store, id) = app();
        let before = store.definition("login", false).unwrap();
        assert_eq!(before.results[0].id, local("src/lib.rs", "login()."));
        assert_eq!(before.results[0].providers, ["treesitter"]);
        assert_eq!(before.results[0].confidence, 2);
        store.write_scip(id, &index()).unwrap();
        let after = store.definition("login", false).unwrap();
        assert_eq!(after.results[0].providers, ["scip", "treesitter"]);
        assert_eq!(after.results[0].id, scip("login()."));
        assert_eq!(after.results[0].confidence, 3);
        assert_eq!(after.results[0].citation, before.results[0].citation);
        let dotted = store.definition("Client.validate", false).unwrap();
        assert_eq!(dotted.total, 1);
        assert_eq!(dotted.results[0].providers, ["scip", "treesitter"]);
        // SCIP-only symbols have no chunk, so `definition` doesn't list them,
        // nor suggest them.
        let err = store.definition("Issuer", false).unwrap_err();
        let Error::NoSuchSymbol { candidates, .. } = &err else {
            panic!("{err:?}")
        };
        assert!(candidates.is_empty(), "{candidates:?}");
    }

    /// A second source, `other`, whose one file SCIP says defines
    /// `login()` though tree-sitter found nothing there.
    fn other_source(store: &mut Store) -> (i64, Vec<ScipDocument>) {
        use crate::{DocumentRecord, SourceRecord};
        let id = store
            .upsert_source(&SourceRecord {
                name: "other",
                kind: "local",
                url: "/src/other",
                revision: fixture::HEAD,
                license_spdx: None,
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        store
            .write_document(
                id,
                &DocumentRecord {
                    path: "src/lib.rs",
                    language: Some("rust"),
                    content_hash: b"other",
                    size_bytes: 3,
                    dirty: false,
                    text: Some("// login\n"),
                    chunks: vec![],
                },
            )
            .unwrap();
        let docs = vec![ScipDocument {
            path: "src/lib.rs".into(),
            definitions: vec![ScipDefinition {
                symbol: scip("login()."),
                display_name: "login".into(),
                kind: "function".into(),
                line: 1,
                start_line: 1,
                end_line: 1,
            }],
            occurrences: vec![],
        }];
        (id, docs)
    }

    /// The source holding a symbol ID.
    fn owner(store: &Store, symbol: &str) -> String {
        store
            .conn
            .query_row(
                "SELECT s.name FROM symbol sym JOIN document d ON d.id = sym.document_id
                 JOIN source s ON s.id = d.source_id WHERE sym.id = ?1",
                [symbol],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn a_definition_on_a_tree_sitter_symbol_wins_an_id_whatever_the_order() {
        // `other` first: app's definition, on its tree-sitter `login`, takes
        // the ID over.
        let (_dir, mut store, app_id) = app();
        let (other, docs) = other_source(&mut store);
        store.write_scip(other, &docs).unwrap();
        let summary = store.write_scip(app_id, &index()).unwrap();
        assert_eq!((summary.taken_over, summary.conflicts), (1, 0));
        assert_eq!(owner(&store, &scip("login().")), "app");

        // app first: `other` can't take it, and the summary names app.
        let (_dir, mut store, app_id) = app();
        let (other, docs) = other_source(&mut store);
        store.write_scip(app_id, &index()).unwrap();
        let summary = store.write_scip(other, &docs).unwrap();
        assert_eq!(summary.conflicts, 1);
        assert_eq!(summary.conflicting_sources, ["app"]);
        assert_eq!(owner(&store, &scip("login().")), "app");
    }

    #[test]
    fn taking_an_id_over_drops_the_losers_edges_from_it() {
        let (_dir, mut store, app_id) = app();
        let (other, mut docs) = other_source(&mut store);
        // `other`'s login calls `new` and `check`; app's calls `new` too.
        for callee in ["client/impl#[Client]new().", "client/check()."] {
            docs[0].occurrences.push(ScipOccurrence {
                symbol: scip(callee),
                start_line: 1,
                end_line: 1,
                roles: 0,
            });
        }
        store.write_scip(other, &docs).unwrap();
        let summary = store.write_scip(app_id, &index()).unwrap();
        assert_eq!(summary.taken_over, 1);
        let edges: Vec<(String, String)> = {
            let mut stmt = store
                .conn
                .prepare(
                    "SELECT e.dst, d.path || '@' || s.name FROM edge e
                     JOIN document d ON d.id = e.evidence_doc JOIN source s ON s.id = d.source_id
                     WHERE e.src = ?1 ORDER BY e.dst",
                )
                .unwrap();
            stmt.query_map([scip("login().")], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        // Only app's calls, each with app's evidence.
        assert_eq!(
            edges,
            [
                (scip("client/Client#"), "src/lib.rs@app".to_owned()),
                (
                    scip("client/impl#[Client]new()."),
                    "src/lib.rs@app".to_owned()
                ),
                (
                    scip("client/impl#[Client]validate()."),
                    "src/lib.rs@app".to_owned()
                ),
            ]
        );
        let held: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM occurrence WHERE enclosing = ?1 AND document_id IN
                   (SELECT id FROM document WHERE source_id = ?2)",
                params![scip("login()."), other],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(held, 0, "the loser's references are no longer inside login");
    }

    #[test]
    fn clearing_every_source_frees_the_ids_they_held() {
        let (_dir, mut store, app_id) = app();
        let (other, docs) = other_source(&mut store);
        store.write_scip(app_id, &index()).unwrap();
        store.write_scip(other, &docs).unwrap();
        store.clear_all_scip().unwrap();
        let scip_rows: i64 = store
            .conn
            .query_row(
                "SELECT (SELECT count(*) FROM symbol WHERE provider = 'scip')
                      + (SELECT count(*) FROM occurrence) + (SELECT count(*) FROM edge)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scip_rows, 0);
        // With app's layer gone, `other` now holds the ID.
        store.write_scip(other, &docs).unwrap();
        assert_eq!(owner(&store, &scip("login().")), "other");
    }

    #[test]
    fn only_method_descriptors_are_callable() {
        assert!(is_callable("a b c d x/f()."));
        assert!(is_callable("a b c d T#f(+1)."));
        assert!(!is_callable("a b c d T#field."));
        assert!(!is_callable("a b c d f().(param)"));
        assert!(!is_callable("a b c d T#"));
    }
}
