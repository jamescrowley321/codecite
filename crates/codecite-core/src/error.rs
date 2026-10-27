//! Errors returned by the store and the query engine.

use std::path::PathBuf;

use crate::CitationError;

/// An error from `codecite-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Reading or writing a file failed.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// The path does not hold a usable corpus.
    #[error("invalid corpus at {}: {reason}", path.display())]
    InvalidCorpus {
        /// Where the corpus was expected.
        path: PathBuf,
        /// Why it can't be used.
        reason: String,
    },

    /// A SQLite call failed.
    #[error("SQLite error")]
    Sqlite(#[from] rusqlite::Error),

    /// The corpus was written by a newer codecite, with a schema this one
    /// doesn't know.
    #[error(
        "corpus at {} has schema version {found}, but this codecite supports up to {supported}; upgrade codecite",
        path.display()
    )]
    SchemaTooNew {
        /// Where the corpus is.
        path: PathBuf,
        /// The schema version stored in the corpus.
        found: u32,
        /// The newest schema version this build supports.
        supported: u32,
    },

    /// A corpus name that isn't a plain file name.
    #[error(
        "invalid corpus name {name:?}: use ASCII letters, digits, `-`, `_` and `.`, not starting with `.`"
    )]
    InvalidCorpusName {
        /// The name as given.
        name: String,
    },

    /// No corpus file has this name.
    #[error("unknown corpus `{corpus}`; {}", known_corpora(known))]
    UnknownCorpus {
        /// The name asked for.
        corpus: String,
        /// The corpora that do exist, sorted.
        known: Vec<String>,
    },

    /// The caller named no corpus, and the data directory does not hold
    /// exactly one to use instead.
    #[error("{}", no_corpus_named(known))]
    NoCorpusNamed {
        /// The corpora that do exist, sorted.
        known: Vec<String>,
    },

    /// The corpus was last indexed longer ago than its manifest's
    /// `max_age_days` allows.
    #[error(
        "corpus `{corpus}` was indexed at {indexed_at}, more than {max_age_days} days ago; re-run codecite index, or allow a stale index"
    )]
    StaleIndex {
        /// The corpus's name.
        corpus: String,
        /// When it was last indexed.
        indexed_at: String,
        /// Its maximum age.
        max_age_days: u32,
    },

    /// The query has no letter or digit to search for.
    #[error("the query has no words to search for")]
    EmptyQuery,

    /// The text matches none of the citation forms `read` accepts.
    #[error(
        "{text:?} is not a citation; write repo@sha:path#L10-L42, doc@sha256:<hash>#section-4.3, a spec URL, or a form like RFC 9449 §4.3"
    )]
    NotACitation {
        /// The text as given.
        text: String,
    },

    /// The text has the shape of a citation but is malformed.
    #[error("invalid citation {text:?}")]
    BadCitation {
        /// The text as given.
        text: String,
        /// What is wrong with it.
        #[source]
        source: CitationError,
    },

    /// The corpus has no source or document with this name.
    #[error(
        "the corpus has no source or document `{name}`; {}",
        list_of("known", known)
    )]
    UnknownSource {
        /// The name the citation gives.
        name: String,
        /// The names the corpus does have, of the same kind, sorted.
        known: Vec<String>,
    },

    /// The source or document is in the corpus, but not at the version the
    /// citation names.
    #[error("`{name}` is not indexed at {version}; {}", list_of("indexed", known))]
    UnknownVersion {
        /// The source or document.
        name: String,
        /// The version the citation names: a commit, or `sha256:<hash>`.
        version: String,
        /// The versions that are indexed, as citations name them.
        known: Vec<String>,
    },

    /// A spec citation that names no document, like `§3.1.3.7`, is not in
    /// exactly one current document.
    #[error("{}", ambiguous(reference, candidates))]
    AmbiguousDocument {
        /// The citation as given.
        reference: String,
        /// The pinned citations of each current document that has it.
        candidates: Vec<String>,
    },

    /// The document version has no such section.
    #[error(
        "{document} has no section {section}; {}",
        list_of("nearest", candidates)
    )]
    NoSuchSection {
        /// The pinned document: `rfc9449@sha256:1a2b3c4d5e6f`.
        document: String,
        /// The section as cited: `4.9`, or `#anchor` for a URL's anchor.
        section: String,
        /// The nearest sections that do exist, as pinned citations.
        candidates: Vec<String>,
    },

    /// The section exists, but its number, like an appendix's `A.1`, has no
    /// canonical citation form, so no citation could read it back.
    #[error("{document} section {section} has no canonical citation form yet")]
    UncitableSection {
        /// The pinned document: `rfc9449@sha256:1a2b3c4d5e6f`.
        document: String,
        /// The section's number as the document writes it: `A.1`.
        section: String,
    },

    /// The section has no such step.
    #[error("{section} has no step {step}; {}", list_of("nearest", candidates))]
    NoSuchStep {
        /// The section, as a pinned citation.
        section: String,
        /// The step asked for.
        step: u32,
        /// The nearest steps that do exist, or the section itself when it
        /// has none, as pinned citations.
        candidates: Vec<String>,
    },

    /// The source has no indexed file at this path.
    #[error(
        "{repo} has no indexed file {path:?}; {}",
        list_of("candidates", candidates)
    )]
    NoSuchFile {
        /// The source at its commit: `app@0123abc`.
        repo: String,
        /// The path asked for.
        path: String,
        /// Indexed files with the same file name.
        candidates: Vec<String>,
    },

    /// No definition in the corpus has this name.
    #[error(
        "the corpus defines no symbol `{}`; {}",
        escape_controls(symbol),
        list_of("did you mean", candidates)
    )]
    NoSuchSymbol {
        /// The symbol as asked for.
        symbol: String,
        /// Defined names that differ from it only in case, `_` or `-`.
        candidates: Vec<String>,
    },

    /// A code citation runs past the end of its file.
    #[error("{citation} runs past the end of the file, which has {lines} lines")]
    LinesPastEnd {
        /// The citation, with the source's full commit.
        citation: String,
        /// How many lines the file has.
        lines: u32,
    },
}

