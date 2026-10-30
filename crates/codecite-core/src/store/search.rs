//! Searching a corpus: BM25 over the full-text index, weighted by what each
//! document is for and lifted for a symbol a query word names, with each
//! result cited.

use std::collections::HashMap;

use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OptionalExtension, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Store;
use super::read::byte_prefix;
use crate::{Error, Result, Role, text_terms};

/// The most lines of a chunk a result's snippet holds. Agents read the
/// rest through its citation.
pub const MAX_SNIPPET_LINES: usize = 40;

/// The most bytes a result's snippet holds, after the line limit: one
/// minified or generated line can hold a whole file.
pub const MAX_SNIPPET_BYTES: usize = 4 * 1024;

/// How many results a search returns unless asked for another number:
/// the CLI's `--limit` and the MCP `search` tool's `limit` default.
pub const DEFAULT_SEARCH_LIMIT: u32 = 5;

/// The most results a search may ask for: the CLI's `--limit` and the MCP
/// `search` tool's `limit` refuse more.
pub const MAX_SEARCH_LIMIT: u32 = 50;

/// How many days an index stays fresh when its manifest doesn't say.
pub const DEFAULT_MAX_AGE_DAYS: u32 = 30;

/// The `bm25()` weight of each full-text column, in the table's order:
/// a match in a symbol's name counts most, then one in the path, then one
/// in the text.
const COLUMN_WEIGHTS: (f64, f64, f64) = (10.0, 5.0, 1.0);

/// The factor on a chunk's score when a word of the query, or the whole
/// query, is its symbol's name. Large enough that a long definition, whose
/// BM25 score its length dilutes, outranks short chunks that only use it.
const NAME_BOOST: f64 = 3.0;

/// The most definitions a name can have and still be boosted. A name that
/// picks out a few definitions is one the query means; a common word that
/// many helpers are named after (`app`, `user`, `get`) is not.
const MAX_NAME_DEFINITIONS: u32 = 3;

/// The most slots at the end of a result list held for the best spec
/// results when the query names a symbol and code fills the rest.
const SPEC_SLOTS: usize = 3;

/// Which sources a search covers: specs, or everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Only spec sections and steps.
    Spec,
    /// Only code and generic documents: repositories, local directories
    /// and fetched pages that are not specs.
    Code,
}

/// A search: the text, the filters and how many results to return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery<'a> {
    /// What to search for: words, identifiers or both.
    pub text: &'a str,
    /// Only documents in this language, as the index names it (`rust`).
    pub language: Option<&'a str>,
    /// Only documents whose path matches this glob (SQLite `GLOB`: `*`
    /// matches any run of characters, `/` included, and case matters).
    pub path: Option<&'a str>,
    /// Only specs or only code. Without it, a search covers both.
    pub only: Option<Scope>,
    /// The most results to return.
    pub limit: u32,
    /// Search an index older than its maximum age instead of refusing it.
    pub allow_stale: bool,
}

/// The results of a search, best first: what `codecite search --json`
/// prints and the MCP `search` tool returns, as the same JSON. An MCP
/// tool's structured result must be an object, so the list is a field.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SearchResults {
    /// The ranked results; none when nothing matched.
    pub results: Vec<SearchResult>,
}

/// One ranked result, in the shape the CLI prints as JSON and the MCP
/// server returns.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SearchResult {
    /// Where the chunk is: `repo@sha:path#L10-L42`,
    /// `doc@sha256:<12 hex>#L3-L9` for a fetched document that is not a
    /// spec, or `doc@sha256:<12 hex>#section-4.3[/step-2]` for a spec.
    pub citation: String,
    /// `function`, `class`, `variable`, `type_alias`, `section`, `step` or
    /// `window`.
    pub kind: String,
    /// The name of the definition the chunk holds, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// For a section of a file or generic document, its heading's text.
    /// Spec results have none: their citation names the section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// For a section of a file or generic document, its heading's anchor,
    /// which `read <source>@<pin>[:<path>]#<anchor>` reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// What the chunk's document is for: `source`, `test`, `example`,
    /// `docs` or `spec`.
    pub role: &'static str,
    /// The ranking score, higher first: BM25 times the role's weight, times
    /// a boost when the query names the chunk's symbol. Spec and code
    /// results each come in this order; how the two are merged is up to
    /// `Store::search`, since their scores are not on one scale.
    pub score: f64,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    pub license_class: String,
    /// When the chunk's source was last indexed.
    pub indexed_at: String,
    /// For code, whether the file differed from the cited commit when it
    /// was indexed. Fetched documents have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    /// The chunk's first 40 lines (`MAX_SNIPPET_LINES`), and at most 4 KiB
    /// of them (`MAX_SNIPPET_BYTES`): a prefix of the chunk's lines joined
    /// with `\n`, cut at a character boundary, with no marker in it. `read`
    /// its citation for the rest.
    pub snippet: String,
    /// Whether the snippet stops before the end of the chunk, at the line
    /// limit or the byte budget. The citation still names the whole chunk.
    pub snippet_truncated: bool,
    /// For a spec result whose document has older pinned versions, their
    /// pinned citations: `diff` shows how they differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub older_versions: Option<Vec<String>>,
}

/// When a corpus was indexed, and whether that is too long ago.
#[derive(Debug, Clone, PartialEq)]
pub struct Freshness {
    /// When the last index run finished.
    pub indexed_at: String,
    /// The manifest's `max_age_days` at that run.
    pub max_age_days: u32,
    /// Days since the run, fractional.
    pub age_days: f64,
}

impl Freshness {
    /// Whether the index is older than its maximum age.
    pub fn is_stale(&self) -> bool {
        self.age_days > f64::from(self.max_age_days)
    }
}

