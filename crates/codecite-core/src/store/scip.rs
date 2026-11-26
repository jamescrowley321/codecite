//! Writing a source's SCIP layer: the precise symbols, occurrences and
//! edges a SCIP index gives, over the tree-sitter symbols an index run
//! wrote. Each write first undoes the previous one, so the layer always
//! matches the index it was last given.

use std::collections::{BTreeMap, HashMap, hash_map::Entry};

use rusqlite::{Connection, OptionalExtension, params};

use super::Store;
use super::read::file_lines;
use super::search::name_key;
use crate::Result;

/// How sure a SCIP row is: exact (3), the top of the confidence ladder.
pub const SCIP_CONFIDENCE: u8 = 3;

/// The SCIP role bit that marks a definition.
pub const ROLE_DEFINITION: u32 = 0x1;

/// The longest name taken from a SCIP index that reaches agents: a display
/// name, or the name a symbol's descriptors give. A longer one is cut on a
/// character boundary or gives way to a shorter one, so an index can't hand
/// agents a long text as a name.
pub const MAX_SCIP_NAME_BYTES: usize = 256;

/// The SCIP role bit that marks an import.
pub const ROLE_IMPORT: u32 = 0x2;

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
    /// The unit the occurrences' columns count in.
    pub encoding: ColumnUnit,
}

/// The unit a SCIP document's columns count in: its `PositionEncoding`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColumnUnit {
    /// UTF-8 code units: bytes.
    Utf8,
    /// UTF-16 code units.
    Utf16,
    /// UTF-32 code units: Unicode scalar values, Rust `char`s.
    Utf32,
    /// The index doesn't say.
    #[default]
    Unspecified,
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
    /// Where the range ends on its last line: the half-open end character
    /// in the document's [`ColumnUnit`]. `None` when the range doesn't give
    /// one.
    pub end_column: Option<u32>,
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
    /// `calls`, `references` and `imports` edges written.
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
    /// edge from that definition, with the first line of each kind as
    /// evidence: `imports` when the occurrence has the import role; `calls`
    /// when the target is a method, function or term and the source text
    /// after the name opens an argument list, or, when the occurrence's
    /// column can't be read there, when the target is a method or function;
    /// `references` otherwise.
    /// Documents the source doesn't have, and lines past a file's end, are
    /// counted and skipped.
    pub fn write_scip(
        &mut self,
        source_id: i64,
        documents: &[ScipDocument],
    ) -> Result<ScipSummary> {
        let tx = self.conn.savepoint()?;
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

    /// Undoes every source's SCIP layer, in one savepoint, so an index run
    /// applies each source's index to a store where no other source, one the
    /// manifest no longer lists included, still holds a SCIP ID.
    pub fn clear_all_scip(&mut self) -> Result<()> {
        let tx = self.conn.savepoint()?;
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

    /// Undoes the source's SCIP layer, in one savepoint: its occurrences,
    /// its SCIP edges and the symbols only SCIP found are deleted, and every
    /// symbol a SCIP ID replaced gets its tree-sitter ID back. An index run
    /// calls it for a source that no longer names a SCIP index.
    pub fn clear_scip(&mut self, source_id: i64) -> Result<()> {
        let tx = self.conn.savepoint()?;
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

/// The kind of edge an occurrence that isn't a definition gives: see
/// [`Store::write_scip`]. `line` is the text of the occurrence's last line,
/// when the file has it.
fn edge_kind(occ: &ScipOccurrence, line: Option<&str>, unit: ColumnUnit) -> &'static str {
    if occ.roles & ROLE_IMPORT != 0 {
        return "imports";
    }
    let callable = is_callable(&occ.symbol);
    // A term, `name.`, may hold a function: an arrow function, a lambda.
    let term = !callable && occ.symbol.ends_with('.');
    if !callable && !term {
        return "references";
    }
    let site = match (line, occ.end_column) {
        (Some(line), Some(column)) => super::definition::symbol_name(&occ.symbol)
            .and_then(|name| call_site(line, column, unit, &name)),
        _ => None,
    };
    match site {
        Some(true) => "calls",
        // An unreadable site keeps the rule the symbol alone gives.
        None if callable => "calls",
        _ => "references",
    }
}

/// Whether the occurrence of `name` that ends at `end_column` on `line` is a
/// call: the text after it, past whitespace, an optional generic argument
/// list (`::<T>` or `<T>`) and an optional `?.`, opens an argument list.
///
/// `end_column` counts in `unit`; with [`ColumnUnit::Unspecified`] it is
/// tried as UTF-16, UTF-8, then UTF-32. A unit is accepted only when the
/// text before the column ends with `name`, so `None` means no unit puts
/// the name there and the site can't be read.
fn call_site(line: &str, end_column: u32, unit: ColumnUnit, name: &str) -> Option<bool> {
    if name.is_empty() {
        return None;
    }
    let units: &[ColumnUnit] = match unit {
        ColumnUnit::Unspecified => &[ColumnUnit::Utf16, ColumnUnit::Utf8, ColumnUnit::Utf32],
        _ => std::slice::from_ref(&unit),
    };
    let end = units
        .iter()
        .filter_map(|&unit| byte_offset(line, end_column, unit))
        .find(|&end| line[..end].ends_with(name))?;
    let mut rest = line[end..].trim_start();
    if let Some(after) = rest.strip_prefix("::").unwrap_or(rest).strip_prefix('<') {
        rest = skip_generics(after)?.trim_start();
    }
    if let Some(after) = rest.strip_prefix("?.") {
        rest = after.trim_start();
    }
    Some(rest.starts_with('('))
}

/// The byte offset of `column` on `line` when counted in `unit`, or `None`
/// when the column is past the line or inside a character.
fn byte_offset(line: &str, column: u32, unit: ColumnUnit) -> Option<usize> {
    let column = column as usize;
    let width: fn(char) -> usize = match unit {
        ColumnUnit::Utf8 => {
            return line.is_char_boundary(column).then_some(column);
        }
        ColumnUnit::Utf16 => char::len_utf16,
        ColumnUnit::Utf32 | ColumnUnit::Unspecified => |_| 1,
    };
    let mut units = 0;
    for (byte, c) in line.char_indices() {
        if units == column {
            return Some(byte);
        }
        units += width(c);
    }
    (units == column).then_some(line.len())
}

/// The text after the `>` that closes a generic argument list whose `<` was
/// just before `text`, or `None` when the line ends first. The `>` of `->`
/// closes nothing.
fn skip_generics(text: &str) -> Option<&str> {
    let mut depth = 1;
    let mut previous = '<';
    for (byte, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if previous != '-' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[byte + 1..]);
                }
            }
            _ => {}
        }
        previous = c;
    }
    None
}

