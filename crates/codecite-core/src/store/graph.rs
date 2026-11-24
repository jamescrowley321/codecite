//! The graph tools: where a symbol is referenced, and what calls it or what
//! it calls, from the occurrences and edges SCIP indexes gave.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{OptionalExtension, params};
use schemars::JsonSchema;
use serde::Serialize;

use super::Store;
use super::definition::{names_in_file, qualified, rank};
use super::read::{byte_prefix, copy_instruction, file_lines};
use super::scip::ROLE_DEFINITION;
use super::search::{MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES, code_citation};
use crate::{Error, Result, Role};

/// The deepest `callers` and `callees` walk.
pub const MAX_GRAPH_DEPTH: u32 = 10;

/// The highest confidence on the ladder: 3, exact.
pub const MAX_CONFIDENCE: u8 = 3;

/// The SCIP role bits a reference reports, with their names.
const ROLE_NAMES: [(u32, &str); 5] = [
    (0x2, "import"),
    (0x4, "write"),
    (0x8, "read"),
    (0x10, "generated"),
    (0x20, "test"),
];

/// Every reference to one symbol: what `codecite refs --json` prints and
/// the MCP `references` tool returns.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct References {
    /// The symbol as asked for.
    pub symbol: String,
    /// The SCIP symbol it resolved to.
    pub id: String,
    /// How many lines reference it, which may be more than `results`
    /// holds.
    pub total: usize,
    /// The references, at most 50 (`MAX_SEARCH_LIMIT`): source first, then
    /// docs, examples and tests, each in citation order.
    pub results: Vec<Reference>,
}

/// One line that references a symbol.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Reference {
    /// The reference's line: `repo@sha:path#L12`.
    pub citation: String,
    /// The innermost definition holding the reference, as a symbol ID.
    /// Left out at a file's top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enclosing: Option<String>,
    /// What the reference does, as the indexer reports it: `import`,
    /// `write`, `read`, `generated`, `test`. Left out when it reports none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub access: Vec<&'static str>,
    /// What the reference's file is for: `source`, `test`, `example` or
    /// `docs`.
    pub role: &'static str,
    /// The reference's first line, at most 4 KiB of it.
    pub text: String,
    /// Whether `text` was cut.
    pub truncated: bool,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    pub license_class: String,
    /// For an `approach_only` source, the instruction not to copy its code,
    /// as search and grep results carry it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    /// When the source was last indexed.
    pub indexed_at: String,
    /// Whether the file differed from the cited commit when it was indexed.
    pub dirty: bool,
    /// The providers that found the reference: `scip`.
    pub providers: Vec<String>,
    /// How sure they are: 3 exact, 2 extracted, 1 inferred, 0 ambiguous.
    pub confidence: u8,
}

/// Which way a graph walk goes along `calls` edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// To the symbols that call this one.
    Callers,
    /// To the symbols this one calls.
    Callees,
}

/// A graph walk to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphQuery<'a> {
    /// The symbol to start from: a SCIP ID, a name, or a dotted name.
    pub symbol: &'a str,
    /// Callers or callees.
    pub direction: Direction,
    /// How many edges away to go: 1 for direct callers or callees, at most
    /// [`MAX_GRAPH_DEPTH`].
    pub depth: u32,
    /// The lowest confidence an edge may have to be followed.
    pub min_confidence: u8,
    /// Answer from an index older than its maximum age.
    pub allow_stale: bool,
}

/// The symbols a walk reached: what `codecite graph callers|callees
/// --json` prints and the MCP `callers` and `callees` tools return.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Graph {
    /// The symbol as asked for.
    pub symbol: String,
    /// The SCIP symbol it resolved to.
    pub id: String,
    /// `callers` or `callees`.
    pub direction: Direction,
    /// How many edges away the walk went.
    pub depth: u32,
    /// How many symbols it reached, which may be more than `results`
    /// holds.
    pub total: usize,
    /// The symbols, nearest first, at most 50 (`MAX_SEARCH_LIMIT`).
    pub results: Vec<GraphNode>,
}

/// One symbol a graph walk reached.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct GraphNode {
    /// Its SCIP symbol ID.
    pub id: String,
    /// Its name as written, or its last descriptor's for a symbol defined
    /// outside the corpus.
    pub name: String,
    /// What kind of definition it is, when the corpus defines it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// How many edges from the asked symbol: 1 for a direct caller or
    /// callee.
    pub distance: u32,
    /// Its definition's lines, when the corpus defines it. `read` it for
    /// the text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    /// The symbol one edge nearer the asked one, which this one calls (for
    /// callers) or is called by (for callees).
    pub via: String,
    /// The line of that call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_site: Option<String>,
    /// The providers that found that call.
    pub providers: Vec<String>,
    /// How sure the best of them is: 3 exact, 2 extracted, 1 inferred, 0
    /// ambiguous.
    pub confidence: u8,
    /// How agents may use its definition's source, when the corpus defines
    /// it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_class: Option<String>,
    /// When its definition's source was last indexed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<String>,
    /// The source its definition is in, when that isn't the asked symbol's
    /// (its definition's, else its first call site's): SCIP IDs are global,
    /// so another source indexing the same package can hold the definitions
    /// a walk reaches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defined_in: Option<String>,
}