impl Store {
    /// When the corpus was last indexed, and its maximum age.
    ///
    /// A corpus whose first index run never finished has no `indexed_at`
    /// and is an [`Error::InvalidCorpus`]. One indexed before the maximum
    /// age was stored gets [`DEFAULT_MAX_AGE_DAYS`].
    pub fn freshness(&self) -> Result<Freshness> {
        let row: Option<(String, Option<String>, f64)> = self
            .conn
            .query_row(
                "SELECT i.value, (SELECT value FROM meta WHERE key = 'max_age_days'),
                   julianday('now') - julianday(i.value)
                 FROM meta i WHERE i.key = 'indexed_at'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((indexed_at, max_age_days, age_days)) = row else {
            return Err(Error::InvalidCorpus {
                path: self.path.clone(),
                reason: "it was never fully indexed; run codecite index".into(),
            });
        };
        let max_age_days = match max_age_days {
            None => DEFAULT_MAX_AGE_DAYS,
            Some(days) => days.parse().map_err(|_| Error::InvalidCorpus {
                path: self.path.clone(),
                reason: format!("unreadable max_age_days {days:?}"),
            })?,
        };
        Ok(Freshness {
            indexed_at,
            max_age_days,
            age_days,
        })
    }

    /// Searches the corpus and returns at most `query.limit` results, best
    /// first.
    ///
    /// Each word of the query is split into identifier terms as the index
    /// splits text, and a chunk matches when it holds any of them. BM25
    /// scores the matches, weighting the symbol column, then the path, then
    /// the text; the score is then weighted by the document's [`Role`], so
    /// tests and examples rank below the source they use, and multiplied by
    /// [`NAME_BOOST`] when a word of the query, or the whole query, is the
    /// chunk's symbol's name (ignoring case, `_` and `-`) and at most
    /// [`MAX_NAME_DEFINITIONS`] symbols have that name. Only the current
    /// version of each document is searched.
    ///
    /// Specs are searched in their own full-text table, stemmed with Porter,
    /// so "authenticates" finds "authentication", and without the query's
    /// [`STOPWORDS`] (see [`spec_fts_query`]). Code is searched as written,
    /// with every term.
    ///
    /// Spec and code results are ranked separately, because their BM25
    /// scores are not on one scale: a short constants file that repeats a
    /// term outscores the long section that defines it. The two lists are
    /// then merged. When the query names a symbol, results keep score
    /// order, except that the best spec results, up to [`SPEC_SLOTS`] and a
    /// third of `query.limit`, are lifted into the last slots. Otherwise the
    /// two alternate, spec first. `query.only` searches just one of them.
    ///
    /// A query with no letter or digit is [`Error::EmptyQuery`]. An index
    /// older than its maximum age is [`Error::StaleIndex`] unless
    /// `query.allow_stale` is set.
    pub fn search(&self, query: &SearchQuery<'_>) -> Result<Vec<SearchResult>> {
        let fts = fts_query(query.text).ok_or(Error::EmptyQuery)?;
        self.check_fresh(query.allow_stale)?;
        let boosted = self.boosted_names(query.text)?;
        let named = !boosted.is_empty();
        let mut names = String::from(NAME_SEPARATOR);
        for name in boosted {
            names.push_str(&name);
            names.push(NAME_SEPARATOR);
        }
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
        let ranked =
            |table: &str, fts: &str, spec: bool| self.ranked(table, fts, spec, &names, query);
        let code = match query.only {
            Some(Scope::Spec) => Vec::new(),
            _ => ranked("chunk_fts", &fts, false)?,
        };
        let mut spec = match query.only {
            Some(Scope::Code) => Vec::new(),
            _ => ranked("spec_fts", &spec_fts_query(query.text).unwrap_or(fts), true)?,
        };
        self.add_older_versions(&mut spec)?;
        Ok(blend(code, spec, limit, named, |r| r.score))
    }

    /// The best `query.limit` spec chunks, or code chunks, that the
    /// full-text `table` matches for `fts`, best first. `chunk_fts` holds
    /// every chunk, `spec_fts` only specs. A fetched document stored with
    /// its text is a generic page, ranked with code.
    fn ranked(
        &self,
        table: &str,
        fts: &str,
        spec: bool,
        names: &str,
        query: &SearchQuery<'_>,
    ) -> Result<Vec<SearchResult>> {
        let (w_symbol, w_path, w_text) = COLUMN_WEIGHTS;
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT c.kind, c.start_line, c.end_line, c.number, p.number, c.text, d.path, d.dirty,
               s.name, s.kind, s.revision, s.license_class, s.indexed_at, sym.display_name,
               -bm25({table}, ?3, ?4, ?5)
                 * codecite_role_weight(d.path, s.kind, d.text IS NOT NULL)
                 * CASE WHEN instr(?2, char(31) || codecite_name_key(sym.display_name) || char(31)) > 0
                        THEN ?9 ELSE 1.0 END AS score,
               d.text IS NOT NULL, c.title, c.anchor
             FROM {table}
             JOIN chunk c ON c.id = {table}.rowid
             JOIN document d ON d.id = c.document_id
             JOIN source s ON s.id = d.source_id
             LEFT JOIN chunk p ON p.id = c.parent_id
             LEFT JOIN symbol sym ON sym.id = c.symbol_id
             WHERE {table} MATCH ?1 AND s.is_current
               AND (?6 IS NULL OR d.language = ?6)
               AND (?7 IS NULL OR d.path GLOB ?7)
               AND (s.kind = 'document' AND d.text IS NULL) = ?10
             -- Break ties on what the result cites, not on c.id: a chunk ID
             -- records when the chunk was written, and a reindex keeps the
             -- IDs of unchanged files, so ID order differs from a fresh index.
             ORDER BY score DESC, s.name, d.path, c.start_line, c.end_line
             LIMIT ?8"
        ))?;
        let rows = stmt.query_map(
            params![
                fts,
                names,
                w_symbol,
                w_path,
                w_text,
                query.language,
                query.path,
                query.limit,
                NAME_BOOST,
                spec,
            ],
            |row| {
                let kind: String = row.get(0)?;
                let start: u32 = row.get(1)?;
                let end: u32 = row.get(2)?;
                let number: Option<String> = row.get(3)?;
                let parent_number: Option<String> = row.get(4)?;
                let text: String = row.get(5)?;
                let path: String = row.get(6)?;
                let dirty: bool = row.get(7)?;
                let source: String = row.get(8)?;
                let source_kind: String = row.get(9)?;
                let revision: String = row.get(10)?;
                let has_text: bool = row.get(15)?;
                let fetched = source_kind == "document";
                let is_spec = fetched && !has_text;
                let (snippet, snippet_truncated) = snippet(&text);
                let role = Role::of_stored(&path, &source_kind, has_text).as_str();
                let citation = if is_spec {
                    spec_citation(&source, &revision, &kind, number, parent_number)
                } else if fetched {
                    document_citation(&source, &revision, start, end)
                } else {
                    code_citation(&source, &revision, &path, start, end)
                };
                // A spec's citation already names its section.
                let heading = |i| -> rusqlite::Result<Option<String>> {
                    Ok(if is_spec { None } else { row.get(i)? })
                };
                Ok(SearchResult {
                    citation,
                    kind,
                    symbol: row.get(13)?,
                    title: heading(16)?,
                    anchor: heading(17)?,
                    role,
                    score: row.get(14)?,
                    license_class: row.get(11)?,
                    indexed_at: row.get(12)?,
                    dirty: (!fetched).then_some(dirty),
                    snippet,
                    snippet_truncated,
                    older_versions: None,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sets each spec result's `older_versions`. Search reads only current
    /// versions, so each result's document is current.
    fn add_older_versions(&self, results: &mut [SearchResult]) -> Result<()> {
        let mut by_name: HashMap<String, Option<Vec<String>>> = HashMap::new();
        for r in results {
            let Some((name, _)) = r.citation.split_once('@') else {
                continue;
            };
            if !by_name.contains_key(name) {
                let older = match self.current_version(name)? {
                    Some(current) => self.older_versions(name, current.id)?,
                    None => None,
                };
                by_name.insert(name.to_owned(), older);
            }
            r.older_versions.clone_from(&by_name[name]);
        }
        Ok(())
    }

    /// The [`query_names`] that at least one and at most
    /// [`MAX_NAME_DEFINITIONS`] symbols have.
    fn boosted_names(&self, text: &str) -> Result<Vec<String>> {
        let mut count = self.conn.prepare_cached(
            "SELECT count(*) FROM symbol WHERE codecite_name_key(display_name) = ?1",
        )?;
        let mut names = Vec::new();
        for name in query_names(text) {
            let defined: u32 = count.query_row([&name], |row| row.get(0))?;
            if (1..=MAX_NAME_DEFINITIONS).contains(&defined) {
                names.push(name);
            }
        }
        Ok(names)
    }

    /// Refuses an index older than its maximum age with
    /// [`Error::StaleIndex`], unless `allow_stale` is set.
    pub(super) fn check_fresh(&self, allow_stale: bool) -> Result<()> {
        let freshness = self.freshness()?;
        if freshness.is_stale() && !allow_stale {
            return Err(Error::StaleIndex {
                corpus: self.corpus_name(),
                indexed_at: freshness.indexed_at,
                max_age_days: freshness.max_age_days,
            });
        }
        Ok(())
    }

    /// The corpus's name: its file's name without `.db`.
    fn corpus_name(&self) -> String {
        self.path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
    }
}

/// Registers the SQL functions [`Store::search`] ranks with, once per
/// connection: `codecite_role_weight(path, source_kind, has_text)` and
/// `codecite_name_key(name)`.
pub(super) fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "codecite_role_weight",
        3,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let path: String = ctx.get(0)?;
            let source_kind: String = ctx.get(1)?;
            let has_text: bool = ctx.get(2)?;
            Ok(Role::of_stored(&path, &source_kind, has_text).weight())
        },
    )?;
    conn.create_scalar_function(
        "codecite_name_key",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(ctx.get::<Option<String>>(0)?.map(|name| name_key(&name))),
    )
}

/// Merges code and spec results, each best first, into at most `limit`.
///
/// When the query `named` a symbol, the order is by `score`, with ties to
/// code, except that the best `reserve` spec results move up into the last
/// `reserve` slots if they would fall below them. Otherwise the two kinds
/// alternate, spec first, and whichever runs out first leaves the rest to
/// the other.
fn blend<T>(
    code: Vec<T>,
    spec: Vec<T>,
    limit: usize,
    named: bool,
    score: impl Fn(&T) -> f64,
) -> Vec<T> {
    // Sized by the results, not the caller's limit, which may be huge.
    let room = limit.min(code.len() + spec.len());
    if !named {
        let mut merged = Vec::with_capacity(room);
        let (mut code, mut spec) = (code.into_iter(), spec.into_iter());
        while merged.len() < limit {
            match (spec.next(), code.next()) {
                (None, None) => break,
                (s, c) => merged.extend(s.into_iter().chain(c)),
            }
        }
        merged.truncate(limit);
        return merged;
    }
    let reserve = (limit / 3).min(SPEC_SLOTS).min(spec.len());
    let head = limit - reserve;
    // (held for a spec, result), best first.
    let mut by_score: Vec<(bool, bool, T)> = code
        .into_iter()
        .map(|r| (false, false, r))
        .chain(
            spec.into_iter()
                .enumerate()
                .map(|(i, r)| (true, i < reserve, r)),
        )
        .collect();
    by_score.sort_by(|a, b| score(&b.2).total_cmp(&score(&a.2)).then(a.0.cmp(&b.0)));
    // The first `head` by score, then the held spec results that missed
    // them, then the next best.
    let mut slots: Vec<Option<(bool, bool, T)>> = by_score.into_iter().map(Some).collect();
    let mut merged = Vec::with_capacity(room);
    for slot in slots.iter_mut().take(head) {
        merged.extend(slot.take());
    }
    let held = slots.iter_mut().filter(|s| s.as_ref().is_some_and(|s| s.1));
    let held: Vec<_> = held.filter_map(Option::take).collect();
    let rest = slots.into_iter().flatten();
    let mut tail: Vec<_> = held
        .into_iter()
        .chain(rest)
        .take(limit - merged.len())
        .collect();
    tail.sort_by(|a, b| score(&b.2).total_cmp(&score(&a.2)).then(a.0.cmp(&b.0)));
    merged.extend(tail);
    merged.into_iter().map(|(_, _, r)| r).collect()
}

/// Turns the user's text into an FTS5 query, or `None` when it has no
/// terms.
///
/// Each term is quoted, so FTS5 reads it as a plain string and never as
/// syntax (`AND`, `NEAR`, `col:`, `*`), and the terms are joined with `OR`.
fn fts_query(text: &str) -> Option<String> {
    let terms = query_terms(text);
    (!terms.is_empty()).then(|| join_terms(&terms, " OR "))
}

/// [`fts_query`] for specs: the terms without [`STOPWORDS`], since in an
/// `OR` of every term, "is client authentication required" matches every
/// section that says "is". A query of only stopwords keeps them all and
/// joins them with `AND`, so it finds the sections that say all of them
/// rather than every section.
fn spec_fts_query(text: &str) -> Option<String> {
    let terms = query_terms(text);
    let kept: Vec<String> = terms
        .iter()
        .filter(|t| STOPWORDS.binary_search(&t.as_str()).is_err())
        .cloned()
        .collect();
    if kept.is_empty() {
        (!terms.is_empty()).then(|| join_terms(&terms, " AND "))
    } else {
        Some(join_terms(&kept, " OR "))
    }
}

/// The query's terms, as the index splits text, lowercased and without
/// repeats, in the order they first appear.
fn query_terms(text: &str) -> Vec<String> {
    // The tokenizer folds case, so `NEAR` and `near` are one term.
    let mut seen: Vec<String> = Vec::new();
    for term in text_terms(text).to_lowercase().split(' ') {
        if !term.is_empty() && !seen.iter().any(|t| t == term) {
            seen.push(term.to_owned());
        }
    }
    seen
}

/// Each term quoted, with `"` doubled, joined with `op`.
fn join_terms(terms: &[String], op: &str) -> String {
    let quoted: Vec<String> = terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    quoted.join(op)
}

/// Common English words, and the question words agents put in front of a
/// query, that a spec search leaves out: the list the retrieval spike
/// measured (`experiments/spec-retrieval/lexical.py`), without the words
/// that say which requirement a query is after. Those are RFC 2119's
/// keywords ("must", "shall", "should", "may", "required") and the
/// negation in "MUST NOT" and "no". Sorted, for `binary_search`.
const STOPWORDS: &[&str] = &[
    "a", "about", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been", "being",
    "by", "can", "could", "did", "do", "does", "doing", "done", "down", "each", "every", "for",
    "from", "get", "got", "had", "has", "have", "having", "he", "her", "his", "how", "i", "if",
    "in", "into", "is", "it", "its", "just", "let", "make", "me", "more", "most", "my", "of",
    "off", "on", "only", "or", "other", "our", "out", "over", "own", "s", "same", "she", "so",
    "some", "someone", "such", "sure", "t", "than", "that", "the", "their", "them", "then",
    "there", "these", "they", "this", "those", "to", "too", "under", "up", "very", "was", "we",
    "were", "what", "when", "where", "which", "who", "whom", "whose", "why", "will", "with",
    "would", "you", "your",
];

/// Separates the names passed to the search, so SQL finds one with
/// `instr`. No name holds it: [`query_names`] splits words at it.
const NAME_SEPARATOR: char = '\u{1f}';

/// The names a query can mean: each word, and all the words run together,
/// so `validate token` names both `validate` and `validateToken`. Each is a
/// [`name_key`].
fn query_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .map(name_key)
        .filter(|w| !w.is_empty())
        .collect();
    if names.len() > 1 {
        names.push(names.concat());
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    names
}

/// A name as the search compares it: lowercase, without `_` or `-`.
pub(super) fn name_key(name: &str) -> String {
    name.chars()
        .filter(|&c| c != '_' && c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// `repo@sha:path#L10-L42`, or `#L10` for one line.
pub(super) fn code_citation(
    source: &str,
    revision: &str,
    path: &str,
    start: u32,
    end: u32,
) -> String {
    if start == end {
        format!("{source}@{revision}:{path}#L{start}")
    } else {
        format!("{source}@{revision}:{path}#L{start}-L{end}")
    }
}

/// `doc@sha256:<12 hex>#L3-L9`, or `#L3` for one line: lines of a fetched
/// document that is not a spec.
pub(super) fn document_citation(doc: &str, sha256: &str, start: u32, end: u32) -> String {
    let hash = sha256.get(..12).unwrap_or(sha256);
    if start == end {
        format!("{doc}@sha256:{hash}#L{start}")
    } else {
        format!("{doc}@sha256:{hash}#L{start}-L{end}")
    }
}

/// `doc@sha256:<12 hex>#section-<n>`, or `…#section-<n>/step-<k>` for a step,
/// whose section is its parent's number.
pub(super) fn spec_citation(
    doc: &str,
    sha256: &str,
    kind: &str,
    number: Option<String>,
    parent_number: Option<String>,
) -> String {
    let hash = sha256.get(..12).unwrap_or(sha256);
    let number = number.unwrap_or_default();
    match (kind, parent_number) {
        ("step", Some(section)) => format!("{doc}@sha256:{hash}#section-{section}/step-{number}"),
        _ => format!("{doc}@sha256:{hash}#section-{number}"),
    }
}

/// The first [`MAX_SNIPPET_LINES`] lines of a chunk's text, cut to
/// [`MAX_SNIPPET_BYTES`], and whether anything was left out.
pub(super) fn snippet(text: &str) -> (String, bool) {
    let mut lines = text.lines();
    let kept: Vec<&str> = lines.by_ref().take(MAX_SNIPPET_LINES).collect();
    let more_lines = lines.next().is_some();
    let joined = kept.join("\n");
    let (cut, over) = byte_prefix(&joined, MAX_SNIPPET_BYTES);
    (cut.to_owned(), over || more_lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, Citation, DocumentRecord, Keyword, SourceRecord, SymbolRecord};
    use std::path::{Path, PathBuf};

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    /// The one function named `validate`. Its long body dilutes its BM25
    /// score below another chunk's.
    const VALIDATE: &str = "pub fn validate(token: &str) -> Result<Claims, Error> {
    let claims = decode(token)?;
    check_expiry(&claims)?;
    check_audience(&claims)
}";
    const SHA256: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";

    /// A store holding a small fixture corpus, in its own directory.
    struct Fixture {
        store: Store,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn search(&self, text: &str) -> Vec<SearchResult> {
            self.store.search(&query(text)).unwrap()
        }

        fn citations(&self, query: &SearchQuery<'_>) -> Vec<String> {
            let results = self.store.search(query).unwrap();
            results.into_iter().map(|r| r.citation).collect()
        }

        fn path(&self) -> PathBuf {
            self.dir.path().join("demo.db")
        }

        fn raw(&self) -> rusqlite::Connection {
            rusqlite::Connection::open(self.path()).unwrap()
        }
    }

    fn query(text: &str) -> SearchQuery<'_> {
        SearchQuery {
            text,
            language: None,
            path: None,
            only: None,
            limit: 10,
            allow_stale: false,
        }
    }

    fn code_source(name: &str) -> SourceRecord<'_> {
        SourceRecord {
            name,
            kind: "local",
            url: "/src/app",
            revision: HEAD,
            license_spdx: Some("MIT"),
            license_class: "adapt",
            is_current: true,
            obsoleted_by: None,
            updated_by: None,
        }
    }