/// The source's code documents with their text, as `(ID, line count)` by
/// path.
fn stored_documents(tx: &Connection, source_id: i64) -> Result<HashMap<String, (i64, u32)>> {
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
fn undo_scip(tx: &Connection, source_id: i64) -> Result<()> {
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
    tx: &Connection,
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
    tx: &Connection,
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
                // The losing source's references stay, now inside the next
                // definition out, as if it had lost the ID before it wrote.
                tx.prepare_cached("DELETE FROM symbol WHERE id = ?1")?
                    .execute([&def.symbol])?;
                reassign(tx, &def.symbol, other)?;
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

/// One definition's line range in a file, and whether it may hold a
/// reference.
struct Range {
    id: String,
    start: u32,
    end: u32,
    holds_references: bool,
}

/// Kinds that name a value rather than a body of code.
const VALUE_KINDS: &str = "'field', 'property', 'variable', 'constant', 'term', 'enum_member',
                           'parameter', 'type_parameter'";

/// Every definition in the file, SCIP's and tree-sitter's. A value only
/// SCIP found (a field set in `__init__`, say) never holds a reference when
/// it lies inside another definition: the code on its line belongs to that
/// one. Tree-sitter's symbols hold references whatever their kind, so the
/// body of `const f = () => { … }` keeps its callees.
fn file_ranges(tx: &Connection, document_id: i64) -> Result<Vec<Range>> {
    let mut stmt = tx.prepare_cached(&format!(
        "SELECT id, start_line, end_line,
                provider = 'scip' AND local_id IS NULL AND kind IN ({VALUE_KINDS})
         FROM symbol WHERE document_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt.query_map([document_id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    let rows: Vec<(String, u32, u32, bool)> = rows.collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .iter()
        .map(|(id, start, end, value)| {
            let inside = rows
                .iter()
                .any(|(other, s, e, _)| other != id && s <= start && end <= e);
            Range {
                id: id.clone(),
                start: *start,
                end: *end,
                holds_references: !(*value && inside),
            }
        })
        .collect())
}

/// The definition holding an occurrence: a definition is inside the next
/// definition out; a reference, a recursive call included, is inside the
/// innermost one that may hold references.
fn innermost<'a>(ranges: &'a [Range], occ: &ScipOccurrence) -> Option<&'a str> {
    let definition = occ.roles & ROLE_DEFINITION != 0;
    ranges
        .iter()
        .filter(|r| {
            (if definition {
                r.id != occ.symbol
            } else {
                r.holds_references
            }) && r.start <= occ.start_line
                && occ.start_line <= r.end
        })
        // Shortest first, then the one starting later, then by ID.
        .min_by_key(|r| {
            (
                r.end.saturating_sub(r.start),
                std::cmp::Reverse(r.start),
                &r.id,
            )
        })
        .map(|r| r.id.as_str())
}

/// Moves the losing source's rows off the definition `lost`, once it is
/// deleted: each occurrence it held gets the innermost definition left
/// around it, and each edge from it becomes an edge from the definition now
/// holding its evidence line, keeping the earlier line when that edge
/// exists already. Edges from a definition come only from its own
/// document, so the two lines always share one.
fn reassign(tx: &Connection, lost: &str, loser: i64) -> Result<()> {
    const LOSER: &str = "(SELECT id FROM document WHERE source_id = ?2)";
    // `rowid` is SQLite's implicit row key: `occurrence` has no other.
    let held: Vec<(i64, i64, String, u32, u32)> = {
        let mut stmt = tx.prepare_cached(&format!(
            "SELECT rowid, document_id, symbol_id, start_line, roles FROM occurrence
             WHERE enclosing = ?1 AND document_id IN {LOSER} ORDER BY rowid"
        ))?;
        let rows = stmt.query_map(params![lost, loser], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let edges: Vec<(String, String, i64, u32)> = {
        let mut stmt = tx.prepare_cached(&format!(
            "DELETE FROM edge WHERE src = ?1 AND provider = 'scip' AND evidence_doc IN {LOSER}
             RETURNING dst, kind, evidence_doc, evidence_line"
        ))?;
        let rows = stmt.query_map(params![lost, loser], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // Each document's ranges, read once.
    let mut ranges: HashMap<i64, Vec<Range>> = HashMap::new();
    let mut holder = |document_id: i64, symbol: &str, line: u32, roles: u32| -> Result<_> {
        // `entry` looks the key up once; `?` can't run inside
        // `or_insert_with`, so the vacant arm reads the ranges itself.
        let doc_ranges = match ranges.entry(document_id) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(file_ranges(tx, document_id)?),
        };
        let occ = ScipOccurrence {
            symbol: symbol.to_owned(),
            start_line: line,
            end_line: line,
            end_column: None,
            roles,
        };
        Ok(innermost(doc_ranges, &occ).map(str::to_owned))
    };
    for (rowid, document_id, symbol, line, roles) in held {
        let enclosing = holder(document_id, &symbol, line, roles)?;
        tx.prepare_cached("UPDATE occurrence SET enclosing = ?2 WHERE rowid = ?1")?
            .execute(params![rowid, enclosing])?;
    }
    for (dst, kind, document_id, line) in edges {
        let Some(src) = holder(document_id, &dst, line, 0)? else {
            continue;
        };
        tx.prepare_cached(
            "INSERT INTO edge (src, dst, kind, provider, confidence, evidence_doc, evidence_line)
             VALUES (?1, ?2, ?3, 'scip', ?4, ?5, ?6)
             ON CONFLICT (src, dst, kind, provider)
             DO UPDATE SET evidence_line = min(evidence_line, excluded.evidence_line)",
        )?
        .execute(params![src, dst, kind, SCIP_CONFIDENCE, document_id, line])?;
    }
    Ok(())
}

/// Writes one document's occurrences, each with the innermost definition
/// holding it, and the edges its references give.
fn write_occurrences(
    tx: &Connection,
    document_id: i64,
    lines: u32,
    doc: &ScipDocument,
    summary: &mut ScipSummary,
) -> Result<()> {
    // Every definition in the file, SCIP's and tree-sitter's.
    let ranges = file_ranges(tx, document_id)?;
    let text: String = tx.query_row(
        "SELECT text FROM document WHERE id = ?1",
        [document_id],
        |row| row.get(0),
    )?;
    let text_lines = file_lines(&text);
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
    let mut edges: BTreeMap<(&str, &str, &str), u32> = BTreeMap::new();
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
        let enclosing = innermost(&ranges, occ);
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
            // The column counts on the range's last line, unless the file
            // ends first.
            let line = (end == occ.end_line)
                .then(|| text_lines.get(end as usize - 1).copied())
                .flatten();
            let kind = edge_kind(occ, line, doc.encoding);
            edges
                .entry((from, &occ.symbol, kind))
                .or_insert(occ.start_line);
        }
    }
    let mut edge = tx.prepare_cached(
        "INSERT OR IGNORE INTO edge (src, dst, kind, provider, confidence, evidence_doc,
                                     evidence_line)
         VALUES (?1, ?2, ?3, 'scip', ?4, ?5, ?6)",
    )?;
    for ((from, to, kind), line) in edges {
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
    pub type Def<'a> = (&'a str, &'a str, &'a str, (u32, u32));

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

    /// A store holding one source, `app`, with these `(path, text)` files
    /// and no tree-sitter symbols in them, and the source's ID.
    pub fn files(files: &[(&'static str, &'static str)]) -> (tempfile::TempDir, Store, i64) {
        let (dir, mut store, id) = empty();
        for &(path, text) in files {
            store
                .write_document(id, &document(path, text, &[]))
                .unwrap();
        }
        store.set_indexed_at(30).unwrap();
        (dir, store, id)
    }

    /// A store holding the source `app` with one file whose tree-sitter
    /// definitions are `defs`, and the source's ID.
    pub fn file(
        path: &'static str,
        text: &'static str,
        defs: &[Def<'static>],
    ) -> (tempfile::TempDir, Store, i64) {
        let (dir, mut store, id) = empty();
        store
            .write_document(id, &document(path, text, defs))
            .unwrap();
        store.set_indexed_at(30).unwrap();
        (dir, store, id)
    }

    /// A store holding the source `app` with no documents yet.
    fn empty() -> (tempfile::TempDir, Store, i64) {
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
        (dir, store, id)
    }

    /// The crate as tree-sitter alone indexes it, and its source's ID.
    pub fn app() -> (tempfile::TempDir, Store, i64) {
        let (dir, mut store, id) = empty();
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
            end_column: None,
            roles,
        }
    }

    pub const STRING_NEW: &str = "rust-analyzer cargo alloc https://github.com/rust-lang/rust/library/alloc string/impl#[String]new().";
    pub const IS_EMPTY: &str = "rust-analyzer cargo core https://github.com/rust-lang/rust/library/core str/impl#[str]is_empty().";

    /// The crate's SCIP index, as rust-analyzer writes it (lines from 1).
    pub fn index() -> Vec<ScipDocument> {
        let d = ROLE_DEFINITION;
        let mut lib = ScipDocument {
            encoding: Default::default(),
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
            encoding: Default::default(),
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
    use super::fixture::{app, file, files, index, local, scip};
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
        // inside the file's. Line 5's `Client` has the import role, so it
        // is an `imports` edge.
        assert_eq!(
            edges(&scip("crate/")),
            [
                (scip("client/"), "references".into(), 3),
                (scip("client/Client#"), "imports".into(), 5),
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
            end_column: None,
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
            end_column: None,
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
            encoding: Default::default(),
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
            encoding: Default::default(),
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
                end_column: None,
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

    /// `other`'s login, SCIP-only inside its SCIP-only `outer` module, calls
    /// `check` on line 3; `outer` calls it again on line 4.
    fn nested_other_source(store: &mut Store) -> (i64, Vec<ScipDocument>) {
        let (id, _) = other_source(store);
        store
            .write_document(
                id,
                &crate::DocumentRecord {
                    path: "src/outer.rs",
                    language: Some("rust"),
                    content_hash: b"outer",
                    size_bytes: 5,
                    dirty: false,
                    text: Some("mod outer {\n  // login\n  check\n  check\n}\n"),
                    chunks: vec![],
                },
            )
            .unwrap();
        let reference = |line| ScipOccurrence {
            symbol: scip("check()."),
            start_line: line,
            end_line: line,
            end_column: None,
            roles: 0,
        };
        let docs = vec![ScipDocument {
            encoding: Default::default(),
            path: "src/outer.rs".into(),
            definitions: vec![
                defined(&scip("outer/"), "outer", "module", (1, 5)),
                ScipDefinition {
                    line: 2,
                    ..defined(&scip("login()."), "login", "function", (2, 3))
                },
            ],
            occurrences: vec![
                ScipOccurrence {
                    symbol: scip("login()."),
                    roles: ROLE_DEFINITION,
                    ..reference(2)
                },
                reference(3),
                reference(4),
            ],
        }];
        (id, docs)
    }

    /// The source's occurrences as `(symbol, line, enclosing)` and the edges
    /// its documents give as `(src, dst, kind, line)`.
    #[allow(clippy::type_complexity)]
    fn source_rows(
        store: &Store,
        source_id: i64,
    ) -> (
        Vec<(String, u32, Option<String>)>,
        Vec<(String, String, String, u32)>,
    ) {
        const DOCS: &str = "(SELECT id FROM document WHERE source_id = ?1)";
        let mut stmt = store
            .conn
            .prepare(&format!(
                "SELECT symbol_id, start_line, enclosing FROM occurrence
                 WHERE document_id IN {DOCS} ORDER BY 1, 2, 3"
            ))
            .unwrap();
        let occurrences = stmt
            .query_map([source_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut stmt = store
            .conn
            .prepare(&format!(
                "SELECT src, dst, kind, evidence_line FROM edge
                 WHERE evidence_doc IN {DOCS} ORDER BY 1, 2, 3"
            ))
            .unwrap();
        let edges = stmt
            .query_map([source_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        (occurrences, edges)
    }

    #[test]
    fn taking_an_id_over_leaves_the_loser_as_if_it_had_lost_first() {
        // app first: `other` loses login while it writes.
        let (_dir, mut store, app_id) = app();
        let (other, docs) = nested_other_source(&mut store);
        store.write_scip(app_id, &index()).unwrap();
        store.write_scip(other, &docs).unwrap();
        let lost_first = source_rows(&store, other);

        // `other` first: app takes login over afterwards.
        let (_dir, mut store, app_id) = app();
        let (other, docs) = nested_other_source(&mut store);
        store.write_scip(other, &docs).unwrap();
        let summary = store.write_scip(app_id, &index()).unwrap();
        assert_eq!(summary.taken_over, 1);
        let taken_over = source_rows(&store, other);

        let outer = Some(scip("outer/"));
        assert_eq!(
            lost_first.0,
            [
                (scip("check()."), 3, outer.clone()),
                (scip("check()."), 4, outer.clone()),
                (scip("login()."), 2, outer.clone()),
            ]
        );
        assert_eq!(
            lost_first.1,
            [(scip("outer/"), scip("check()."), "calls".to_owned(), 3)]
        );
        assert_eq!(taken_over, lost_first);
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

    #[test]
    fn a_call_site_opens_an_argument_list_after_the_name() {
        // `end` is the column just past `name` in UTF-8, the unit these
        // ASCII lines share with every other.
        let call = |line: &str, name: &str, unit| {
            let end = line.find(name).unwrap() + name.len();
            call_site(line, end as u32, unit, name)
        };
        for unit in [ColumnUnit::Utf8, ColumnUnit::Unspecified] {
            // Rust
            assert_eq!(call("    check(token)", "check", unit), Some(true));
            assert_eq!(call("    check (token)", "check", unit), Some(true));
            assert_eq!(call("    Self::new()", "new", unit), Some(true));
            assert_eq!(call("    x.check::<T>(a)", "check", unit), Some(true));
            assert_eq!(call("    x.f::<fn() -> u8>(a)", "f", unit), Some(true));
            assert_eq!(
                call("use crate::client::check;", "check", unit),
                Some(false)
            );
            assert_eq!(call("    v.map(Self::check)", "check", unit), Some(false));
            // Python
            assert_eq!(
                call(
                    "        parser.UnparsedEntityDeclHandler = self.unparsed_entity_decl",
                    "unparsed_entity_decl",
                    unit
                ),
                Some(false)
            );
            assert_eq!(
                call("        super().__init__(x)", "__init__", unit),
                Some(true)
            );
            // TypeScript
            assert_eq!(call("  return f(a);", "f", unit), Some(true));
            assert_eq!(call("  return f<T>(a);", "f", unit), Some(true));
            assert_eq!(call("  return f?.(a);", "f", unit), Some(true));
            assert_eq!(call(r#"import { f } from "./a";"#, "f", unit), Some(false));
            assert_eq!(call("  <b onClick={f}>", "f", unit), Some(false));
        }
    }

    #[test]
    fn a_call_site_reads_columns_in_the_documents_unit() {
        // `é` is two UTF-8 bytes but one UTF-16 or UTF-32 unit, so `f` ends
        // at byte 7 and at code unit 6.
        let line = r#""é"; f(x)"#;
        assert_eq!(call_site(line, 7, ColumnUnit::Utf8, "f"), Some(true));
        assert_eq!(call_site(line, 6, ColumnUnit::Utf16, "f"), Some(true));
        assert_eq!(call_site(line, 6, ColumnUnit::Utf32, "f"), Some(true));
        assert_eq!(call_site(line, 6, ColumnUnit::Unspecified, "f"), Some(true));
        assert_eq!(call_site(line, 7, ColumnUnit::Unspecified, "f"), Some(true));
        // `𝑥` is one UTF-32 unit, two UTF-16 units and four bytes.
        let line = "𝑥; f(x)";
        assert_eq!(call_site(line, 4, ColumnUnit::Utf32, "f"), Some(true));
        assert_eq!(call_site(line, 5, ColumnUnit::Utf16, "f"), Some(true));
        assert_eq!(call_site(line, 7, ColumnUnit::Utf8, "f"), Some(true));
        assert_eq!(call_site(line, 4, ColumnUnit::Unspecified, "f"), Some(true));
    }

    #[test]
    fn a_column_that_does_not_end_the_name_is_unknown() {
        assert_eq!(call_site("f(x)", 9, ColumnUnit::Utf8, "f"), None);
        assert_eq!(call_site("f(x)", 9, ColumnUnit::Unspecified, "f"), None);
        assert_eq!(call_site("g(f)", 1, ColumnUnit::Unspecified, "f"), None);
        // Inside a multi-byte character: no unit ends `f` there.
        assert_eq!(call_site("é f(x)", 1, ColumnUnit::Utf8, "f"), None);
        assert_eq!(call_site("f(x)", 1, ColumnUnit::Utf8, ""), None);
    }

    /// The SCIP edges from `src`, as `(dst, kind, evidence line)`, sorted.
    fn edges_from(store: &Store, src: &str) -> Vec<(String, String, u32)> {
        let mut stmt = store
            .conn
            .prepare(
                "SELECT dst, kind, evidence_line FROM edge WHERE src = ?1 AND provider = 'scip'
                 ORDER BY dst, kind",
            )
            .unwrap();
        stmt.query_map([src], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// A definition whose name is on its first line.
    fn defined(symbol: &str, name: &str, kind: &str, lines: (u32, u32)) -> ScipDefinition {
        ScipDefinition {
            symbol: symbol.into(),
            display_name: name.into(),
            kind: kind.into(),
            line: lines.0,
            start_line: lines.0,
            end_line: lines.1,
        }
    }

    /// An occurrence on one line that ends at `end_column`.
    fn site(symbol: &str, line: u32, end_column: u32, roles: u32) -> ScipOccurrence {
        ScipOccurrence {
            symbol: symbol.into(),
            start_line: line,
            end_line: line,
            end_column: Some(end_column),
            roles,
        }
    }

    /// Where `name`'s first occurrence on `line` ends, in bytes.
    fn end_of(line: &str, name: &str) -> u32 {
        (line.find(name).unwrap() + name.len()) as u32
    }

    #[test]
    fn a_call_to_an_arrow_function_is_a_call() {
        const G: &str = "function g() { return f(); }";
        let (_dir, mut store, id) = files(&[(
            "src/a.ts",
            "const f = () => 1;\nfunction g() { return f(); }\n",
        )]);
        let f = "scip-typescript npm app 0.1.0 src/`a.ts`/f.";
        let g = "scip-typescript npm app 0.1.0 src/`a.ts`/g().";
        let doc = ScipDocument {
            path: "src/a.ts".into(),
            definitions: vec![
                defined(f, "f", "variable", (1, 1)),
                defined(g, "g", "function", (2, 2)),
            ],
            occurrences: vec![
                site(f, 1, 7, ROLE_DEFINITION),
                site(g, 2, 10, ROLE_DEFINITION),
                site(f, 2, end_of(G, "return f"), 0),
            ],
            encoding: ColumnUnit::Utf16,
        };
        store.write_scip(id, &[doc]).unwrap();
        assert_eq!(edges_from(&store, g), [(f.into(), "calls".into(), 2)]);
        let callers = store
            .graph(&crate::GraphQuery {
                symbol: f,
                direction: crate::Direction::Callers,
                depth: 1,
                min_confidence: 0,
                allow_stale: false,
            })
            .unwrap();
        let ids: Vec<&str> = callers.results.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, [g]);
    }

    #[test]
    fn an_import_is_an_imports_edge_and_a_use_line_a_reference() {
        const IMPORT: &str = r#"import { f, g } from "./a";"#;
        const USE: &str = "use client::check;";
        const RUN: &str = "pub fn run() { check() }";
        let ts = Box::leak(format!("{IMPORT}\n").into_boxed_str());
        let rs = Box::leak(format!("{USE}\n{RUN}\n").into_boxed_str());
        let (_dir, mut store, id) = files(&[("src/b.ts", ts), ("src/lib.rs", rs)]);
        let module = "scip-typescript npm app 0.1.0 src/`b.ts`/";
        let f = "scip-typescript npm app 0.1.0 src/`a.ts`/f.";
        let g = "scip-typescript npm app 0.1.0 src/`a.ts`/g().";
        let krate = scip("crate/");
        let run = scip("run().");
        let check = scip("client/check().");
        let docs = [
            ScipDocument {
                path: "src/b.ts".into(),
                definitions: vec![defined(module, "b.ts", "module", (1, 1))],
                occurrences: vec![
                    site(f, 1, end_of(IMPORT, "{ f"), 2),
                    site(g, 1, end_of(IMPORT, ", g"), 2),
                ],
                encoding: ColumnUnit::Utf16,
            },
            ScipDocument {
                path: "src/lib.rs".into(),
                definitions: vec![
                    defined(&krate, "crate", "module", (1, 2)),
                    defined(&run, "run", "function", (2, 2)),
                ],
                occurrences: vec![
                    site(&check, 1, end_of(USE, "check"), 0),
                    site(&check, 2, end_of(RUN, "check"), 0),
                ],
                encoding: ColumnUnit::Utf8,
            },
        ];
        store.write_scip(id, &docs).unwrap();
        // The import role names `g` without calling it.
        assert_eq!(
            edges_from(&store, module),
            [
                (f.into(), "imports".into(), 1),
                (g.into(), "imports".into(), 1),
            ]
        );
        // rust-analyzer sets no import role; the site shows no call.
        assert_eq!(
            edges_from(&store, &krate),
            [(check.clone(), "references".into(), 1)]
        );
        assert_eq!(edges_from(&store, &run), [(check, "calls".into(), 2)]);
    }

    #[test]
    fn a_handler_assignment_references_and_a_call_calls_from_one_definition() {
        const ASSIGN: &str = "        parser.H = self.decl";
        const CALL: &str = "        self.decl(1)";
        let text = Box::leak(
            format!(
                "class P:\n    def start(self, parser):\n{ASSIGN}\n{CALL}\n    \
                 def decl(self, x):\n        pass\n"
            )
            .into_boxed_str(),
        );
        let (_dir, mut store, id) = files(&[("src/p.py", text)]);
        let class = "scip-python python app 0.1.0 `p`/P#";
        let start = "scip-python python app 0.1.0 `p`/P#start().";
        let decl = "scip-python python app 0.1.0 `p`/P#decl().";
        let doc = ScipDocument {
            path: "src/p.py".into(),
            definitions: vec![
                defined(class, "P", "class", (1, 6)),
                defined(start, "start", "method", (2, 4)),
                defined(decl, "decl", "method", (5, 6)),
            ],
            occurrences: vec![
                site(decl, 3, end_of(ASSIGN, "decl"), 8),
                site(decl, 4, end_of(CALL, "decl"), 8),
            ],
            encoding: ColumnUnit::Unspecified,
        };
        store.write_scip(id, &[doc]).unwrap();
        // One row per kind, each with its own first line.
        assert_eq!(
            edges_from(&store, start),
            [
                (decl.into(), "calls".into(), 4),
                (decl.into(), "references".into(), 3),
            ]
        );
    }

    #[test]
    fn a_scip_only_field_never_encloses_a_reference() {
        const LINE: &str = "        self._regex, self._converters = _route_to_regex(route)";
        let text = Box::leak(
            format!(
                "class RoutePattern:\n    def __init__(self, route):\n{LINE}\n\n\
                 def _route_to_regex(route):\n    return 1, 2\n"
            )
            .into_boxed_str(),
        );
        let (_dir, mut store, id) = files(&[("src/r.py", text)]);
        let class = "scip-python python app 0.1.0 `r`/RoutePattern#";
        let init = "scip-python python app 0.1.0 `r`/RoutePattern#__init__().";
        let regex = "scip-python python app 0.1.0 `r`/RoutePattern#_regex.";
        let to_regex = "scip-python python app 0.1.0 `r`/_route_to_regex().";
        let doc = ScipDocument {
            path: "src/r.py".into(),
            definitions: vec![
                defined(class, "RoutePattern", "class", (1, 3)),
                defined(init, "__init__", "method", (2, 3)),
                defined(regex, "_regex", "field", (3, 3)),
                defined(to_regex, "_route_to_regex", "function", (5, 6)),
            ],
            occurrences: vec![
                site(regex, 3, end_of(LINE, "_regex"), ROLE_DEFINITION),
                site(to_regex, 3, end_of(LINE, "_route_to_regex"), 0),
            ],
            encoding: ColumnUnit::Utf8,
        };
        store.write_scip(id, &[doc]).unwrap();
        assert_eq!(edges_from(&store, regex), []);
        assert_eq!(
            edges_from(&store, init),
            [(to_regex.into(), "calls".into(), 3)]
        );
        let enclosing: Option<String> = store
            .conn
            .query_row(
                "SELECT enclosing FROM occurrence WHERE symbol_id = ?1",
                [to_regex],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(enclosing.as_deref(), Some(init));
    }

    #[test]
    fn a_parameter_does_not_make_a_name_ambiguous() {
        let (_dir, mut store, id) = files(&[(
            "a.py",
            "def f(x):\n    return x\n\ndef g(f):\n    return f\n",
        )]);
        let f = "scip-python python app 0.1.0 a/f().";
        let x = "scip-python python app 0.1.0 a/f().(x)";
        let g = "scip-python python app 0.1.0 a/g().";
        let g_f = "scip-python python app 0.1.0 a/g().(f)";
        let doc = ScipDocument {
            path: "a.py".into(),
            definitions: vec![
                defined(f, "f", "function", (1, 2)),
                defined(x, "x", "parameter", (1, 1)),
                defined(g, "g", "function", (4, 5)),
                defined(g_f, "f", "parameter", (4, 4)),
            ],
            occurrences: vec![
                site(f, 1, 5, ROLE_DEFINITION),
                site(x, 1, 7, ROLE_DEFINITION),
                site(g, 4, 5, ROLE_DEFINITION),
                site(g_f, 4, 7, ROLE_DEFINITION),
            ],
            encoding: ColumnUnit::Utf8,
        };
        store.write_scip(id, &[doc]).unwrap();
        let walk = |symbol| {
            store.graph(&crate::GraphQuery {
                symbol,
                direction: crate::Direction::Callers,
                depth: 1,
                min_confidence: 0,
                allow_stale: false,
            })
        };
        assert_eq!(walk("f").unwrap().id, f, "the parameter `f` is left out");
        // A parameter still resolves by its ID.
        assert_eq!(walk(g_f).unwrap().id, g_f);
        let err = walk("X").unwrap_err();
        let Error::NoSuchSymbol { candidates, .. } = &err else {
            panic!("{err:?}");
        };
        assert!(candidates.is_empty(), "no parameter names: {candidates:?}");
    }

    #[test]
    fn a_dotted_name_finds_a_trait_impl_method() {
        const FOO: &str = "pub struct Foo;\n\nimpl IntoIterator for Foo {\n    fn into_iter(self) -> Iter {\n        helper()\n    }\n}\n\nfn helper() -> Iter { todo!() }\n";
        let (_dir, mut store, id) = file(
            "src/foo.rs",
            FOO,
            &[
                ("Foo#", "Foo", "struct", (1, 1)),
                ("Foo#into_iter().", "into_iter", "method", (4, 6)),
                ("helper().", "helper", "function", (9, 9)),
            ],
        );
        let into_iter = scip("impl#[Foo][IntoIterator]into_iter().");
        let helper = scip("helper().");
        let doc = ScipDocument {
            path: "src/foo.rs".into(),
            definitions: vec![
                defined(&scip("Foo#"), "Foo", "struct", (1, 1)),
                defined(&into_iter, "into_iter", "method", (4, 6)),
                defined(&helper, "helper", "function", (9, 9)),
            ],
            occurrences: vec![
                site(&scip("Foo#"), 1, 14, ROLE_DEFINITION),
                site(&into_iter, 4, 16, ROLE_DEFINITION),
                site(&helper, 5, 14, 0),
                site(&helper, 9, 9, ROLE_DEFINITION),
            ],
            encoding: ColumnUnit::Utf8,
        };
        store.write_scip(id, &[doc]).unwrap();
        let found = store.definition("Foo.into_iter", false).unwrap();
        assert_eq!(found.total, 1);
        assert_eq!(found.results[0].id, into_iter);
        let callees = store
            .graph(&crate::GraphQuery {
                symbol: "Foo.into_iter",
                direction: crate::Direction::Callees,
                depth: 1,
                min_confidence: 0,
                allow_stale: false,
            })
            .unwrap();
        assert_eq!(callees.id, into_iter);
        assert_eq!(
            edges_from(&store, &into_iter),
            [(helper, "calls".into(), 5)]
        );
    }
}
