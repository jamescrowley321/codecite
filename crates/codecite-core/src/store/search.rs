//! Searching a corpus: BM25 over the full-text index, weighted by what each
//! document is for and lifted for a symbol a query word names, with each
//! result cited.

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

/// Which sources a search covers: specs (document sources) or code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Only spec sections and steps.
    Spec,
    /// Only code: repositories and local directories.
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
    /// Where the chunk is: `repo@sha:path#L10-L42`, or
    /// `doc@sha256:<12 hex>#section-4.3[/step-2]` for a spec.
    pub citation: String,
    /// `function`, `class`, `variable`, `type_alias`, `section`, `step` or
    /// `window`.
    pub kind: String,
    /// The name of the definition the chunk holds, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
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
    /// was indexed. Spec results have none.
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
        let only_spec = query.only.map(|scope| scope == Scope::Spec);

        let (w_symbol, w_path, w_text) = COLUMN_WEIGHTS;
        // The best `?8` of each kind, specs after code.
        let mut stmt = self.conn.prepare_cached(
            "WITH m AS (
               SELECT c.id AS cid, c.kind, c.start_line, c.end_line, c.number,
                 p.number AS parent_number, c.text, d.path, d.dirty, s.name,
                 s.kind AS source_kind, s.revision, s.license_class, s.indexed_at,
                 sym.display_name,
                 -bm25(chunk_fts, ?3, ?4, ?5)
                   * codecite_role_weight(d.path, s.kind)
                   * CASE WHEN instr(?2, char(31) || codecite_name_key(sym.display_name) || char(31)) > 0
                          THEN ?9 ELSE 1.0 END AS score,
                 s.kind = 'document' AS is_spec
               FROM chunk_fts
               JOIN chunk c ON c.id = chunk_fts.rowid
               JOIN document d ON d.id = c.document_id
               JOIN source s ON s.id = d.source_id
               LEFT JOIN chunk p ON p.id = c.parent_id
               LEFT JOIN symbol sym ON sym.id = c.symbol_id
               WHERE chunk_fts MATCH ?1 AND s.is_current
                 AND (?6 IS NULL OR d.language = ?6)
                 AND (?7 IS NULL OR d.path GLOB ?7)
                 AND (?10 IS NULL OR (s.kind = 'document') = ?10)
             )
             SELECT kind, start_line, end_line, number, parent_number, text, path, dirty,
               name, source_kind, revision, license_class, indexed_at, display_name, score
             FROM (SELECT *, row_number() OVER (PARTITION BY is_spec ORDER BY score DESC, cid) AS kind_rank
                   FROM m)
             WHERE kind_rank <= ?8
             ORDER BY is_spec, kind_rank",
        )?;
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
                only_spec,
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
                let is_spec = source_kind == "document";
                let (snippet, snippet_truncated) = snippet(&text);
                let role = Role::of(&path, &source_kind).as_str();
                let citation = if is_spec {
                    spec_citation(&source, &revision, &kind, number, parent_number)
                } else {
                    code_citation(&source, &revision, &path, start, end)
                };
                Ok(SearchResult {
                    citation,
                    kind,
                    symbol: row.get(13)?,
                    role,
                    score: row.get(14)?,
                    license_class: row.get(11)?,
                    indexed_at: row.get(12)?,
                    dirty: (!is_spec).then_some(dirty),
                    snippet,
                    snippet_truncated,
                })
            },
        )?;
        let (spec, code): (Vec<_>, Vec<_>) = rows
            .collect::<rusqlite::Result<Vec<SearchResult>>>()?
            .into_iter()
            .partition(|r| r.role == Role::Spec.as_str());
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
        Ok(blend(code, spec, limit, named, |r| r.score))
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
/// connection: `codecite_role_weight(path, source_kind)` and
/// `codecite_name_key(name)`.
pub(super) fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "codecite_role_weight",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let path: String = ctx.get(0)?;
            let source_kind: String = ctx.get(1)?;
            Ok(Role::of(&path, &source_kind).weight())
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
    // The tokenizer folds case, so `NEAR` and `near` are one term.
    let terms = text_terms(text).to_lowercase();
    let mut seen: Vec<&str> = Vec::new();
    for term in terms.split(' ').filter(|t| !t.is_empty()) {
        if !seen.contains(&term) {
            seen.push(term);
        }
    }
    if seen.is_empty() {
        return None;
    }
    let quoted: Vec<String> = seen
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    Some(quoted.join(" OR "))
}

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
fn name_key(name: &str) -> String {
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
fn snippet(text: &str) -> (String, bool) {
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
        // A `Value`'s keys are sorted. A spec result has no symbol or dirty.
        let keys: Vec<&String> = spec.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "citation",
                "indexed_at",
                "kind",
                "license_class",
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