/// Where a definition is, to sort by: its source, path and first line.
type Place = (String, String, u32);

/// A stored reference before it is cut down to the results.
struct Site {
    role: Role,
    sort: (String, String, u32),
    document_id: i64,
    start_line: u32,
    reference: Reference,
}

impl Store {
    /// Every line that references `symbol`, as SCIP found it, each cited.
    ///
    /// `symbol` is a SCIP ID, or a name or dotted name as
    /// [`Store::definition`] takes it, matched against every symbol. A name
    /// that matches several symbols is [`Error::AmbiguousSymbol`] with their
    /// IDs, one that matches none [`Error::NoSuchSymbol`], and one whose
    /// definition no SCIP index covers [`Error::NoGraph`]. Definitions
    /// aren't references; at most [`MAX_SEARCH_LIMIT`] are returned, by role
    /// and then citation.
    pub fn references(
        &self,
        symbol: &str,
        min_confidence: u8,
        allow_stale: bool,
    ) -> Result<References> {
        let id = self.graph_symbol(symbol, Walk::References, allow_stale)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT o.start_line, o.roles, o.enclosing, o.provider, o.confidence, d.id, d.path,
                    d.dirty, s.name, s.kind, s.revision, s.license_class, s.indexed_at
             FROM occurrence o
             JOIN document d ON d.id = o.document_id
             JOIN source s ON s.id = d.source_id
             WHERE o.symbol_id = ?1 AND (o.roles & ?2) = 0 AND o.confidence >= ?3
               AND s.is_current",
        )?;
        let rows = stmt.query_map(params![id, ROLE_DEFINITION, min_confidence], |row| {
            let line: u32 = row.get(0)?;
            let roles: u32 = row.get(1)?;
            let path: String = row.get(6)?;
            let source: String = row.get(8)?;
            let source_kind: String = row.get(9)?;
            let revision: String = row.get(10)?;
            let role = Role::of(&path, &source_kind);
            let reference = Reference {
                citation: code_citation(&source, &revision, &path, line, line),
                enclosing: row.get(2)?,
                access: ROLE_NAMES
                    .iter()
                    .filter(|(bit, _)| roles & bit != 0)
                    .map(|&(_, name)| name)
                    .collect(),
                role: role.as_str(),
                text: String::new(),
                truncated: false,
                instruction: copy_instruction(&row.get::<_, String>(11)?),
                license_class: row.get(11)?,
                indexed_at: row.get(12)?,
                dirty: row.get(7)?,
                providers: vec![row.get(3)?],
                confidence: row.get(4)?,
            };
            Ok(Site {
                role,
                sort: (source, path, line),
                document_id: row.get(5)?,
                start_line: line,
                reference,
            })
        })?;
        let mut sites: Vec<Site> = rows.collect::<rusqlite::Result<_>>()?;
        sites.sort_by(|a, b| (rank(a.role), &a.sort).cmp(&(rank(b.role), &b.sort)));
        // Two references on one line are one result, as a citation names
        // lines, with what either does.
        sites.dedup_by(|later, kept| {
            let same = later.document_id == kept.document_id && later.start_line == kept.start_line;
            if same {
                for access in &later.reference.access {
                    if !kept.reference.access.contains(access) {
                        kept.reference.access.push(access);
                    }
                }
                kept.reference
                    .access
                    .sort_by_key(|a| ROLE_NAMES.iter().position(|(_, n)| n == a));
                kept.reference.confidence =
                    kept.reference.confidence.max(later.reference.confidence);
            }
            same
        });
        let total = sites.len();
        let limit = usize::try_from(MAX_SEARCH_LIMIT).unwrap_or(usize::MAX);
        let mut texts: HashMap<i64, String> = HashMap::new();
        let mut results = Vec::new();
        for site in sites.into_iter().take(limit) {
            if let std::collections::hash_map::Entry::Vacant(entry) = texts.entry(site.document_id)
            {
                let text: Option<String> = self.conn.query_row(
                    "SELECT text FROM document WHERE id = ?1",
                    [site.document_id],
                    |row| row.get(0),
                )?;
                entry.insert(text.unwrap_or_default());
            }
            let lines = file_lines(&texts[&site.document_id]);
            let line = usize::try_from(site.start_line)
                .ok()
                .and_then(|n| lines.get(n.checked_sub(1)?))
                .copied()
                .unwrap_or_default();
            let (text, truncated) = byte_prefix(line, MAX_SNIPPET_BYTES);
            let mut reference = site.reference;
            reference.text = text.to_owned();
            reference.truncated = truncated;
            results.push(reference);
        }
        Ok(References {
            symbol: symbol.into(),
            id,
            total,
            results,
        })
    }

    /// The symbols that call `query.symbol` (or that it calls), out to
    /// `query.depth` edges, each at its shortest distance.
    ///
    /// The walk is a recursive query over `calls` edges with at least
    /// `query.min_confidence`; a cycle ends where the depth does, and the
    /// asked symbol is never in the results. Each result names the symbol
    /// one edge nearer (`via`) and that call's line. The symbol resolves as
    /// [`Store::references`] resolves it, with the same errors. At most
    /// [`MAX_SEARCH_LIMIT`] are returned, nearest first.
    pub fn graph(&self, query: &GraphQuery<'_>) -> Result<Graph> {
        let walk = match query.direction {
            Direction::Callers => Walk::Callers,
            Direction::Callees => Walk::Callees,
        };
        let id = self.graph_symbol(query.symbol, walk, query.allow_stale)?;
        let depth = query.depth.clamp(1, MAX_GRAPH_DEPTH);
        // `near` is the column the walk comes from, `far` the one it goes to.
        let (near, far) = match query.direction {
            Direction::Callers => ("dst", "src"),
            Direction::Callees => ("src", "dst"),
        };
        let distances: HashMap<String, u32> = {
            let mut stmt = self.conn.prepare_cached(&format!(
                "WITH RECURSIVE walk(id, distance) AS (
                   VALUES (?1, 0)
                   UNION
                   SELECT e.{far}, w.distance + 1 FROM walk w
                   JOIN edge e ON e.{near} = w.id
                   WHERE e.kind = 'calls' AND e.confidence >= ?3 AND w.distance < ?2
                 )
                 SELECT id, min(distance) FROM walk GROUP BY id"
            ))?;
            let rows = stmt.query_map(params![id, depth, query.min_confidence], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let total = distances.values().filter(|&&d| d > 0).count();
        // Only the nearest distances that fill the results are looked up:
        // every symbol at a distance, since they sort by where they are
        // defined, until there are enough.
        let limit = usize::try_from(MAX_SEARCH_LIMIT).unwrap_or(usize::MAX);
        let mut by_distance: Vec<(u32, &String)> = distances
            .iter()
            .filter(|&(_, &d)| d > 0)
            .map(|(id, &d)| (d, id))
            .collect();
        by_distance.sort();
        let reach = by_distance
            .get(limit.saturating_sub(1))
            .map_or(u32::MAX, |&(d, _)| d);
        let candidates: Vec<(u32, &String)> = by_distance
            .into_iter()
            .filter(|&(d, _)| d <= reach)
            .collect();
        // Where each candidate is defined, in one query, to rank them; only
        // the ones kept are looked up in full.
        let places: HashMap<String, Place> = {
            let ids: Vec<&String> = candidates.iter().map(|&(_, id)| id).collect();
            let ids = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into());
            let mut stmt = self.conn.prepare_cached(
                "SELECT sym.id, s.name, d.path, sym.start_line FROM json_each(?1) j
                 JOIN symbol sym ON sym.id = j.value
                 JOIN document d ON d.id = sym.document_id
                 JOIN source s ON s.id = d.source_id
                 WHERE s.is_current",
            )?;
            let rows = stmt.query_map([ids], |row| {
                Ok((row.get(0)?, (row.get(1)?, row.get(2)?, row.get(3)?)))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut ranked: Vec<(u32, Option<&Place>, &String)> = candidates
            .into_iter()
            .map(|(d, id)| (d, places.get(id), id))
            .collect();
        // Nearest first; at one distance, the corpus's definitions in file
        // and line order, then symbols defined outside it, by ID.
        ranked.sort_by(|a, b| (a.0, a.1.is_none(), a.1, a.2).cmp(&(b.0, b.1.is_none(), b.1, b.2)));
        let home = self.home_source(&id)?;
        let mut nodes = Vec::new();
        for (distance, _, node) in ranked.into_iter().take(limit) {
            nodes.push(self.graph_node(
                node,
                distance,
                &distances,
                (near, far),
                query.min_confidence,
                home.as_deref(),
            )?);
        }
        Ok(Graph {
            symbol: query.symbol.into(),
            id,
            direction: query.direction,
            depth,
            total,
            results: nodes,
        })
    }

    /// The source the asked symbol `id` belongs to: its definition's, or,
    /// for a symbol defined outside the corpus, the source of its first
    /// call site (by source, path and line). `None` when no current source
    /// has either.
    fn home_source(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT s.name FROM symbol sym
                   JOIN document d ON d.id = sym.document_id
                   JOIN source s ON s.id = d.source_id
                   WHERE sym.id = ?1 AND s.is_current
                 UNION ALL
                 SELECT name FROM (
                   SELECT s.name FROM edge e
                   JOIN document d ON d.id = e.evidence_doc
                   JOIN source s ON s.id = d.source_id
                   WHERE e.dst = ?1 AND e.kind = 'calls' AND s.is_current
                   ORDER BY s.name, d.path, e.evidence_line
                 )
                 LIMIT 1",
            )?
            .query_row([id], |row| row.get(0))
            .optional()?)
    }

    /// One reached symbol, with its definition when the corpus has one and
    /// the call that links it to a symbol one edge nearer. `home` is the
    /// asked symbol's source (see [`Store::home_source`]).
    fn graph_node(
        &self,
        node: &str,
        distance: u32,
        distances: &HashMap<String, u32>,
        (near, far): (&str, &str),
        min_confidence: u8,
        home: Option<&str>,
    ) -> Result<GraphNode> {
        // The edges between `node` and the symbols one step nearer, the
        // nearest by ID first, its providers' rows together.
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT e.{near}, e.provider, e.confidence, e.evidence_doc, e.evidence_line
             FROM edge e
             WHERE e.{far} = ?1 AND e.kind = 'calls' AND e.confidence >= ?2
             ORDER BY e.{near}, e.confidence DESC, e.provider"
        ))?;
        let rows = stmt.query_map(params![node, min_confidence], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u8>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<u32>>(4)?,
            ))
        })?;
        let edges: Vec<_> = rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(other, ..)| distances.get(other) == Some(&(distance - 1)))
            .collect();
        let via = edges.first().map(|e| e.0.clone()).unwrap_or_default();
        let link: Vec<_> = edges.iter().filter(|e| e.0 == via).collect();
        let providers: BTreeSet<String> = link.iter().map(|e| e.1.clone()).collect();
        let confidence = link.iter().map(|e| e.2).max().unwrap_or_default();
        let call_site = match link.first() {
            Some((_, _, _, Some(doc), Some(line))) => self.line_citation(*doc, *line)?,
            _ => None,
        };
        let defined = self
            .conn
            .prepare_cached(
                "SELECT sym.display_name, sym.kind, sym.start_line, sym.end_line, d.path, s.name,
                        s.revision, s.license_class, s.indexed_at
                 FROM symbol sym
                 JOIN document d ON d.id = sym.document_id
                 JOIN source s ON s.id = d.source_id
                 WHERE sym.id = ?1 AND s.is_current",
            )?
            .query_row([node], |row| {
                let path: String = row.get(4)?;
                let source: String = row.get(5)?;
                let revision: String = row.get(6)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    code_citation(&source, &revision, &path, row.get(2)?, row.get(3)?),
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    source,
                ))
            })
            .optional()?;
        let (name, kind, citation, license_class, indexed_at, source) = match defined {
            Some((name, kind, citation, class, indexed, source)) => (
                name,
                kind,
                Some(citation),
                Some(class),
                Some(indexed),
                Some(source),
            ),
            None => (descriptor_name(node), None, None, None, None, None),
        };
        // SCIP IDs are global, so a walk from one source can reach a
        // definition in another; say so rather than cite it silently.
        let defined_in = source.filter(|s| home.is_some_and(|h| h != s));
        Ok(GraphNode {
            id: node.into(),
            name,
            kind,
            distance,
            citation,
            via,
            call_site,
            providers: providers.into_iter().collect(),
            confidence,
            license_class,
            indexed_at,
            defined_in,
        })
    }

    /// One line of a stored document, cited, or `None` when the document
    /// isn't a current code file.
    fn line_citation(&self, document_id: i64, line: u32) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT d.path, s.name, s.revision FROM document d
                 JOIN source s ON s.id = d.source_id WHERE d.id = ?1 AND s.is_current",
            )?
            .query_row([document_id], |row| {
                let path: String = row.get(0)?;
                let source: String = row.get(1)?;
                let revision: String = row.get(2)?;
                Ok(code_citation(&source, &revision, &path, line, line))
            })
            .optional()?)
    }

    /// The one SCIP symbol `symbol` names: a stored ID as written, then a
    /// name or dotted name as [`Store::definition`] resolves it, over every
    /// symbol in a current source. A tree-sitter symbol counts only where
    /// `walk` can start from it (see [`Store::walkable`]).
    fn graph_symbol(&self, symbol: &str, walk: Walk, allow_stale: bool) -> Result<String> {
        let name = symbol.trim();
        if name.is_empty() {
            return Err(Error::EmptyQuery);
        }
        self.check_fresh(allow_stale)?;
        let exact: Option<String> = self
            .conn
            .prepare_cached(
                "SELECT sym.provider FROM symbol sym
                 JOIN document d ON d.id = sym.document_id
                 JOIN source s ON s.id = d.source_id
                 WHERE sym.id = ?1 AND s.is_current",
            )?
            .query_row([name], |row| row.get(0))
            .optional()?;
        match exact.as_deref() {
            Some("treesitter") if !self.walkable(name, walk)? => {
                return Err(Error::NoGraph {
                    symbol: name.into(),
                });
            }
            Some(_) => return Ok(name.into()),
            None => {}
        }
        // A symbol defined outside the corpus, which only occurrences name.
        let occurs: bool = self
            .conn
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM occurrence o
                   JOIN document d ON d.id = o.document_id
                   JOIN source s ON s.id = d.source_id
                   WHERE o.symbol_id = ?1 AND s.is_current)",
            )?
            .query_row([name], |row| row.get(0))?;
        if occurs {
            return Ok(name.into());
        }
        let mut found = self.symbols_named(name)?;
        let parts = qualified(name);
        let last = parts
            .as_ref()
            .and_then(|p| p.last().copied())
            .unwrap_or(name);
        if found.is_empty()
            && let Some(parts) = &parts
        {
            found = self.symbols_named(last)?;
            found.retain(|(id, _)| {
                names_in_file(id).is_some_and(|names| {
                    names.len() >= parts.len()
                        && names[names.len() - parts.len()..].iter().eq(parts.iter())
                })
            });
        }
        if found.is_empty() {
            return Err(Error::NoSuchSymbol {
                symbol: name.into(),
                candidates: self.similar_graph_names(last)?,
            });
        }
        // Only symbols this walk can start from count: any other
        // tree-sitter symbol would only answer `no_graph`.
        let mut walkable = Vec::new();
        for (id, provider) in found {
            if provider != "treesitter" || self.walkable(&id, walk)? {
                walkable.push(id);
            }
        }
        match walkable.len() {
            0 => Err(Error::NoGraph {
                symbol: name.into(),
            }),
            1 => Ok(walkable.remove(0)),
            _ => {
                let limit = usize::try_from(MAX_SEARCH_LIMIT).unwrap_or(usize::MAX);
                walkable.sort();
                walkable.truncate(limit);
                Err(Error::AmbiguousSymbol {
                    symbol: name.into(),
                    candidates: walkable,
                })
            }
        }
    }

    /// Whether `walk` can start from the tree-sitter symbol `id`. A
    /// tree-sitter definition no SCIP definition replaced can still hold
    /// references a SCIP index found, so a SCIP edge can start at it and
    /// `callers` gives its ID as a result. Nothing ends at one: occurrences
    /// and edge targets only ever name SCIP IDs. So only `callees` walks from
    /// it, and only when an edge starts there.
    fn walkable(&self, id: &str, walk: Walk) -> Result<bool> {
        match walk {
            Walk::References | Walk::Callers => Ok(false),
            Walk::Callees => Ok(self
                .conn
                .prepare_cached("SELECT EXISTS (SELECT 1 FROM edge WHERE src = ?1)")?
                .query_row([id], |row| row.get(0))?),
        }
    }

    /// The IDs and providers of the symbols in current sources whose name
    /// as written is `name`. Parameters (an ID ending `(x)`) and type
    /// parameters (`[T]`) are left out: scip-python names every parameter,
    /// and a function's name would be ambiguous with each parameter that
    /// shares it. An ID still finds one, as written.
    fn symbols_named(&self, name: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT sym.id, sym.provider FROM symbol sym
             JOIN document d ON d.id = sym.document_id
             JOIN source s ON s.id = d.source_id
             WHERE sym.display_name = ?1 AND s.is_current
               AND substr(sym.id, -1) NOT IN (')', ']')",
        )?;
        let rows = stmt.query_map([name], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// What a graph tool walks from the symbol it resolves.
#[derive(Clone, Copy)]
enum Walk {
    /// The occurrences that name it.
    References,
    /// The edges that end at it.
    Callers,
    /// The edges that start at it.
    Callees,
}

/// The last name a symbol ID's descriptors give, for a symbol the corpus
/// doesn't define: `is_empty` for
/// ``rust-analyzer cargo core 1.0 str/impl#[str]is_empty().``. The ID
/// itself when it has none.
fn descriptor_name(id: &str) -> String {
    super::definition::symbol_name(id).unwrap_or_else(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::super::scip::fixture::{HEAD, IS_EMPTY, STRING_NEW, app, index, scip};
    use super::*;
    use crate::{ScipDocument, ScipOccurrence};

    fn walk(store: &Store, symbol: &str, direction: Direction, depth: u32) -> Graph {
        store
            .graph(&GraphQuery {
                symbol,
                direction,
                depth,
                min_confidence: 0,
                allow_stale: false,
            })
            .unwrap()
    }

    fn refused(store: &Store, symbol: &str, direction: Direction) -> Error {
        store
            .graph(&GraphQuery {
                symbol,
                direction,
                depth: 1,
                min_confidence: 0,
                allow_stale: false,
            })
            .unwrap_err()
    }

    /// `(name, distance, via's name)` of each result.
    fn reached(graph: &Graph) -> Vec<(String, u32, String)> {
        graph
            .results
            .iter()
            .map(|n| (n.name.clone(), n.distance, descriptor_name(&n.via)))
            .collect()
    }

    fn line(path: &str, n: u32) -> String {
        format!("app@{HEAD}:{path}#L{n}")
    }

    #[test]
    fn references_cite_each_line_with_its_enclosing_definition() {
        let (_dir, mut store, id) = app();
        store.write_scip(id, &index()).unwrap();
        let found = store.references("login", 0, false).unwrap();
        assert_eq!(found.id, scip("login()."));
        assert_eq!(found.total, 1, "two calls on one line are one reference");
        let r = &found.results[0];
        assert_eq!(r.citation, line("src/lib.rs", 15));
        assert_eq!(r.text, "    login(token) && login(token)");
        assert_eq!(r.enclosing, Some(scip("relogin().")));
        assert_eq!(
            (r.role, r.providers.as_slice(), r.confidence),
            ("source", &["scip".to_owned()][..], 3)
        );

        let found = store.references("Client.issuer", 0, false).unwrap();
        let sites: Vec<(&str, &[&str])> = found
            .results
            .iter()
            .map(|r| (r.citation.as_str(), r.access.as_slice()))
            .collect();
        let (l10, l16) = (line("src/client.rs", 10), line("src/client.rs", 16));
        assert_eq!(
            sites,
            [
                (l10.as_str(), &["write"][..]),
                (l16.as_str(), &["read"][..])
            ]
        );
        assert_eq!(r.instruction, None);
        store
            .conn
            .execute("UPDATE source SET license_class = 'approach_only'", [])
            .unwrap();
        let found = store.references("login", 0, false).unwrap();
        assert_eq!(
            found.results[0].instruction.as_deref(),
            Some(crate::APPROACH_ONLY_INSTRUCTION)
        );
        store
            .conn
            .execute("UPDATE source SET license_class = 'adapt'", [])
            .unwrap();
        // Above the confidence any provider gives, nothing is left.
        assert_eq!(store.references("login", 4, false).unwrap().total, 0);
        // An ID outside the corpus works as written.
        let found = store.references(IS_EMPTY, 0, false).unwrap();
        assert_eq!(found.results[0].citation, line("src/client.rs", 21));
    }

    #[test]
    fn callers_and_callees_walk_to_the_depth_asked() {
        let (_dir, mut store, id) = app();
        store.write_scip(id, &index()).unwrap();
        let check = scip("client/check().");
        let graph = walk(&store, &check, Direction::Callers, 1);
        assert_eq!(reached(&graph), [("validate".into(), 1, "check".into())]);
        let validate = &graph.results[0];
        assert_eq!(
            validate.citation,
            Some(format!("app@{HEAD}:src/client.rs#L14-L17"))
        );
        assert_eq!(validate.call_site, Some(line("src/client.rs", 16)));
        assert_eq!(validate.kind.as_deref(), Some("method"));
        assert_eq!(
            (validate.providers.as_slice(), validate.confidence),
            (&["scip".to_owned()][..], 3)
        );

        let graph = walk(&store, &check, Direction::Callers, 3);
        assert_eq!(
            reached(&graph),
            [
                ("validate".into(), 1, "check".into()),
                ("login".into(), 2, "validate".into()),
                ("relogin".into(), 3, "login".into()),
            ]
        );
        assert_eq!(graph.total, 3);

        let graph = walk(&store, "login", Direction::Callees, 1);
        assert_eq!(
            reached(&graph),
            [
                ("new".into(), 1, "login".into()),
                ("validate".into(), 1, "login".into())
            ]
        );
        let graph = walk(&store, "login", Direction::Callees, 2);
        // A callee outside the corpus has no citation and comes last at its
        // distance.
        assert_eq!(
            reached(&graph),
            [
                ("new".into(), 1, "login".into()),
                ("validate".into(), 1, "login".into()),
                ("check".into(), 2, "validate".into()),
                ("new".into(), 2, "new".into()),
            ]
        );
        assert_eq!(graph.results[3].id, STRING_NEW);
        assert_eq!(graph.results[3].citation, None);
        assert_eq!(graph.results[3].license_class, None);
        // Edges below the confidence asked for are not followed.
        let none = store
            .graph(&GraphQuery {
                symbol: "login",
                direction: Direction::Callees,
                depth: 1,
                min_confidence: 4,
                allow_stale: false,
            })
            .unwrap();
        assert_eq!(none.total, 0);
    }

    #[test]
    fn a_walk_returns_at_most_50_and_counts_them_all() {
        let (_dir, mut store, id) = app();
        store.write_scip(id, &index()).unwrap();
        let check = scip("client/check().");
        for i in 0..60 {
            store
                .conn
                .execute(
                    "INSERT INTO edge (src, dst, kind, provider, confidence, evidence_doc,
                                       evidence_line)
                     VALUES (?1, ?2, 'calls', 'scip', 3, 1, 1)",
                    params![format!("ext . . . caller{i:02}()."), check],
                )
                .unwrap();
        }
        let graph = walk(&store, &check, Direction::Callers, 2);
        // validate, the 60 outside the corpus, then login at distance 2.
        assert_eq!(graph.total, 62);
        assert_eq!(graph.results.len(), 50);
        assert_eq!(graph.results[0].name, "validate");
        assert_eq!(graph.results[1].name, "caller00");
        assert!(graph.results.iter().all(|n| n.distance == 1));
    }

    #[test]
    fn a_cycle_ends_where_the_depth_does() {
        let (_dir, mut store, id) = app();
        let mut docs = index();
        // check calls validate back.
        docs[1].occurrences.push(ScipOccurrence {
            symbol: scip("client/impl#[Client]validate()."),
            start_line: 21,
            end_line: 21,
            end_column: None,
            roles: 0,
        });
        store.write_scip(id, &docs).unwrap();
        let graph = walk(&store, "relogin", Direction::Callees, MAX_GRAPH_DEPTH);
        let names: Vec<(String, u32)> = graph
            .results
            .iter()
            .map(|n| (n.name.clone(), n.distance))
            .collect();
        assert_eq!(
            names,
            [
                ("login".into(), 1),
                ("new".into(), 2),
                ("validate".into(), 2),
                ("check".into(), 3),
                ("new".into(), 3),
                ("is_empty".into(), 4),
            ]
        );
        // The asked symbol is never its own caller.
        let graph = walk(&store, "validate", Direction::Callers, MAX_GRAPH_DEPTH);
        assert!(
            graph.results.iter().all(|n| n.name != "validate"),
            "{graph:?}"
        );
    }

    #[test]
    fn a_tree_sitter_symbol_in_the_scip_graph_can_be_walked_from() {
        let (_dir, mut store, id) = app();
        let mut docs = index();
        // SCIP defines nothing named `helper`, but it calls check.
        docs[0].occurrences.push(ScipOccurrence {
            symbol: scip("client/check()."),
            start_line: 18,
            end_line: 18,
            end_column: None,
            roles: 0,
        });
        store.write_scip(id, &docs).unwrap();
        let helper = "codecite . app . src/`lib.rs`/helper().";
        let callers = walk(&store, &scip("client/check()."), Direction::Callers, 1);
        assert!(
            callers.results.iter().any(|n| n.id == helper),
            "{callers:?}"
        );
        // The ID a result gave is accepted, by ID and by name.
        let callees = walk(&store, helper, Direction::Callees, 1);
        assert_eq!(callees.results[0].id, scip("client/check()."));
        assert_eq!(walk(&store, "helper", Direction::Callees, 1).total, 1);
        // Occurrences and edge targets only ever name SCIP IDs, so nothing
        // refers to or calls a tree-sitter symbol: those walks have no graph.
        for symbol in [helper, "helper"] {
            let err = refused(&store, symbol, Direction::Callers);
            assert!(matches!(err, Error::NoGraph { .. }), "{err:?}");
            let err = store.references(symbol, 0, false).unwrap_err();
            assert!(matches!(err, Error::NoGraph { .. }), "{err:?}");
        }
    }

    #[test]
    fn a_definition_in_another_source_than_the_asked_symbol_is_marked() {
        use crate::{DocumentRecord, ScipDefinition, SourceRecord};
        let (_dir, mut store, id) = app();
        store.write_scip(id, &index()).unwrap();
        // `other` has a function that calls app's `check` and `login`.
        let other = store
            .upsert_source(&SourceRecord {
                name: "other",
                kind: "local",
                url: "/src/other",
                revision: HEAD,
                license_spdx: None,
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        let text = "fn run() {\n    check(\"x\");\n    login(\"x\");\n}\n";
        store
            .write_document(
                other,
                &DocumentRecord {
                    path: "src/run.rs",
                    language: Some("rust"),
                    content_hash: b"run",
                    size_bytes: text.len() as u64,
                    dirty: false,
                    text: Some(text),
                    chunks: vec![],
                },
            )
            .unwrap();
        let run = "rust-analyzer cargo other 0.1.0 run().";
        let call = |symbol: String, line| ScipOccurrence {
            symbol,
            start_line: line,
            end_line: line,
            end_column: None,
            roles: 0,
        };
        store
            .write_scip(
                other,
                &[ScipDocument {
                    encoding: Default::default(),
                    path: "src/run.rs".into(),
                    definitions: vec![ScipDefinition {
                        symbol: run.into(),
                        display_name: "run".into(),
                        kind: "function".into(),
                        line: 1,
                        start_line: 1,
                        end_line: 4,
                    }],
                    occurrences: vec![call(scip("client/check()."), 2), call(scip("login()."), 3)],
                }],
            )
            .unwrap();
        let marks = |graph: &Graph| -> Vec<(String, Option<String>)> {
            graph
                .results
                .iter()
                .map(|n| (n.name.clone(), n.defined_in.clone()))
                .collect()
        };
        let app = || Some("app".to_owned());
        // Every node app defines is marked, two hops out as well as one:
        // `new` and `validate` are called from app's `login`, but `run`,
        // the asked symbol, is in `other`.
        let callees = walk(&store, run, Direction::Callees, 2);
        assert_eq!(
            marks(&callees),
            [
                ("check".to_owned(), app()),
                ("login".to_owned(), app()),
                ("new".to_owned(), app()),
                ("validate".to_owned(), app()),
                ("is_empty".to_owned(), None),
            ],
            "{callees:?}"
        );
        // A caller in the asked symbol's own source isn't marked; one in
        // another source is.
        let callers = walk(&store, &scip("login()."), Direction::Callers, 1);
        assert_eq!(
            marks(&callers),
            [
                ("relogin".to_owned(), None),
                ("run".to_owned(), Some("other".to_owned())),
            ],
            "{callers:?}"
        );
    }

    #[test]
    fn a_symbol_must_name_one_symbol_with_a_graph() {
        let (_dir, mut store, id) = app();
        // Before SCIP, nothing has a graph.
        let err = store.references("login", 0, false).unwrap_err();
        assert!(matches!(err, Error::NoGraph { .. }), "{err:?}");
        store.write_scip(id, &index()).unwrap();
        let err = store.references("helper", 0, false).unwrap_err();
        assert!(matches!(err, Error::NoGraph { .. }), "{err:?}");
        // lib.rs's tree-sitter `check` has no graph, so `check` is
        // client.rs's SCIP one.
        assert_eq!(
            store.references("check", 0, false).unwrap().id,
            scip("client/check().")
        );
        // Once lib.rs's `check` holds a reference SCIP found, both can be
        // walked from for callees, and the name is ambiguous there; only
        // client.rs's can be referred to or called.
        let mut docs = index();
        docs[0].occurrences.push(ScipOccurrence {
            symbol: scip("login()."),
            start_line: 20,
            end_line: 20,
            end_column: None,
            roles: 0,
        });
        store.write_scip(id, &docs).unwrap();
        assert_eq!(
            store.references("check", 0, false).unwrap().id,
            scip("client/check().")
        );
        assert_eq!(
            walk(&store, "check", Direction::Callers, 1).id,
            scip("client/check().")
        );
        let err = refused(&store, "check", Direction::Callees);
        let Error::AmbiguousSymbol { candidates, .. } = &err else {
            panic!("{err:?}")
        };
        assert_eq!(
            candidates,
            &[
                "codecite . app . src/`lib.rs`/check().".to_owned(),
                scip("client/check().")
            ]
        );
        let err = store.references("Login", 0, false).unwrap_err();
        let Error::NoSuchSymbol { candidates, .. } = &err else {
            panic!("{err:?}")
        };
        assert_eq!(candidates, &["login".to_owned()]);
        // A SCIP-only name is suggested, since the graph tools find it.
        let err = store.references("Issuer", 0, false).unwrap_err();
        let Error::NoSuchSymbol { candidates, .. } = &err else {
            panic!("{err:?}")
        };
        assert_eq!(candidates, &["issuer".to_owned()]);
        let err = store.references("  ", 0, false).unwrap_err();
        assert!(matches!(err, Error::EmptyQuery), "{err:?}");
        // Dotted names and IDs both find a symbol.
        assert_eq!(
            store.references("Client.validate", 0, false).unwrap().total,
            1
        );
        assert_eq!(
            store
                .references(&scip("client/impl#[Client]validate()."), 0, false)
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn graph_tools_refuse_a_stale_index() {
        let (_dir, mut store, id) = app();
        store.write_scip(id, &[ScipDocument::default()]).unwrap();
        store
            .conn
            .execute(
                "UPDATE meta SET value = '2000-01-01T00:00:00Z' WHERE key = 'indexed_at'",
                [],
            )
            .unwrap();
        let err = store.references("login", 0, false).unwrap_err();
        assert!(matches!(err, Error::StaleIndex { .. }), "{err:?}");
        let err = store
            .graph(&GraphQuery {
                symbol: "login",
                direction: Direction::Callers,
                depth: 1,
                min_confidence: 0,
                allow_stale: false,
            })
            .unwrap_err();
        assert!(matches!(err, Error::StaleIndex { .. }), "{err:?}");
    }
}