    fn chunk<'a>(
        kind: &'a str,
        lines: (u32, u32),
        text: &'a str,
        symbol: Option<&'a str>,
    ) -> ChunkRecord<'a> {
        ChunkRecord {
            kind,
            start_line: lines.0,
            end_line: lines.1,
            text,
            number: None,
            anchor: None,
            title: None,
            parent: None,
            strength: &[],
            symbol: symbol.map(|name| SymbolRecord {
                id: format!("codecite . app . {name}()."),
                display_name: name,
                kind: "function",
            }),
        }
    }

    fn file<'a>(
        path: &'a str,
        language: &'a str,
        dirty: bool,
        chunks: Vec<ChunkRecord<'a>>,
    ) -> DocumentRecord<'a> {
        DocumentRecord {
            path,
            language: Some(language),
            content_hash: path.as_bytes(),
            size_bytes: 1,
            dirty,
            text: None,
            chunks,
        }
    }

    /// One spec version: section 4.3 with step 2, which holds MUST.
    fn spec<'a>(text: &'a str) -> DocumentRecord<'a> {
        let section = ChunkRecord {
            kind: "section",
            number: Some("4.3"),
            anchor: Some("checking"),
            title: Some("Checking DPoP Proofs"),
            ..chunk("section", (10, 20), "Checking DPoP Proofs", None)
        };
        let step = ChunkRecord {
            kind: "step",
            number: Some("2"),
            anchor: Some("checking"),
            parent: Some(0),
            strength: &[Keyword::Must],
            ..chunk("step", (12, 13), text, None)
        };
        DocumentRecord {
            path: "rfc9449",
            language: None,
            content_hash: text.as_bytes(),
            size_bytes: 1,
            dirty: false,
            text: None,
            chunks: vec![section, step],
        }
    }

    fn spec_source<'a>(sha256: &'a str, is_current: bool) -> SourceRecord<'a> {
        SourceRecord {
            name: "rfc9449",
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

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store.upsert_source(&code_source("app")).unwrap();
        let docs = [
            // Says "validate" most often, but defines nothing by that name.
            file(
                "src/checks.rs",
                "rust",
                false,
                vec![chunk(
                    "function",
                    (1, 3),
                    "fn checks() { validate(); validate(); validate(); }",
                    Some("checks"),
                )],
            ),
            file(
                "src/validate.rs",
                "rust",
                true,
                vec![chunk("function", (5, 9), VALIDATE, Some("validate"))],
            ),
            file(
                "web/validate_token.ts",
                "typescript",
                false,
                vec![chunk(
                    "function",
                    (1, 9),
                    "export function validateToken(token) { return validate(token); }",
                    Some("validateToken"),
                )],
            ),
            file(
                "README.md",
                "markdown",
                false,
                vec![chunk(
                    "section",
                    (1, 2),
                    "# App\nValidates the token.",
                    None,
                )],
            ),
        ];
        for doc in &docs {
            store.write_document(app, doc).unwrap();
        }
        let old = "a".repeat(64);
        let current = store.upsert_source(&spec_source(SHA256, true)).unwrap();
        store
            .write_document(current, &spec("The jti MUST be unique."))
            .unwrap();
        let older = store.upsert_source(&spec_source(&old, false)).unwrap();
        store
            .write_document(older, &spec("The jti MUST be unique, said the old one."))
            .unwrap();
        store.set_indexed_at(30).unwrap();
        drop(store);
        let store = Store::open_read_only(&dir.path().join("demo.db")).unwrap();
        Fixture { store, dir }
    }

    /// Moves the corpus's `indexed_at` back by `days`.
    fn backdate(path: &Path, days: u32) {
        rusqlite::Connection::open(path)
            .unwrap()
            .execute(
                "UPDATE meta SET value = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)
                 WHERE key = 'indexed_at'",
                [format!("-{days} days")],
            )
            .unwrap();
    }

    #[test]
    fn a_spec_result_names_its_documents_older_versions() {
        let f = fixture();
        let results = f.search("jti unique");
        let spec = results.iter().find(|r| r.role == "spec").unwrap();
        assert_eq!(
            spec.older_versions.as_deref(),
            Some(&["rfc9449@sha256:aaaaaaaaaaaa".to_owned()][..])
        );
        assert!(
            results
                .iter()
                .filter(|r| r.role != "spec")
                .all(|r| r.older_versions.is_none())
        );
    }

    #[test]
    fn an_exact_symbol_name_comes_first() {
        let f = fixture();
        let results = f.search("validate");
        assert_eq!(results[0].symbol.as_deref(), Some("validate"));
        assert_eq!(
            results[0].citation,
            format!("app@{HEAD}:src/validate.rs#L5-L9")
        );
        // The printed score is the one the order uses.
        let scores: Vec<f64> = results.iter().map(|r| r.score).collect();
        assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
    }

    #[test]
    fn a_symbol_named_by_any_query_word_comes_first() {
        let f = fixture();
        for text in ["validate rust", "how does validate work", "VALIDATE"] {
            let results = f.search(text);
            assert_eq!(results[0].symbol.as_deref(), Some("validate"), "{text}");
        }
        // The words run together name `validateToken`, and `_` is ignored.
        assert_eq!(
            f.search("validate_token")[0].symbol.as_deref(),
            Some("validateToken")
        );
    }

    #[test]
    fn source_outranks_the_tests_and_examples_that_use_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store.upsert_source(&code_source("app")).unwrap();
        // The same text in each, so only the role tells them apart, and an
        // unrelated file so the term is not in every document.
        let text = "app.add_middleware(TokenMiddleware)";
        let docs = [
            ("tests/test_app.py", text),
            ("examples/app.py", text),
            ("src/app.py", text),
            ("src/other.py", "def unrelated(): pass"),
        ];
        for (path, text) in docs {
            let doc = file(
                path,
                "python",
                false,
                vec![chunk("window", (1, 1), text, None)],
            );
            store.write_document(app, &doc).unwrap();
        }
        store.set_indexed_at(30).unwrap();
        let results = store.search(&query("add middleware")).unwrap();
        let roles: Vec<&str> = results.iter().map(|r| r.role).collect();
        assert_eq!(roles, ["source", "example", "test"], "{results:?}");
    }

    #[test]
    fn a_symbol_match_outranks_the_same_words_in_text() {
        let f = fixture();
        let results = f.search("validate token");
        assert_eq!(results[0].symbol.as_deref(), Some("validateToken"));
    }

    #[test]
    fn results_carry_their_citation_license_and_freshness() {
        let f = fixture();
        let results = f.search("validate");
        let code = results
            .iter()
            .find(|r| r.symbol.as_deref() == Some("validate"))
            .unwrap();
        assert_eq!(code.kind, "function");
        assert_eq!(code.license_class, "adapt");
        assert_eq!(code.dirty, Some(true));
        assert_eq!(code.snippet, VALIDATE);
        assert!(!code.snippet_truncated);
        assert_eq!(code.indexed_at.len(), "2026-10-20T06:00:00Z".len());
        assert!(matches!(code.citation.parse(), Ok(Citation::Code(_))));

        let results = f.search("jti");
        assert_eq!(results.len(), 1, "only the current version");
        let step = &results[0];
        assert_eq!(
            step.citation,
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2"
        );
        assert!(matches!(step.citation.parse(), Ok(Citation::Spec(_))));
        assert_eq!(step.kind, "step");
        assert_eq!(step.license_class, "quote");
        assert_eq!(step.dirty, None);
        assert_eq!(step.snippet, "The jti MUST be unique.");
        let section = &f.search("checking dpop")[0];
        assert_eq!(section.citation, "rfc9449@sha256:1a2b3c4d5e6f#section-4.3");
    }

    #[test]
    fn json_has_the_documented_shape() {
        let f = fixture();
        let code = serde_json::to_value(&f.search("validate")[0]).unwrap();
        assert_eq!(
            code["citation"],
            format!("app@{HEAD}:src/validate.rs#L5-L9")
        );
        assert_eq!(code["symbol"], "validate");
        assert_eq!(code["license_class"], "adapt");
        assert_eq!(code["dirty"], true);
        assert!(code["score"].is_f64());
        assert_eq!(code["role"], "source");
        let spec = serde_json::to_value(&f.search("jti")[0]).unwrap();
        // A `Value`'s keys are sorted. A spec result has no symbol or dirty;
        // this one's document has an older version pinned.
        let keys: Vec<&String> = spec.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "citation",
                "indexed_at",
                "kind",
                "license_class",
                "older_versions",
                "role",
                "score",
                "snippet",
                "snippet_truncated"
            ]
        );
    }

    #[test]
    fn filters_narrow_the_results() {
        let f = fixture();
        let by_language = SearchQuery {
            language: Some("typescript"),
            ..query("validate")
        };
        assert_eq!(
            f.citations(&by_language),
            [format!("app@{HEAD}:web/validate_token.ts#L1-L9")]
        );
        let by_path = SearchQuery {
            path: Some("src/*"),
            ..query("validate")
        };
        let paths = f.citations(&by_path);
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert!(paths.iter().all(|c| c.contains(":src/")), "{paths:?}");
        let limited = SearchQuery {
            limit: 1,
            ..query("validate")
        };
        assert_eq!(f.citations(&limited).len(), 1);
        let nothing = SearchQuery {
            language: Some("go"),
            ..query("validate")
        };
        assert!(f.citations(&nothing).is_empty());
    }

    #[test]
    fn only_keeps_spec_or_code_results() {
        let f = fixture();
        let specs = SearchQuery {
            only: Some(Scope::Spec),
            ..query("validate jti")
        };
        assert_eq!(
            f.citations(&specs),
            ["rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2"]
        );
        let code = SearchQuery {
            only: Some(Scope::Code),
            ..query("validate jti")
        };
        let citations = f.citations(&code);
        assert!(!citations.is_empty());
        assert!(
            citations.iter().all(|c| c.starts_with("app@")),
            "{citations:?}"
        );
    }

    /// A store where short code chunks that repeat `token_type_hint`
    /// outscore the long spec section that defines it once.
    fn buried_spec() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store.upsert_source(&code_source("app")).unwrap();
        for i in 0..4 {
            let path = format!("src/c{i}.py");
            let text = "token_type_hint = 'token_type_hint'";
            let doc = file(
                &path,
                "python",
                false,
                vec![chunk("window", (1, 1), text, None)],
            );
            store.write_document(app, &doc).unwrap();
        }
        let defines = "The client MAY pass a token_type_hint parameter. \
            The authorization server looks up the token and revokes it; \
            an unknown hint is ignored and the server searches all types.";
        let doc = file(
            "src/other.py",
            "python",
            false,
            vec![chunk("window", (1, 1), "def unrelated(): pass", None)],
        );
        store.write_document(app, &doc).unwrap();
        let spec_id = store.upsert_source(&spec_source(SHA256, true)).unwrap();
        store.write_document(spec_id, &spec(defines)).unwrap();
        store.set_indexed_at(30).unwrap();
        (dir, store)
    }

    #[test]
    fn a_spec_section_is_not_buried_under_code_that_repeats_its_term() {
        let (_dir, store) = buried_spec();
        let results = store.search(&query("token_type_hint")).unwrap();
        assert_eq!(results[0].role, "spec", "{results:?}");
        // On score alone, every code chunk beats it.
        let mut code = results.iter().filter(|r| r.role != "spec");
        assert!(code.clone().count() > 1);
        assert!(code.all(|r| r.score > results[0].score), "{results:?}");
    }

    #[test]
    fn a_query_that_names_a_symbol_keeps_score_order_with_a_spec_slot() {
        // Named results keep score order but hold the last slots for the
        // best spec results.
        let r = |spec: bool, score: f64| (spec, score);
        let code: Vec<_> = (0..5).map(|i| r(false, 10.0 - f64::from(i))).collect();
        let spec = vec![r(true, 1.0)];
        let blended = blend(code.clone(), spec.clone(), 3, true, |x| x.1);
        assert_eq!(blended, [r(false, 10.0), r(false, 9.0), r(true, 1.0)]);
        // A spec that already makes the cut keeps its place.
        let high = vec![r(true, 9.5)];
        let blended = blend(code.clone(), high, 3, true, |x| x.1);
        assert_eq!(blended, [r(false, 10.0), r(true, 9.5), r(false, 9.0)]);
        // At limit 10, all three held slots go to the three best specs.
        let code: Vec<_> = (0..20).map(|i| r(false, 100.0 - f64::from(i))).collect();
        let spec: Vec<_> = (0..5).map(|i| r(true, 3.0 - f64::from(i) * 0.5)).collect();
        let blended = blend(code.clone(), spec.clone(), 10, true, |x| x.1);
        assert_eq!(blended[..7], code[..7]);
        assert_eq!(blended[7..], spec[..3]);
        // A query that names nothing alternates, spec first.
        let code: Vec<_> = (0..5).map(|i| r(false, 10.0 - f64::from(i))).collect();
        let spec = vec![r(true, 1.0), r(true, 0.5)];
        let blended = blend(code.clone(), spec, 4, false, |x| x.1);
        assert_eq!(
            blended,
            [r(true, 1.0), r(false, 10.0), r(true, 0.5), r(false, 9.0)]
        );
        // With one kind, it is that kind in order.
        assert_eq!(blend(code.clone(), vec![], 2, false, |x| x.1), code[..2]);
        assert_eq!(blend(vec![], code.clone(), 2, true, |x| x.1), code[..2]);
        // A huge limit returns what there is without reserving room for it.
        for named in [true, false] {
            let blended = blend(code.clone(), vec![], usize::MAX, named, |x| x.1);
            assert_eq!(blended, code);
        }
    }

    #[test]
    fn query_names_are_each_word_and_all_of_them() {
        assert_eq!(query_names("validate"), ["validate"]);
        assert_eq!(
            query_names("Token-Validation  middleware!"),
            ["tokenvalidation", "middleware", "tokenvalidationmiddleware"]
        );
        assert!(query_names("--").is_empty());
        assert_eq!(query_names("foo bar foo"), ["foo", "bar", "foobarfoo"]);
    }

    #[test]
    fn a_name_many_helpers_share_is_not_boosted() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store.upsert_source(&code_source("app")).unwrap();
        // Four test helpers named `app`, and the source that says it most.
        for i in 0..4 {
            let path = format!("src/h{i}.py");
            let name = format!("codecite . app . h{i}/app().");
            let mut helper = chunk("function", (1, 1), "def app(): return 1", None);
            helper.symbol = Some(SymbolRecord {
                id: name,
                display_name: "app",
                kind: "function",
            });
            store
                .write_document(app, &file(&path, "python", false, vec![helper]))
                .unwrap();
        }
        let mut create = chunk("function", (1, 1), "def create(): return App()", None);
        create.symbol = Some(SymbolRecord {
            id: "codecite . app . main/create().".into(),
            display_name: "create",
            kind: "function",
        });
        store
            .write_document(app, &file("src/main.py", "python", false, vec![create]))
            .unwrap();
        store.set_indexed_at(30).unwrap();
        // `search` registers the name function `boosted_names` needs.
        store.search(&query("app")).unwrap();
        assert_eq!(store.boosted_names("app create").unwrap(), ["create"]);
    }

    /// A corpus with a repository README split at its headings and a
    /// fetched page that is not a spec, stored with its text, next to the
    /// spec fixture.
    fn documents() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store.upsert_source(&code_source("app")).unwrap();
        let readme = ChunkRecord {
            anchor: Some("install"),
            title: Some("Install"),
            ..chunk(
                "section",
                (4, 7),
                "## Install\nRun cargo install widget.",
                None,
            )
        };
        store
            .write_document(app, &file("README.md", "markdown", false, vec![readme]))
            .unwrap();
        let page_text = "# Notes\n\n## Rollout\nShip the widget behind a flag.\n";
        let rollout = ChunkRecord {
            anchor: Some("rollout"),
            title: Some("Rollout"),
            ..chunk(
                "section",
                (3, 4),
                "## Rollout\nShip the widget behind a flag.",
                None,
            )
        };
        let notes = store
            .upsert_source(&SourceRecord {
                name: "notes",
                ..spec_source(SHA256, true)
            })
            .unwrap();
        let page = DocumentRecord {
            path: "notes",
            language: Some("markdown"),
            text: Some(page_text),
            ..file("notes", "markdown", false, vec![rollout])
        };
        store.write_document(notes, &page).unwrap();
        let rfc = store
            .upsert_source(&spec_source(&"b".repeat(64), true))
            .unwrap();
        store
            .write_document(rfc, &spec("The widget MUST be unique."))
            .unwrap();
        store.set_indexed_at(30).unwrap();
        drop(store);
        let store = Store::open_read_only(&dir.path().join("demo.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn a_repository_heading_hit_carries_its_title_and_anchor() {
        let (_dir, store) = documents();
        let results = store.search(&query("install")).unwrap();
        let readme = &results[0];
        assert_eq!(readme.citation, format!("app@{HEAD}:README.md#L4-L7"));
        assert_eq!(readme.kind, "section");
        assert_eq!(readme.title.as_deref(), Some("Install"));
        assert_eq!(readme.anchor.as_deref(), Some("install"));
        let json = serde_json::to_value(readme).unwrap();
        assert_eq!(
            (&json["title"], &json["anchor"]),
            (&"Install".into(), &"install".into())
        );
    }

    #[test]
    fn a_generic_document_hit_is_cited_by_its_lines() {
        let (_dir, store) = documents();
        let results = store.search(&query("rollout flag")).unwrap();
        let page = results
            .iter()
            .find(|r| r.citation.starts_with("notes@"))
            .expect("the page is searched");
        assert_eq!(page.citation, "notes@sha256:1a2b3c4d5e6f#L3-L4");
        assert!(matches!(page.citation.parse(), Ok(Citation::Document(_))));
        assert_eq!(page.kind, "section");
        assert_eq!(page.role, "docs");
        assert_eq!(page.title.as_deref(), Some("Rollout"));
        assert_eq!(page.anchor.as_deref(), Some("rollout"));
        assert_eq!(page.dirty, None);
        // The page is not a spec, so `only = spec` leaves it out and
        // `only = code` keeps it with the repository's documents.
        let only = |scope| {
            let q = SearchQuery {
                only: Some(scope),
                ..query("widget")
            };
            store
                .search(&q)
                .unwrap()
                .into_iter()
                .map(|r| r.citation)
                .collect::<Vec<_>>()
        };
        let specs = only(Scope::Spec);
        assert_eq!(
            specs,
            [format!(
                "rfc9449@sha256:{}#section-4.3/step-2",
                "b".repeat(12)
            )]
        );
        let rest = only(Scope::Code);
        assert!(rest.iter().any(|c| c.starts_with("notes@")), "{rest:?}");
        assert!(!rest.iter().any(|c| c.starts_with("rfc9449@")), "{rest:?}");
    }

    #[test]
    fn a_spec_hit_carries_no_title_or_anchor() {
        let (_dir, store) = documents();
        let q = SearchQuery {
            only: Some(Scope::Spec),
            ..query("widget")
        };
        let spec = &store.search(&q).unwrap()[0];
        assert_eq!(
            (spec.title.as_deref(), spec.anchor.as_deref()),
            (None, None)
        );
        let json = serde_json::to_value(spec).unwrap();
        assert!(json.get("title").is_none() && json.get("anchor").is_none());
    }

    #[test]
    fn query_syntax_is_searched_as_words() {
        let f = fixture();
        for text in [
            "validate AND",
            "NEAR(validate",
            "path:validate",
            "\"validate*",
        ] {
            let results = f.store.search(&query(text)).unwrap();
            assert!(!results.is_empty(), "{text}");
        }
        assert_eq!(
            fts_query("say \"hi\" NEAR x*").as_deref(),
            Some("\"say\" OR \"hi\" OR \"near\" OR \"x\"")
        );
    }

    #[test]
    fn spec_queries_drop_stopwords() {
        assert_eq!(
            spec_fts_query("Is client authentication required?").as_deref(),
            Some("\"client\" OR \"authentication\" OR \"required\"")
        );
        // Requirement keywords and their negation stay: they say which
        // requirement a query is after.
        assert_eq!(
            spec_fts_query("what MUST NOT the client send").as_deref(),
            Some("\"must\" OR \"not\" OR \"client\" OR \"send\"")
        );
        assert_eq!(
            spec_fts_query("what SHOULD it do if no SHALL").as_deref(),
            Some("\"should\" OR \"no\" OR \"shall\"")
        );
        // Only stopwords: all of them, joined with AND.
        assert_eq!(
            spec_fts_query("what is this").as_deref(),
            Some("\"what\" AND \"is\" AND \"this\"")
        );
        assert_eq!(spec_fts_query("-- !!"), None);
        // Code queries keep every term.
        assert_eq!(
            fts_query("Is client authentication required?").as_deref(),
            Some("\"is\" OR \"client\" OR \"authentication\" OR \"required\"")
        );
    }

    #[test]
    fn stopwords_are_sorted_lowercase_and_unique() {
        assert!(STOPWORDS.windows(2).all(|w| w[0] < w[1]), "{STOPWORDS:?}");
        assert!(
            STOPWORDS
                .iter()
                .all(|w| w.chars().all(|c| c.is_ascii_lowercase()))
        );
    }

    /// A store with three spec sections and no code.
    fn specs(sections: &[&'static str]) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let id = store.upsert_source(&spec_source(SHA256, true)).unwrap();
        let numbers = ["1", "2", "3", "4", "5"];
        let chunks = sections
            .iter()
            .zip(numbers)
            .map(|(&text, number)| ChunkRecord {
                number: Some(number),
                ..chunk("section", (1, 1), text, None)
            })
            .collect();
        let doc = DocumentRecord {
            chunks,
            ..spec("unused")
        };
        store.write_document(id, &doc).unwrap();
        store.set_indexed_at(30).unwrap();
        (dir, store)
    }

    fn spec_citations(store: &Store, text: &str) -> Vec<String> {
        let results = store.search(&query(text)).unwrap();
        results.into_iter().map(|r| r.citation).collect()
    }

    #[test]
    fn spec_search_is_stemmed_and_ignores_stopwords() {
        let (_dir, store) = specs(&[
            "The client authenticates with the authorization server.",
            "It is what it is: this is the section that says is and the.",
            "Clients are registered by the server.",
        ]);
        let section = |n: &str| format!("rfc9449@sha256:1a2b3c4d5e6f#section-{n}");
        // "authentication" finds "authenticates", and "is" doesn't pull in
        // every section that says it.
        assert_eq!(
            spec_citations(&store, "is client authentication required"),
            [section("1"), section("3")]
        );
        // A query of only stopwords finds the sections that hold all of
        // them, not every section.
        assert_eq!(spec_citations(&store, "what is this"), [section("2")]);
    }

    #[test]
    fn code_search_is_not_stemmed_and_keeps_stopwords() {
        let f = fixture();
        let code = |text| SearchQuery {
            only: Some(Scope::Code),
            ..query(text)
        };
        // The README says "Validates the token." Unstemmed, "validated" is
        // another word.
        assert!(f.citations(&code("validated")).is_empty());
        assert_eq!(
            f.citations(&code("validates")),
            [format!("app@{HEAD}:README.md#L1-L2")]
        );
        // "the" is a stopword only for specs.
        assert_eq!(
            f.citations(&code("the")),
            [format!("app@{HEAD}:README.md#L1-L2")]
        );
    }

    /// Code results as the search ranked them before specs had their own
    /// table: one query over `chunk_fts`, code and spec chunks together.
    fn code_as_before(store: &Store, text: &str) -> Vec<(String, f64)> {
        let mut names = String::from(NAME_SEPARATOR);
        for name in store.boosted_names(text).unwrap() {
            names.push_str(&name);
            names.push(NAME_SEPARATOR);
        }
        let (w_symbol, w_path, w_text) = COLUMN_WEIGHTS;
        let mut stmt = store
            .conn
            .prepare(
                "SELECT s.name, s.revision, d.path, c.start_line, c.end_line,
                   -bm25(chunk_fts, ?3, ?4, ?5)
                     * codecite_role_weight(d.path, s.kind, d.text IS NOT NULL)
                     * CASE WHEN instr(?2, char(31) || codecite_name_key(sym.display_name) || char(31)) > 0
                            THEN ?6 ELSE 1.0 END AS score
                 FROM chunk_fts
                 JOIN chunk c ON c.id = chunk_fts.rowid
                 JOIN document d ON d.id = c.document_id
                 JOIN source s ON s.id = d.source_id
                 LEFT JOIN symbol sym ON sym.id = c.symbol_id
                 WHERE chunk_fts MATCH ?1 AND s.is_current AND s.kind <> 'document'
                 ORDER BY score DESC, s.name, d.path, c.start_line, c.end_line",
            )
            .unwrap();
        let rows = stmt
            .query_map(
                params![
                    fts_query(text).unwrap(),
                    names,
                    w_symbol,
                    w_path,
                    w_text,
                    NAME_BOOST
                ],
                |row| {
                    let citation = code_citation(
                        &row.get::<_, String>(0)?,
                        &row.get::<_, String>(1)?,
                        &row.get::<_, String>(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    );
                    Ok((citation, row.get(5)?))
                },
            )
            .unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn code_results_are_ranked_as_before_specs_had_their_own_table() {
        let f = fixture();
        // Spec chunks, both versions, stay in chunk_fts, so code's BM25
        // statistics are what they were.
        let spec_rows: i64 = f
            .store
            .conn
            .query_row(
                "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH 'jti'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(spec_rows, 2);
        for text in [
            "validate",
            "validate token",
            "is the token validated",
            "what is this",
            "jti checking dpop",
        ] {
            let code = SearchQuery {
                only: Some(Scope::Code),
                ..query(text)
            };
            let now: Vec<(String, f64)> = f
                .store
                .search(&code)
                .unwrap()
                .into_iter()
                .map(|r| (r.citation, r.score))
                .collect();
            assert_eq!(now, code_as_before(&f.store, text), "{text}");
        }
    }

    #[test]
    fn an_empty_query_is_an_error() {
        let f = fixture();
        for text in ["", "   ", "-- !!"] {
            let err = f.store.search(&query(text)).unwrap_err();
            assert!(matches!(err, Error::EmptyQuery), "{text:?}: {err:?}");
        }
    }

    #[test]
    fn a_stale_index_is_refused_unless_allowed() {
        let f = fixture();
        backdate(&f.path(), 31);
        let err = f.store.search(&query("validate")).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::StaleIndex { corpus, max_age_days: 30, .. } if corpus == "demo"
            ),
            "{err:?}"
        );
        let stale = SearchQuery {
            allow_stale: true,
            ..query("validate")
        };
        let results = f.store.search(&stale).unwrap();
        assert!(!results.is_empty());

        backdate(&f.path(), 29);
        assert!(f.store.search(&query("validate")).is_ok());
    }

    #[test]
    fn the_stored_max_age_wins_and_defaults_to_30() {
        let f = fixture();
        backdate(&f.path(), 8);
        f.raw()
            .execute("UPDATE meta SET value = '7' WHERE key = 'max_age_days'", [])
            .unwrap();
        assert!(f.store.freshness().unwrap().is_stale());
        f.raw()
            .execute("DELETE FROM meta WHERE key = 'max_age_days'", [])
            .unwrap();
        let freshness = f.store.freshness().unwrap();
        assert_eq!(freshness.max_age_days, DEFAULT_MAX_AGE_DAYS);
        assert!(!freshness.is_stale());
    }

    #[test]
    fn a_corpus_never_indexed_is_invalid() {
        let f = fixture();
        f.raw()
            .execute("DELETE FROM meta WHERE key = 'indexed_at'", [])
            .unwrap();
        let err = f.store.search(&query("validate")).unwrap_err();
        assert!(matches!(err, Error::InvalidCorpus { .. }), "{err:?}");
    }

    #[test]
    fn snippets_stop_at_40_lines() {
        let text: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        let (s, truncated) = snippet(&text);
        assert_eq!(s.lines().count(), MAX_SNIPPET_LINES);
        assert!(s.ends_with("line 40"));
        assert!(truncated);
        let forty: String = (1..=40).map(|i| format!("line {i}\n")).collect();
        assert!(!snippet(&forty).1);
    }

    #[test]
    fn snippets_stop_at_the_byte_budget_on_a_character_boundary() {
        // One long line, with a two-byte `é` across the budget.
        let text = format!("a{}", "é".repeat(MAX_SNIPPET_BYTES));
        let (s, truncated) = snippet(&text);
        assert!(truncated);
        assert_eq!(s.len(), MAX_SNIPPET_BYTES - 1);
        assert!(text.starts_with(&s));
        // CRLF lines are joined with `\n`, and the cut counts those bytes.
        let crlf = format!("a\r\n{}", "b".repeat(MAX_SNIPPET_BYTES));
        let (s, truncated) = snippet(&crlf);
        assert!(truncated);
        assert_eq!(s.len(), MAX_SNIPPET_BYTES);
        assert!(s.starts_with("a\nbbb"));
        // Text within the budget is whole.
        let short = "é".repeat(MAX_SNIPPET_BYTES / 2);
        assert_eq!(snippet(&short), (short.clone(), false));
    }
}