impl Error {
    /// The variant's name in snake_case, such as `unknown_corpus`: what an
    /// MCP client matches on, since the message may change.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Io(_) => "io",
            Error::InvalidCorpus { .. } => "invalid_corpus",
            Error::Sqlite(_) => "sqlite",
            Error::SchemaTooNew { .. } => "schema_too_new",
            Error::InvalidCorpusName { .. } => "invalid_corpus_name",
            Error::UnknownCorpus { .. } => "unknown_corpus",
            Error::NoCorpusNamed { .. } => "no_corpus_named",
            Error::StaleIndex { .. } => "stale_index",
            Error::EmptyQuery => "empty_query",
            Error::NotACitation { .. } => "not_a_citation",
            Error::BadCitation { .. } => "bad_citation",
            Error::UnknownSource { .. } => "unknown_source",
            Error::UnknownVersion { .. } => "unknown_version",
            Error::AmbiguousDocument { .. } => "ambiguous_document",
            Error::NoSuchSection { .. } => "no_such_section",
            Error::UncitableSection { .. } => "uncitable_section",
            Error::NoSuchStep { .. } => "no_such_step",
            Error::NoSuchFile { .. } => "no_such_file",
            Error::NoSuchSymbol { .. } => "no_such_symbol",
            Error::LinesPastEnd { .. } => "lines_past_end",
        }
    }

    /// What the caller could have meant instead: the corpora, names,
    /// versions or pinned citations the error lists. Empty for an error
    /// that lists none.
    pub fn candidates(&self) -> &[String] {
        match self {
            Error::UnknownCorpus { known, .. }
            | Error::NoCorpusNamed { known }
            | Error::UnknownSource { known, .. }
            | Error::UnknownVersion { known, .. } => known,
            Error::AmbiguousDocument { candidates, .. }
            | Error::NoSuchSection { candidates, .. }
            | Error::NoSuchStep { candidates, .. }
            | Error::NoSuchFile { candidates, .. }
            | Error::NoSuchSymbol { candidates, .. } => candidates,
            _ => &[],
        }
    }
}

/// The end of an error message that lists candidates: `nearest: a, b`, or
/// `no candidates`. Candidates come from indexed text, such as file paths,
/// so their control characters are escaped before they reach a terminal.
fn list_of(label: &str, list: &[String]) -> String {
    if list.is_empty() {
        return "no candidates".into();
    }
    let escaped: Vec<String> = list.iter().map(|c| escape_controls(c)).collect();
    format!("{label}: {}", escaped.join(", "))
}

