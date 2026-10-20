//! Searching a corpus: BM25 over the full-text index, with an exact symbol
//! name first, and each result cited.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::Store;
use crate::{Error, Result, text_terms};

/// The most lines of a chunk a result's snippet holds. Agents read the
/// rest through its citation.
pub const MAX_SNIPPET_LINES: usize = 40;

/// How many days an index stays fresh when its manifest doesn't say.
pub const DEFAULT_MAX_AGE_DAYS: u32 = 30;

/// The `bm25()` weight of each full-text column, in the table's order:
/// a match in a symbol's name counts most, then one in the path, then one
/// in the text.
const COLUMN_WEIGHTS: (f64, f64, f64) = (10.0, 5.0, 1.0);

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
    /// The most results to return.
    pub limit: u32,
    /// Search an index older than its maximum age instead of refusing it.
    pub allow_stale: bool,
}

/// One ranked result, in the shape the CLI prints as JSON and the MCP
/// server returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    /// Where the chunk is: `repo@sha:path#L10-L42`, or
    /// `doc@sha256:<12 hex>#section-4.3[/step-2]` for a spec.
    pub citation: String,
    /// `function`, `class`, `section`, `step` or `window`.
    pub kind: String,
    /// The name of the definition the chunk holds, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// The BM25 score: higher is a better match.
    pub score: f64,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    pub license_class: String,
    /// When the chunk's source was last indexed.
    pub indexed_at: String,
    /// For code, whether the file differed from the cited commit when it
    /// was indexed. Spec results have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    /// The chunk's first [`MAX_SNIPPET_LINES`] lines.
    pub snippet: String,
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
    /// splits text, and a chunk matches when it holds any of them; BM25
    /// ranks the matches, weighting the symbol column, then the path, then
    /// the text. A chunk whose symbol's name is exactly the query comes
    /// first. Only the current version of each document is searched.
    ///
    /// A query with no letter or digit is [`Error::EmptyQuery`]. An index
    /// older than its maximum age is [`Error::StaleIndex`] unless
    /// `query.allow_stale` is set.
    pub fn search(&self, query: &SearchQuery<'_>) -> Result<Vec<SearchResult>> {
        let fts = fts_query(query.text).ok_or(Error::EmptyQuery)?;
        let freshness = self.freshness()?;
        if freshness.is_stale() && !query.allow_stale {
            return Err(Error::StaleIndex {
                corpus: self.corpus_name(),
                indexed_at: freshness.indexed_at,
                max_age_days: freshness.max_age_days,
            });
        }

        let (w_symbol, w_path, w_text) = COLUMN_WEIGHTS;
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.kind, c.start_line, c.end_line, c.number, p.number, c.text,
               d.path, d.dirty, s.name, s.kind, s.revision, s.license_class, s.indexed_at,
               sym.display_name,
               -bm25(chunk_fts, ?3, ?4, ?5) AS score,
               coalesce(sym.display_name = ?2, 0) AS exact
             FROM chunk_fts
             JOIN chunk c ON c.id = chunk_fts.rowid
             JOIN document d ON d.id = c.document_id
             JOIN source s ON s.id = d.source_id
             LEFT JOIN chunk p ON p.id = c.parent_id
             LEFT JOIN symbol sym ON sym.id = c.symbol_id
             WHERE chunk_fts MATCH ?1 AND s.is_current
               AND (?6 IS NULL OR d.language = ?6)
               AND (?7 IS NULL OR d.path GLOB ?7)
             ORDER BY exact DESC, score DESC, c.id
             LIMIT ?8",
        )?;
        let rows = stmt.query_map(
            params![
                fts,
                query.text.trim(),
                w_symbol,
                w_path,
                w_text,
                query.language,
                query.path,
                query.limit,
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
                let citation = if is_spec {
                    spec_citation(&source, &revision, &kind, number, parent_number)
                } else {
                    code_citation(&source, &revision, &path, start, end)
                };
                Ok(SearchResult {
                    citation,
                    kind,
                    symbol: row.get(13)?,
                    score: row.get(14)?,
                    license_class: row.get(11)?,
                    indexed_at: row.get(12)?,
                    dirty: (!is_spec).then_some(dirty),
                    snippet: snippet(&text),
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The corpus's name: its file's name without `.db`.
    fn corpus_name(&self) -> String {
        self.path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
    }
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

/// `repo@sha:path#L10-L42`, or `#L10` for one line.
fn code_citation(source: &str, revision: &str, path: &str, start: u32, end: u32) -> String {
    if start == end {
        format!("{source}@{revision}:{path}#L{start}")
    } else {
        format!("{source}@{revision}:{path}#L{start}-L{end}")
    }
}

/// `doc@sha256:<12 hex>#section-<n>`, or `…#section-<n>/step-<k>` for a step,
/// whose section is its parent's number.
fn spec_citation(
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

/// The first [`MAX_SNIPPET_LINES`] lines of a chunk's text.
fn snippet(text: &str) -> String {
    let lines: Vec<&str> = text.lines().take(MAX_SNIPPET_LINES).collect();
    lines.join("\n")
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
        // Without the boost, BM25 alone ranks a chunk above it.
        assert!(results[1..].iter().any(|r| r.score > results[0].score));
        let scores: Vec<f64> = results[1..].iter().map(|r| r.score).collect();
        assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
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
                "score",
                "snippet"
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
        let s = snippet(&text);
        assert_eq!(s.lines().count(), MAX_SNIPPET_LINES);
        assert!(s.ends_with("line 40"));
    }
}