/// `text` with each control character but a tab written as `\u{..}`.
fn escape_controls(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\t' {
                c.escape_unicode().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// The message of an [`Error::AmbiguousDocument`].
fn ambiguous(reference: &str, list: &[String]) -> String {
    if list.is_empty() {
        format!("no current document has {reference}")
    } else {
        format!(
            "{reference} names no document and is not in exactly one current document; name one: {}",
            list.join(", ")
        )
    }
}

/// The end of an [`Error::UnknownCorpus`] message.
fn known_corpora(known: &[String]) -> String {
    if known.is_empty() {
        "no corpus has been indexed yet; run codecite index".into()
    } else {
        format!("known corpora: {}", known.join(", "))
    }
}

/// The message of an [`Error::NoCorpusNamed`].
fn no_corpus_named(known: &[String]) -> String {
    if known.is_empty() {
        known_corpora(known)
    } else {
        format!("name a corpus; {}", known_corpora(known))
    }
}

/// `Result` with [`Error`] as the default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::io;

    fn read_missing() -> Result<()> {
        std::fs::read("/nonexistent/codecite-test")?;
        Ok(())
    }

    #[test]
    fn io_error_converts_with_question_mark() {
        assert!(matches!(read_missing(), Err(Error::Io(_))));
    }

    #[test]
    fn io_display_omits_source() {
        let err = Error::from(io::Error::other("disk on fire"));
        assert_eq!(err.to_string(), "I/O error");
    }

    #[test]
    fn io_source_is_the_io_error() {
        let err = Error::from(io::Error::other("disk on fire"));
        assert_eq!(err.source().unwrap().to_string(), "disk on fire");
    }

    #[test]
    fn invalid_corpus_display_names_path_and_reason() {
        let err = Error::InvalidCorpus {
            path: "/tmp/corpus".into(),
            reason: "not a codecite store".into(),
        };
        assert_eq!(
            err.to_string(),
            "invalid corpus at /tmp/corpus: not a codecite store"
        );
        assert!(err.source().is_none());
    }

    #[test]
    fn sqlite_error_converts_and_is_the_source() {
        let inner = rusqlite::Error::InvalidQuery;
        let msg = inner.to_string();
        let err = Error::from(inner);
        assert!(matches!(err, Error::Sqlite(_)));
        assert_eq!(err.to_string(), "SQLite error");
        assert_eq!(err.source().unwrap().to_string(), msg);
    }

    #[test]
    fn unknown_corpus_lists_the_known_ones() {
        let err = Error::UnknownCorpus {
            corpus: "identty".into(),
            known: vec!["identity".into(), "specs".into()],
        };
        assert_eq!(
            err.to_string(),
            "unknown corpus `identty`; known corpora: identity, specs"
        );
        let err = Error::UnknownCorpus {
            corpus: "identity".into(),
            known: vec![],
        };
        assert_eq!(
            err.to_string(),
            "unknown corpus `identity`; no corpus has been indexed yet; run codecite index"
        );
    }

    #[test]
    fn no_corpus_named_lists_the_corpora_to_choose_from() {
        let err = Error::NoCorpusNamed {
            known: vec!["identity".into(), "specs".into()],
        };
        assert_eq!(
            err.to_string(),
            "name a corpus; known corpora: identity, specs"
        );
        assert_eq!(err.kind(), "no_corpus_named");
        assert_eq!(err.candidates(), ["identity", "specs"]);
        let err = Error::NoCorpusNamed { known: vec![] };
        assert_eq!(
            err.to_string(),
            "no corpus has been indexed yet; run codecite index"
        );
    }

    #[test]
    fn each_error_has_a_kind_and_its_candidates() {
        let err = Error::NoSuchSection {
            document: "rfc9449@sha256:1a2b3c4d5e6f".into(),
            section: "4.9".into(),
            candidates: vec!["rfc9449@sha256:1a2b3c4d5e6f#section-4.3".into()],
        };
        assert_eq!(err.kind(), "no_such_section");
        assert_eq!(
            err.candidates(),
            ["rfc9449@sha256:1a2b3c4d5e6f#section-4.3"]
        );
        let err = Error::StaleIndex {
            corpus: "demo".into(),
            indexed_at: "2026-01-01T00:00:00Z".into(),
            max_age_days: 7,
        };
        assert_eq!(err.kind(), "stale_index");
        assert!(err.candidates().is_empty());
        assert_eq!(Error::EmptyQuery.kind(), "empty_query");
        let err = Error::from(io::Error::other("disk on fire"));
        assert_eq!(err.kind(), "io");
    }

    #[test]
    fn stale_index_names_the_age_limit() {
        let err = Error::StaleIndex {
            corpus: "identity".into(),
            indexed_at: "2026-09-01T00:00:00Z".into(),
            max_age_days: 30,
        };
        assert_eq!(
            err.to_string(),
            "corpus `identity` was indexed at 2026-09-01T00:00:00Z, more than 30 days ago; re-run codecite index, or allow a stale index"
        );
    }

    #[test]
    fn read_errors_list_their_candidates() {
        let err = Error::AmbiguousDocument {
            reference: "§4.3".into(),
            candidates: vec![
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".into(),
                "rfc6749@sha256:abcdefabcdef#section-4.3".into(),
            ],
        };
        assert_eq!(
            err.to_string(),
            "§4.3 names no document and is not in exactly one current document; name one: rfc9449@sha256:1a2b3c4d5e6f#section-4.3, rfc6749@sha256:abcdefabcdef#section-4.3"
        );
        let err = Error::AmbiguousDocument {
            reference: "§99".into(),
            candidates: vec![],
        };
        assert_eq!(err.to_string(), "no current document has §99");
        let err = Error::NoSuchStep {
            section: "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".into(),
            step: 14,
            candidates: vec!["rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12".into()],
        };
        assert_eq!(
            err.to_string(),
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3 has no step 14; nearest: rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12"
        );
        let err = Error::NoSuchFile {
            repo: "app@0123abc".into(),
            path: "src/nope.rs".into(),
            candidates: vec![],
        };
        assert_eq!(
            err.to_string(),
            "app@0123abc has no indexed file \"src/nope.rs\"; no candidates"
        );
    }

    #[test]
    fn no_such_symbol_lists_the_names_it_could_mean() {
        let err = Error::NoSuchSymbol {
            symbol: "callBack".into(),
            candidates: vec!["callback".into()],
        };
        assert_eq!(err.kind(), "no_such_symbol");
        assert_eq!(err.candidates(), ["callback"]);
        assert_eq!(
            err.to_string(),
            "the corpus defines no symbol `callBack`; did you mean: callback"
        );
        let err = Error::NoSuchSymbol {
            symbol: "nope\u{1b}".into(),
            candidates: vec![],
        };
        assert_eq!(
            err.to_string(),
            "the corpus defines no symbol `nope\\u{1b}`; no candidates"
        );
    }

    #[test]
    fn candidates_have_their_control_characters_escaped() {
        let err = Error::NoSuchFile {
            repo: "app@0123abc".into(),
            path: "src/lib.rs".into(),
            candidates: vec!["src/\u{1b}]52;c;aGk=\u{7}/lib.rs".into()],
        };
        assert_eq!(
            err.to_string(),
            "app@0123abc has no indexed file \"src/lib.rs\"; candidates: src/\\u{1b}]52;c;aGk=\\u{7}/lib.rs"
        );
    }

    #[test]
    fn a_bad_citation_keeps_the_parse_error_as_its_source() {
        let err = Error::BadCitation {
            text: "§04.3".into(),
            source: CitationError::BadSection {
                found: "04.3".into(),
            },
        };
        assert_eq!(err.to_string(), "invalid citation \"§04.3\"");
        assert_eq!(
            err.source().unwrap().to_string(),
            "section \"04.3\" is not dot-separated numbers like 3.1.3.7"
        );
    }

    #[test]
    fn schema_too_new_display_names_path_and_versions() {
        let err = Error::SchemaTooNew {
            path: "/tmp/corpus.db".into(),
            found: 7,
            supported: 1,
        };
        assert_eq!(
            err.to_string(),
            "corpus at /tmp/corpus.db has schema version 7, but this codecite supports up to 1; upgrade codecite"
        );
        assert!(err.source().is_none());
    }
}
