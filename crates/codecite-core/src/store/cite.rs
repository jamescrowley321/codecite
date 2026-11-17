//! Checking a citation: whether it resolves, what it points at, and, for
//! a spec citation, the findings that say why not. `cite` resolves through `read` and never
//! forks it. [`CitationChecker`] checks every citation written in a text,
//! each through `cite`.

use std::collections::HashMap;
use std::fmt;

use rusqlite::{OptionalExtension, params};
use schemars::JsonSchema;
use serde::Serialize;

use super::Store;
use super::read::pinned;
use super::search::MAX_SNIPPET_LINES;
use crate::{
    CitationError, Error, ReadOptions, ReadResult, Result, SpecCitation, SpecReference,
    find_citations,
};

/// How to check a citation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CiteOptions {
    /// Check against an index older than its maximum age instead of
    /// refusing it.
    pub allow_stale: bool,
}

/// What `cite` found, in the shape the CLI prints as JSON. Fields with no
/// value are left out; `findings` is always there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
pub struct CiteResult {
    /// Whether the citation names exactly one span of the corpus.
    pub resolved: bool,
    /// When resolved, the canonical citation, with its pinned version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    /// When resolved, what `read` says it is: `code`, `document`,
    /// `section` or `step`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The section's heading; a step has its section's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The heading's anchor, without `#`, when the citation is one
    /// heading's whole span in a file or fetched document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_class: Option<String>,
    /// When the source was last indexed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<String>,
    /// The cited text, at most 40 lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Whether the text stops before the end of what was cited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// The section's or step's requirement keywords, such as `MUST`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Vec<String>>,
    /// For a `quote` source: quote the text exactly, with its citation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    /// When not resolved, the pinned citations it could have meant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
    /// What replaces the cited version: the document that obsoletes it, or
    /// the current version's pinned citation when it is an older one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Why the citation does not resolve. Empty for a clean citation.
    pub findings: Vec<Finding>,
}

/// One thing wrong with a citation that names a document in the corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Finding {
    /// The document version has no such section.
    NoSuchSection {
        /// The pinned document: `rfc9449@sha256:1a2b3c4d5e6f`.
        document: String,
        /// The section as cited: `4.9`, or `#anchor` for a URL's anchor.
        section: String,
        /// The nearest sections that do exist, as pinned citations.
        candidates: Vec<String>,
    },
    /// The section has no such step.
    NoSuchStep {
        /// The section, as a pinned citation.
        section: String,
        /// The step asked for.
        step: u32,
        /// The nearest steps, or the section when it has none.
        candidates: Vec<String>,
    },
    /// A Markdown link whose text cites one section and whose URL's anchor
    /// points at another.
    AnchorLabelMismatch {
        /// The link's text: `OIDC Core §3.1.3.6`.
        label: String,
        /// What the text resolves to, as a pinned citation.
        label_citation: String,
        /// The link's URL, with its anchor.
        url: String,
        /// What the anchor resolves to, as a pinned citation.
        url_citation: String,
    },
    /// A bare `§` that is not in exactly one current document.
    AmbiguousDocument {
        /// The citation as given.
        reference: String,
        /// Each current document's pinned citation of it; empty when none
        /// has it.
        candidates: Vec<String>,
    },
}

impl Finding {
    /// The pinned citations the finding offers instead.
    pub fn candidates(&self) -> Vec<String> {
        match self {
            Finding::NoSuchSection { candidates, .. }
            | Finding::NoSuchStep { candidates, .. }
            | Finding::AmbiguousDocument { candidates, .. } => candidates.clone(),
            Finding::AnchorLabelMismatch {
                label_citation,
                url_citation,
                ..
            } => vec![label_citation.clone(), url_citation.clone()],
        }
    }

    /// The finding `read`'s error stands for, or the error back when it
    /// stays an error.
    fn from_error(err: Error) -> Result<Finding> {
        Ok(match err {
            Error::NoSuchSection {
                document,
                section,
                candidates,
            } => Finding::NoSuchSection {
                document,
                section,
                candidates,
            },
            Error::NoSuchStep {
                section,
                step,
                candidates,
            } => Finding::NoSuchStep {
                section,
                step,
                candidates,
            },
            Error::AmbiguousDocument {
                reference,
                candidates,
            } => Finding::AmbiguousDocument {
                reference,
                candidates,
            },
            err => return Err(err),
        })
    }
}

impl fmt::Display for Finding {
    /// One line: the finding's type and what it found.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Finding::NoSuchSection {
                document, section, ..
            } => write!(f, "no_such_section: {document} has no section {section}"),
            Finding::NoSuchStep { section, step, .. } => {
                write!(f, "no_such_step: {section} has no step {step}")
            }
            Finding::AnchorLabelMismatch {
                label,
                label_citation,
                url,
                url_citation,
            } => write!(
                f,
                "anchor_label_mismatch: {label} is {label_citation}, but {url} is {url_citation}"
            ),
            Finding::AmbiguousDocument {
                reference,
                candidates,
            } if candidates.is_empty() => {
                write!(f, "ambiguous_document: no current document has {reference}")
            }
            Finding::AmbiguousDocument {
                reference,
                candidates,
            } => write!(
                f,
                "ambiguous_document: {reference} names no document and is in {} current documents",
                candidates.len()
            ),
        }
    }
}

impl fmt::Display for CiteResult {
    /// The human form. Resolved: the citation, a line saying what it is,
    /// what supersedes it, a blank line and the text. Not resolved:
    /// `unresolved`, the findings and the candidates.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(citation) = &self.citation else {
            writeln!(f, "unresolved")?;
            for finding in &self.findings {
                writeln!(f, "{finding}")?;
            }
            if !self.candidates.is_empty() {
                writeln!(f, "candidates:")?;
                for candidate in &self.candidates {
                    writeln!(f, "  {candidate}")?;
                }
            }
            return Ok(());
        };
        writeln!(f, "{citation}")?;
        write!(f, "{}", self.kind.as_deref().unwrap_or_default())?;
        if let Some(title) = &self.title {
            write!(f, " · {title}")?;
        }
        if let Some(anchor) = &self.anchor {
            write!(f, " · #{anchor}")?;
        }
        if let Some(strength) = self.strength.as_ref().filter(|s| !s.is_empty()) {
            write!(f, " · {}", strength.join(", "))?;
        }
        if let (Some(class), Some(at)) = (&self.license_class, &self.indexed_at) {
            write!(f, " · {class} · indexed {at}")?;
        }
        writeln!(f)?;
        if let Some(by) = &self.superseded_by {
            writeln!(f, "superseded by {by}")?;
        }
        writeln!(f)?;
        writeln!(f, "{}", self.text.as_deref().unwrap_or_default())?;
        if self.truncated == Some(true) {
            writeln!(f, "… (truncated)")?;
        }
        Ok(())
    }
}

impl Store {
    /// Checks the citation `citation`.
    ///
    /// It takes what `read` takes, and a Markdown link whose text is a
    /// written spec citation and whose target is a spec URL with an
    /// anchor: `[OIDC Core §3.1.3.6](https://openid.net/specs/…#IDTokenValidation)`.
    /// For a spec, a missing section or step, a bare `§` that is not in
    /// exactly one current document, and a link whose text and anchor
    /// disagree are findings, with `resolved: false`. Anything else `read`
    /// refuses stays an error, a missing anchor in a file or fetched
    /// document among them.
    pub fn cite(&self, citation: &str, options: &CiteOptions) -> Result<CiteResult> {
        let text = citation.trim();
        if let Some((label, url)) = markdown_link(text) {
            return self.cite_link(label, url, options);
        }
        let read_options = ReadOptions {
            max_lines: snippet_lines(),
            allow_stale: options.allow_stale,
        };
        match self.read(text, &read_options) {
            Ok(read) => self.resolved(read),
            Err(err) => unresolved(Finding::from_error(err)?),
        }
    }

    /// Checks a Markdown link: its URL's anchor, then its text, which
    /// takes the URL's document when it names none.
    fn cite_link(&self, label: &str, url: &str, options: &CiteOptions) -> Result<CiteResult> {
        self.check_fresh(options.allow_stale)?;
        let aliases = self.aliases()?;
        let parse = |text: &str| match SpecReference::parse(text, &aliases) {
            Ok(Some(reference)) => Ok(reference),
            Ok(None) => Err(Error::NotACitation { text: text.into() }),
            Err(CitationError::UnknownDocument { found }) => Err(Error::UnknownSource {
                name: found,
                known: self.source_names(true)?,
            }),
            Err(source) => Err(Error::BadCitation {
                text: text.into(),
                source,
            }),
        };
        // A link's target must be a spec URL, which always names its
        // document.
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(Error::NotACitation { text: url.into() });
        }
        let at_url = parse(url)?;
        let Some(doc) = at_url.doc() else {
            return Err(Error::NotACitation { text: url.into() });
        };
        let mut written = parse(label)?;
        if written.doc().is_none() {
            written = written.in_document(doc);
        }
        // The freshness check is done, once, above.
        let read_options = ReadOptions {
            max_lines: snippet_lines(),
            allow_stale: true,
        };
        let anchor = match self.read_reference(url, &at_url, &read_options) {
            Ok(read) => read,
            Err(err) => return unresolved(Finding::from_error(err)?),
        };
        let read = match self.read_reference(label, &written, &read_options) {
            Ok(read) => read,
            Err(err) => return unresolved(Finding::from_error(err)?),
        };
        // The anchor names a section; the text may name a step in it.
        if section_of(&read.citation) == section_of(&anchor.citation) {
            return self.resolved(read);
        }
        unresolved(Finding::AnchorLabelMismatch {
            label: label.into(),
            label_citation: read.citation,
            url: url.into(),
            url_citation: anchor.citation,
        })
    }

    /// A resolved result from what `read` returned, with what supersedes
    /// the version it names.
    fn resolved(&self, read: ReadResult) -> Result<CiteResult> {
        let superseded_by = self.superseded_by(&read.citation)?;
        Ok(CiteResult {
            resolved: true,
            citation: Some(read.citation),
            kind: Some(read.kind),
            title: read.title,
            anchor: read.anchor,
            license_class: Some(read.license_class),
            indexed_at: Some(read.indexed_at),
            text: Some(read.text),
            truncated: Some(read.truncated),
            strength: read.strength,
            instruction: read.instruction,
            superseded_by,
            ..CiteResult::default()
        })
    }

    /// What supersedes the document version a pinned spec citation names:
    /// the document its `obsoleted_by` names, pinned when the corpus has
    /// it, or else the current version when this one is older. A document
    /// that is only updated is not superseded.
    fn superseded_by(&self, citation: &str) -> Result<Option<String>> {
        // Only a spec version is superseded: code and generic documents
        // are not tracked across versions.
        let Ok(c) = citation.parse::<SpecCitation>() else {
            return Ok(None);
        };
        let row: Option<(bool, Option<String>)> = self
            .conn
            .query_row(
                "SELECT is_current, obsoleted_by FROM source
                 WHERE name = ?1 AND kind = 'document' AND substr(revision, 1, 12) = ?2",
                params![c.doc(), c.hash().as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((is_current, obsoleted_by)) = row else {
            return Ok(None);
        };
        if let Some(by) = obsoleted_by {
            return Ok(Some(match self.current_version(&by)? {
                Some(source) => pinned(&source),
                None => by,
            }));
        }
        if is_current {
            return Ok(None);
        }
        Ok(self.current_version(c.doc())?.map(|s| pinned(&s)))
    }
}

/// How to check the citations written in a text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckOptions {
    /// Check against an index older than its maximum age instead of
    /// refusing it.
    pub allow_stale: bool,
    /// Count a citation of a superseded version as holding up.
    pub allow_superseded: bool,
}

/// Why a written citation does not hold up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum Problem {
    /// `cite` found it names nothing in the corpus: a missing section or
    /// step, a link whose text and anchor disagree, or a bare `§` in no
    /// single document.
    Unresolved {
        /// What `cite` found.
        findings: Vec<Finding>,
        /// The pinned citations it could have meant.
        candidates: Vec<String>,
    },
    /// `read` refused it: an unknown source, version, file or anchor,
    /// lines past the end, or a malformed citation.
    Refused {
        /// The error's kind, as the MCP tools name it: `no_such_file`.
        kind: String,
        /// The error's message.
        message: String,
        /// What the error offers instead.
        candidates: Vec<String>,
    },
    /// It resolves, to a version that something newer supersedes.
    Superseded {
        /// What it resolves to, pinned.
        resolved: String,
        /// What supersedes it.
        superseded_by: String,
    },
}

impl fmt::Display for Problem {
    /// One line: why the citation does not hold up.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Unresolved { findings, .. } => {
                let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
                write!(f, "{}", lines.join("; "))
            }
            Problem::Refused { kind, message, .. } => write!(f, "{kind}: {message}"),
            Problem::Superseded { superseded_by, .. } => {
                write!(f, "superseded by {superseded_by}")
            }
        }
    }
}

impl Problem {
    /// The pinned citations or names it offers instead: a superseded
    /// citation's successor.
    pub fn candidates(&self) -> &[String] {
        match self {
            Problem::Unresolved { candidates, .. } | Problem::Refused { candidates, .. } => {
                candidates
            }
            Problem::Superseded { superseded_by, .. } => std::slice::from_ref(superseded_by),
        }
    }
}

/// A citation written in a text that does not hold up, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CitationProblem {
    /// The line it is written on, from 1.
    pub line: usize,
    /// The character it starts at on that line, from 1.
    pub column: usize,
    /// The citation as written.
    pub citation: String,
    /// Why it does not hold up.
    #[serde(flatten)]
    pub problem: Problem,
}

/// What checking one text found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextCheck {
    /// How many citations it holds; text that only looked like one is not
    /// counted.
    pub citations: usize,
    /// The ones that do not hold up, in order.
    pub problems: Vec<CitationProblem>,
}

/// Checks the citations written in texts against one corpus, each through
/// [`Store::cite`]. A citation written more than once is checked once.
pub struct CitationChecker<'s> {
    store: &'s Store,
    allow_superseded: bool,
    /// Each citation checked so far: `None` when it is no citation, else
    /// its problem, if any.
    seen: HashMap<String, Option<Option<Problem>>>,
}

impl Store {
    /// A checker for the citations written in texts. A stale index is
    /// refused here, once, unless `options.allow_stale` is set.
    pub fn citation_checker(&self, options: &CheckOptions) -> Result<CitationChecker<'_>> {
        self.check_fresh(options.allow_stale)?;
        Ok(CitationChecker {
            store: self,
            allow_superseded: options.allow_superseded,
            seen: HashMap::new(),
        })
    }
}

impl CitationChecker<'_> {
    /// Checks every citation [`find_citations`] finds in `text`. A
    /// citation that does not resolve, that `read` refuses, or that names a
    /// superseded version (unless allowed) is a problem; text that `cite`
    /// says is no citation at all is skipped. An error that is not about
    /// the citation, such as a database error, stops the check.
    pub fn check(&mut self, text: &str) -> Result<TextCheck> {
        let mut checked = TextCheck::default();
        for written in find_citations(text) {
            let problem = match self.seen.get(written.text) {
                Some(known) => known.clone(),
                None => {
                    let found = self.problem(written.text)?;
                    self.seen.insert(written.text.to_owned(), found.clone());
                    found
                }
            };
            let Some(problem) = problem else {
                continue;
            };
            checked.citations += 1;
            if let Some(problem) = problem {
                checked.problems.push(CitationProblem {
                    line: written.line,
                    column: written.column,
                    citation: written.text.to_owned(),
                    problem,
                });
            }
        }
        Ok(checked)
    }

    /// `None` when `citation` is no citation; else its problem, if any.
    fn problem(&self, citation: &str) -> Result<Option<Option<Problem>>> {
        // The checker refused a stale index when it was made.
        let options = CiteOptions { allow_stale: true };
        let result = match self.store.cite(citation, &options) {
            Ok(result) => result,
            Err(Error::NotACitation { .. }) => return Ok(None),
            Err(
                err @ (Error::NotASpecCitation { .. }
                | Error::BadCitation { .. }
                | Error::UnknownSource { .. }
                | Error::UnknownVersion { .. }
                | Error::AmbiguousDocument { .. }
                | Error::NoSuchSection { .. }
                | Error::NoSuchAnchor { .. }
                | Error::UncitableSection { .. }
                | Error::NoSuchStep { .. }
                | Error::NoSuchFile { .. }
                | Error::LinesPastEnd { .. }),
            ) => {
                return Ok(Some(Some(Problem::Refused {
                    kind: err.kind().to_owned(),
                    message: err.to_string(),
                    candidates: err.candidates().to_vec(),
                })));
            }
            Err(err) => return Err(err),
        };
        if !result.resolved {
            return Ok(Some(Some(Problem::Unresolved {
                findings: result.findings,
                candidates: result.candidates,
            })));
        }
        Ok(Some(match (result.citation, result.superseded_by) {
            (Some(resolved), Some(superseded_by)) if !self.allow_superseded => {
                Some(Problem::Superseded {
                    resolved,
                    superseded_by,
                })
            }
            _ => None,
        }))
    }
}

/// A result with one finding, not resolved.
fn unresolved(finding: Finding) -> Result<CiteResult> {
    Ok(CiteResult {
        candidates: finding.candidates(),
        findings: vec![finding],
        ..CiteResult::default()
    })
}

/// The text and target of a Markdown link, `[text](target)`.
fn markdown_link(text: &str) -> Option<(&str, &str)> {
    let inner = text.strip_prefix('[')?.strip_suffix(')')?;
    let (label, url) = inner.rsplit_once("](")?;
    Some((label.trim(), url.trim()))
}

/// A pinned spec citation without its `/step-<k>`.
fn section_of(citation: &str) -> &str {
    citation
        .split_once("/step-")
        .map_or(citation, |(section, _)| section)
}

/// The most lines of text a result holds: a search snippet's.
fn snippet_lines() -> u32 {
    u32::try_from(MAX_SNIPPET_LINES).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ChunkRecord, DocumentRecord, Keyword, QUOTE_INSTRUCTION, SourceRecord, SpecAliases,
    };

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    const RFC9449: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";
    const CORE: &str = "c0de00000000000000000000000000000000000000000000000000000000c0de";
    const RFC2616: &str = "2616000000000000000000000000000000000000000000000000000000002616";
    const RFC9110: &str = "9110000000000000000000000000000000000000000000000000000000009110";
    const RFC6749: &str = "6749000000000000000000000000000000000000000000000000000000006749";
    const PINNED: &str = "rfc9449@sha256:1a2b3c4d5e6f";
    const CORE_PINNED: &str = "openid-connect-core-1_0@sha256:c0de00000000";
    const CORE_URL: &str = "https://openid.net/specs/openid-connect-core-1_0.html";

    fn document<'a>(name: &'a str, revision: &'a str) -> SourceRecord<'a> {
        SourceRecord {
            name,
            kind: "document",
            url: "https://example.com",
            revision,
            license_spdx: None,
            license_class: "quote",
            is_current: true,
            obsoleted_by: None,
            updated_by: None,
        }
    }

    fn section<'a>(number: &'a str, anchor: &'a str, title: &'a str) -> ChunkRecord<'a> {
        ChunkRecord {
            kind: "section",
            start_line: 1,
            end_line: 1,
            text: title,
            number: Some(number),
            anchor: Some(anchor),
            title: Some(title),
            parent: None,
            strength: &[],
            symbol: None,
        }
    }

    fn step<'a>(number: &'a str, parent: usize, text: &'a str) -> ChunkRecord<'a> {
        ChunkRecord {
            kind: "step",
            anchor: None,
            title: None,
            parent: Some(parent),
            strength: &[Keyword::Must],
            ..section(number, "", text)
        }
    }

    fn spec<'a>(path: &'a str, chunks: Vec<ChunkRecord<'a>>) -> DocumentRecord<'a> {
        DocumentRecord {
            path,
            language: None,
            content_hash: path.as_bytes(),
            size_bytes: 1,
            dirty: false,
            text: None,
            chunks,
        }
    }

    /// Six lines of Markdown: `# Notes` over lines 1-2, `## Install`
    /// over 3-6.
    const NOTES: &str = "# Notes\nIntro.\n## Install\n\nRun it.\nThen check.\n";
    const NOTES_HASH: &str = "9a9b00000000000000000000000000000000000000000000000000000000beef";

    fn heading<'a>(anchor: &'a str, title: &'a str, lines: (u32, u32)) -> ChunkRecord<'a> {
        ChunkRecord {
            start_line: lines.0,
            end_line: lines.1,
            text: "",
            number: None,
            ..section("", anchor, title)
        }
    }

    fn markdown(path: &str) -> DocumentRecord<'_> {
        DocumentRecord {
            language: Some("markdown"),
            text: Some(NOTES),
            ..spec(
                path,
                vec![
                    heading("notes", "Notes", (1, 2)),
                    heading("install", "Install", (3, 6)),
                ],
            )
        }
    }

    struct Fixture {
        store: Store,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn cite(&self, citation: &str) -> Result<CiteResult> {
            self.store.cite(citation, &CiteOptions::default())
        }

        fn ok(&self, citation: &str) -> CiteResult {
            self.cite(citation)
                .unwrap_or_else(|e| panic!("{citation}: {e}"))
        }

        /// The one finding of a citation that does not resolve.
        fn finding(&self, citation: &str) -> (Finding, Vec<String>) {
            let r = self.ok(citation);
            assert!(!r.resolved, "{citation}: {r:?}");
            assert_eq!(r.citation, None, "{citation}");
            assert_eq!(r.findings.len(), 1, "{citation}: {r:?}");
            (r.findings[0].clone(), r.candidates)
        }
    }

    /// A code source; RFC 9449 at two versions; OIDC Core; RFC 2616,
    /// obsoleted by RFC 9110; and RFC 6749, only updated. RFC 9449 and
    /// Core both have a §4.3.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let app = store
            .upsert_source(&SourceRecord {
                kind: "local",
                license_class: "adapt",
                ..document("app", HEAD)
            })
            .unwrap();
        let code = DocumentRecord {
            language: Some("rust"),
            text: Some("fn a() {}\n"),
            ..spec("src/lib.rs", Vec::new())
        };
        store.write_document(app, &code).unwrap();
        store.write_document(app, &markdown("README.md")).unwrap();
        let notes = store.upsert_source(&document("notes", NOTES_HASH)).unwrap();
        store.write_document(notes, &markdown("notes")).unwrap();

        let rfc = store.upsert_source(&document("rfc9449", RFC9449)).unwrap();
        let chunks = vec![
            section("4.3", "section-4.3", "Checking DPoP Proofs"),
            step("1", 0, "1. There is not more than one DPoP header."),
            step("2", 0, "2. The DPoP header is a single well-formed JWT."),
            section("4.4", "section-4.4", "Signing"),
        ];
        store.write_document(rfc, &spec("rfc9449", chunks)).unwrap();
        let old = "a".repeat(64);
        let older = store
            .upsert_source(&SourceRecord {
                is_current: false,
                ..document("rfc9449", &old)
            })
            .unwrap();
        let chunks = vec![section("4.3", "section-4.3", "Checking DPoP Proofs")];
        store
            .write_document(older, &spec("rfc9449", chunks))
            .unwrap();

        let core = store
            .upsert_source(&document("openid-connect-core-1_0", CORE))
            .unwrap();
        let chunks = vec![
            section("3.1.3.6", "CodeIDToken", "ID Token"),
            section("3.1.3.7", "IDTokenValidation", "ID Token Validation"),
            step("12", 1, "12. If the acr Claim was requested, ..."),
            step("13", 1, "13. If the auth_time Claim was requested, ..."),
            section("4.3", "section-4.3", "Unrelated"),
        ];
        store
            .write_document(core, &spec("openid-connect-core-1_0", chunks))
            .unwrap();

        let obsolete = store
            .upsert_source(&SourceRecord {
                obsoleted_by: Some("rfc9110"),
                ..document("rfc2616", RFC2616)
            })
            .unwrap();
        let chunks = vec![section("14.9", "section-14.9", "Cache-Control")];
        store
            .write_document(obsolete, &spec("rfc2616", chunks))
            .unwrap();
        let http = store.upsert_source(&document("rfc9110", RFC9110)).unwrap();
        let chunks = vec![section("1", "section-1", "Introduction")];
        store
            .write_document(http, &spec("rfc9110", chunks))
            .unwrap();
        let updated = store
            .upsert_source(&SourceRecord {
                updated_by: Some("rfc9700"),
                ..document("rfc6749", RFC6749)
            })
            .unwrap();
        let chunks = vec![section(
            "10.12",
            "section-10.12",
            "Cross-Site Request Forgery",
        )];
        store
            .write_document(updated, &spec("rfc6749", chunks))
            .unwrap();

        let aliases: SpecAliases = [("OIDC Core", "openid-connect-core-1_0")]
            .into_iter()
            .collect();
        store.set_aliases(&aliases).unwrap();
        store.set_indexed_at(30).unwrap();
        drop(store);
        Fixture {
            store: Store::open_read_only(&path).unwrap(),
            _dir: dir,
        }
    }

    #[test]
    fn a_clean_citation_resolves_with_no_findings() {
        let f = fixture();
        let r = f.ok("OIDC Core §3.1.3.7 step 13");
        assert!(r.resolved);
        assert_eq!(
            r.citation.as_deref(),
            Some(format!("{CORE_PINNED}#section-3.1.3.7/step-13").as_str())
        );
        assert_eq!(r.kind.as_deref(), Some("step"));
        assert_eq!(r.title.as_deref(), Some("ID Token Validation"));
        assert_eq!(r.strength, Some(vec!["MUST".to_owned()]));
        assert_eq!(
            r.text.as_deref(),
            Some("13. If the auth_time Claim was requested, ...")
        );
        assert_eq!(r.truncated, Some(false));
        assert_eq!(r.license_class.as_deref(), Some("quote"));
        assert_eq!(r.instruction.as_deref(), Some(QUOTE_INSTRUCTION));
        assert!(r.findings.is_empty() && r.candidates.is_empty());
        assert_eq!(r.superseded_by, None);
        // `cite` gives what `read` gives.
        let read = f
            .store
            .read("OIDC Core §3.1.3.7 step 13", &ReadOptions::default())
            .unwrap();
        assert_eq!(r.citation, Some(read.citation));
        assert_eq!(r.text, Some(read.text));
    }

    #[test]
    fn a_code_citation_resolves_to_its_full_commit() {
        let f = fixture();
        let r = f.ok("app@0123456:src/lib.rs#L1");
        assert!(r.resolved && r.findings.is_empty());
        assert_eq!(
            r.citation.as_deref(),
            Some(format!("app@{HEAD}:src/lib.rs#L1").as_str())
        );
        assert_eq!(r.kind.as_deref(), Some("code"));
        assert_eq!(r.text.as_deref(), Some("fn a() {}"));
        assert_eq!(r.license_class.as_deref(), Some("adapt"));
        assert_eq!((r.title, r.anchor, r.superseded_by), (None, None, None));
    }

    #[test]
    fn an_anchor_resolves_to_its_section_lines() {
        let f = fixture();
        let r = f.ok("app@0123456:README.md#install");
        assert!(r.resolved && r.findings.is_empty());
        assert_eq!(
            r.citation.as_deref(),
            Some(format!("app@{HEAD}:README.md#L3-L6").as_str())
        );
        assert_eq!(r.kind.as_deref(), Some("section"));
        assert_eq!(r.title.as_deref(), Some("Install"));
        assert_eq!(r.anchor.as_deref(), Some("install"));
        assert_eq!(
            r.text.as_deref(),
            Some("## Install\n\nRun it.\nThen check.")
        );
    }

    #[test]
    fn a_fetched_document_resolves_by_lines_or_anchor() {
        let f = fixture();
        let r = f.ok("notes@sha256:9a9b00000000#L2-L4");
        assert!(r.resolved && r.findings.is_empty());
        assert_eq!(
            r.citation.as_deref(),
            Some("notes@sha256:9a9b00000000#L2-L4")
        );
        assert_eq!(r.kind.as_deref(), Some("document"));
        assert_eq!(r.text.as_deref(), Some("Intro.\n## Install\n"));
        assert_eq!((r.title, r.anchor), (None, None));
        let r = f.ok("notes@sha256:9a9b00000000#notes");
        assert_eq!(
            r.citation.as_deref(),
            Some("notes@sha256:9a9b00000000#L1-L2")
        );
        assert_eq!(r.kind.as_deref(), Some("section"));
        assert_eq!(r.anchor.as_deref(), Some("notes"));
        assert_eq!(r.superseded_by, None);
        // The human form names the section by its title.
        let human = r.to_string();
        assert!(
            human
                .lines()
                .nth(1)
                .is_some_and(|l| l.starts_with("section · Notes · #notes · quote · indexed ")),
            "{human}"
        );
    }

    #[test]
    fn a_missing_section_is_a_finding() {
        let (finding, candidates) = fixture().finding("RFC 9449 §4.9");
        assert_eq!(
            finding,
            Finding::NoSuchSection {
                document: PINNED.into(),
                section: "4.9".into(),
                candidates: vec![
                    format!("{PINNED}#section-4.4"),
                    format!("{PINNED}#section-4.3"),
                ],
            }
        );
        assert_eq!(candidates, finding.candidates());
    }

    #[test]
    fn a_missing_step_is_a_finding() {
        let (finding, candidates) = fixture().finding("RFC 9449 §4.3 step 7");
        let section = format!("{PINNED}#section-4.3");
        assert_eq!(
            finding,
            Finding::NoSuchStep {
                section: section.clone(),
                step: 7,
                candidates: vec![format!("{section}/step-2"), format!("{section}/step-1")],
            }
        );
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn a_bare_section_resolves_in_one_document_and_is_ambiguous_in_two() {
        let f = fixture();
        let r = f.ok("§3.1.3.7 step 12");
        assert!(r.resolved && r.findings.is_empty());
        assert_eq!(
            r.citation.as_deref(),
            Some(format!("{CORE_PINNED}#section-3.1.3.7/step-12").as_str())
        );
        let (finding, candidates) = f.finding("§4.3");
        let both = vec![
            format!("{CORE_PINNED}#section-4.3"),
            format!("{PINNED}#section-4.3"),
        ];
        assert_eq!(
            finding,
            Finding::AmbiguousDocument {
                reference: "§4.3".into(),
                candidates: both.clone(),
            }
        );
        assert_eq!(candidates, both);
    }

    #[test]
    fn a_link_whose_anchor_is_another_section_is_a_mismatch() {
        let f = fixture();
        let url = format!("{CORE_URL}#IDTokenValidation");
        for label in ["OIDC Core §3.1.3.6", "§3.1.3.6"] {
            let (finding, candidates) = f.finding(&format!("[{label}]({url})"));
            let (written, anchored) = (
                format!("{CORE_PINNED}#section-3.1.3.6"),
                format!("{CORE_PINNED}#section-3.1.3.7"),
            );
            assert_eq!(
                finding,
                Finding::AnchorLabelMismatch {
                    label: label.into(),
                    label_citation: written.clone(),
                    url: url.clone(),
                    url_citation: anchored.clone(),
                },
                "{label}"
            );
            assert_eq!(candidates, [written, anchored]);
        }
    }

    #[test]
    fn a_link_whose_text_and_anchor_agree_resolves() {
        let f = fixture();
        let url = format!("{CORE_URL}#IDTokenValidation");
        for (label, citation) in [
            (
                "OIDC Core §3.1.3.7",
                format!("{CORE_PINNED}#section-3.1.3.7"),
            ),
            // A bare § takes the URL's document, and a step in the
            // anchor's section agrees with it.
            (
                "§3.1.3.7 step 13",
                format!("{CORE_PINNED}#section-3.1.3.7/step-13"),
            ),
        ] {
            let r = f.ok(&format!("[{label}]({url})"));
            assert!(r.resolved && r.findings.is_empty(), "{label}: {r:?}");
            assert_eq!(r.citation, Some(citation), "{label}");
        }
        let r = f.ok("[RFC 9449 §4.3](https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3)");
        assert_eq!(r.citation, Some(format!("{PINNED}#section-4.3")));
    }

    #[test]
    fn a_link_that_misses_reports_the_finding() {
        let f = fixture();
        let (finding, _) = f.finding(&format!("[§3.1.3.9]({CORE_URL}#IDTokenValidation)"));
        assert!(
            matches!(&finding, Finding::NoSuchSection { section, .. } if section == "3.1.3.9"),
            "{finding:?}"
        );
        let (finding, _) = f.finding(&format!("[§3.1.3.7]({CORE_URL}#Nowhere)"));
        assert!(
            matches!(&finding, Finding::NoSuchSection { section, .. } if section == "#Nowhere"),
            "{finding:?}"
        );
    }

    #[test]
    fn a_link_that_is_no_spec_link_is_an_error() {
        let f = fixture();
        for (link, kind) in [
            (
                "[see here](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)",
                "not_a_citation",
            ),
            ("[§3.1](https://example.com/spec#x)", "not_a_citation"),
            ("[§3.1](§3.1)", "not_a_citation"),
            (
                "[Foo §3.1](https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3)",
                "unknown_source",
            ),
            (
                "[§04.3](https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3)",
                "bad_citation",
            ),
        ] {
            let err = f.cite(link).unwrap_err();
            assert_eq!(err.kind(), kind, "{link}: {err}");
        }
    }

    #[test]
    fn an_older_version_is_superseded_by_the_current_one() {
        let f = fixture();
        let r = f.ok("rfc9449@sha256:aaaaaaaaaaaa#section-4.3");
        assert!(r.resolved);
        assert_eq!(r.superseded_by.as_deref(), Some(PINNED));
        assert_eq!(f.ok("RFC 9449 §4.3").superseded_by, None);
    }

    #[test]
    fn an_obsoleted_rfc_is_superseded_but_an_updated_one_is_not() {
        let f = fixture();
        let r = f.ok("RFC 2616 §14.9");
        assert!(r.resolved && r.findings.is_empty());
        assert_eq!(
            r.superseded_by.as_deref(),
            Some("rfc9110@sha256:911000000000")
        );
        assert_eq!(f.ok("RFC 6749 §10.12").superseded_by, None);
    }

    #[test]
    fn an_obsoleting_document_outside_the_corpus_is_named() {
        let f = fixture();
        rusqlite::Connection::open(f.store.path())
            .unwrap()
            .execute(
                "UPDATE source SET obsoleted_by = 'rfc9999' WHERE name = 'rfc2616'",
                [],
            )
            .unwrap();
        assert_eq!(
            f.ok("RFC 2616 §14.9").superseded_by.as_deref(),
            Some("rfc9999")
        );
    }

    #[test]
    fn what_read_refuses_otherwise_stays_an_error() {
        let f = fixture();
        for (citation, kind) in [
            ("hello world", "not_a_citation"),
            ("RFC 9449 §04.3", "bad_citation"),
            ("RFC 9999 §1", "unknown_source"),
            ("rfc9449@sha256:bbbbbbbbbbbb#section-4.3", "unknown_version"),
            ("app@0123456:README.md#instal", "no_such_anchor"),
        ] {
            let err = f.cite(citation).unwrap_err();
            assert_eq!(err.kind(), kind, "{citation}: {err}");
        }
    }

    #[test]
    fn a_stale_index_is_refused_unless_allowed() {
        let f = fixture();
        rusqlite::Connection::open(f.store.path())
            .unwrap()
            .execute(
                "UPDATE meta SET value = '2000-01-01T00:00:00Z' WHERE key = 'indexed_at'",
                [],
            )
            .unwrap();
        let link = format!("[§3.1.3.7]({CORE_URL}#IDTokenValidation)");
        for citation in ["RFC 9449 §4.3", link.as_str()] {
            let err = f.cite(citation).unwrap_err();
            assert_eq!(err.kind(), "stale_index", "{citation}");
            let allowed = CiteOptions { allow_stale: true };
            assert!(f.store.cite(citation, &allowed).unwrap().resolved);
        }
    }

    #[test]
    fn json_and_human_forms() {
        let f = fixture();
        let json = serde_json::to_value(f.ok("§4.3")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "resolved": false,
                "candidates": [
                    format!("{CORE_PINNED}#section-4.3"),
                    format!("{PINNED}#section-4.3"),
                ],
                "findings": [{
                    "type": "ambiguous_document",
                    "reference": "§4.3",
                    "candidates": [
                        format!("{CORE_PINNED}#section-4.3"),
                        format!("{PINNED}#section-4.3"),
                    ],
                }],
            })
        );
        let json = serde_json::to_value(f.ok("RFC 9449 §4.3")).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "citation",
                "findings",
                "indexed_at",
                "instruction",
                "kind",
                "license_class",
                "resolved",
                "strength",
                "text",
                "title",
                "truncated"
            ]
        );
        assert_eq!(json["findings"], serde_json::json!([]));

        let mut r = f.ok("rfc9449@sha256:aaaaaaaaaaaa#section-4.3");
        r.indexed_at = Some("2026-10-20T06:00:00Z".into());
        assert_eq!(
            r.to_string(),
            format!(
                "rfc9449@sha256:aaaaaaaaaaaa#section-4.3
section · Checking DPoP Proofs · quote · indexed 2026-10-20T06:00:00Z
superseded by {PINNED}

Checking DPoP Proofs
"
            )
        );
        assert_eq!(
            f.ok("RFC 9449 §4.3 step 7").to_string(),
            format!(
                "unresolved
no_such_step: {PINNED}#section-4.3 has no step 7
candidates:
  {PINNED}#section-4.3/step-2
  {PINNED}#section-4.3/step-1
"
            )
        );
        assert_eq!(
            f.ok("§9.9").to_string(),
            "unresolved\nambiguous_document: no current document has §9.9\n"
        );
    }

    #[test]
    fn text_is_capped_at_the_snippet_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let long: String = (1..=50).map(|n| format!("line {n}\n")).collect();
        let rfc = store.upsert_source(&document("rfc9449", RFC9449)).unwrap();
        let chunks = vec![ChunkRecord {
            text: &long,
            ..section("4.3", "section-4.3", "Checking DPoP Proofs")
        }];
        store.write_document(rfc, &spec("rfc9449", chunks)).unwrap();
        store.set_indexed_at(30).unwrap();
        let r = store
            .cite("RFC 9449 §4.3", &CiteOptions::default())
            .unwrap();
        assert_eq!(r.text.unwrap().lines().count(), MAX_SNIPPET_LINES);
        assert_eq!(r.truncated, Some(true));
    }

    /// The problems `check` finds in `text`, as (line, citation, problem
    /// kind) triples, and how many citations it counted.
    fn checked(f: &Fixture, options: &CheckOptions, text: &str) -> (usize, Vec<CitationProblem>) {
        let mut checker = f.store.citation_checker(options).unwrap();
        let found = checker.check(text).unwrap();
        (found.citations, found.problems)
    }

    fn problem_kind(p: &Problem) -> String {
        match p {
            Problem::Unresolved { findings, .. } => {
                serde_json::to_value(&findings[0]).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }
            Problem::Refused { kind, .. } => kind.clone(),
            Problem::Superseded { .. } => "superseded".into(),
        }
    }

    #[test]
    fn check_passes_citations_that_resolve() {
        let f = fixture();
        let text = format!(
            "// app@{HEAD}:src/lib.rs#L1 and `app@0123456:src/lib.rs#L1`\n\
             See app@0123456:README.md#install, notes@sha256:9a9b00000000#L3-L6\n\
             and {PINNED}#section-4.3/step-2. [OIDC Core §3.1.3.7]({CORE_URL}#IDTokenValidation)\n\
             rfc6749@sha256:674900000000#section-10.12 is only updated.\n"
        );
        let (citations, problems) = checked(&f, &CheckOptions::default(), &text);
        assert_eq!(problems, []);
        assert_eq!(citations, 7);
    }

    #[test]
    fn check_reports_each_citation_that_does_not_hold_up() {
        let f = fixture();
        let text = format!(
            "app@0123456:src/gone.rs#L1\n\
             app@0123456:src/lib.rs#L1-L9\n\
             {PINNED}#section-9.9\n\
             rfc2616@sha256:261600000000#section-14.9\n\
             nobody@0123456:a.rs#L1\n\
             [OIDC Core §3.1.3.6]({CORE_URL}#IDTokenValidation)\n\
             app@0123456:README.md#nowhere\n"
        );
        let (citations, problems) = checked(&f, &CheckOptions::default(), &text);
        assert_eq!(citations, 7);
        let found: Vec<(usize, String)> = problems
            .iter()
            .map(|p| (p.line, problem_kind(&p.problem)))
            .collect();
        assert_eq!(
            found,
            [
                (1, "no_such_file".to_owned()),
                (2, "lines_past_end".to_owned()),
                (3, "no_such_section".to_owned()),
                (4, "superseded".to_owned()),
                (5, "unknown_source".to_owned()),
                (6, "anchor_label_mismatch".to_owned()),
                (7, "no_such_anchor".to_owned()),
            ]
        );
        let superseded = &problems[3];
        assert_eq!(
            superseded.citation,
            "rfc2616@sha256:261600000000#section-14.9"
        );
        assert_eq!(superseded.column, 1);
        assert_eq!(
            superseded.problem.to_string(),
            "superseded by rfc9110@sha256:911000000000"
        );
        assert_eq!(
            superseded.problem.candidates(),
            ["rfc9110@sha256:911000000000"]
        );
        // A missing section offers the nearest ones.
        assert!(!problems[2].problem.candidates().is_empty());
        // The JSON names the problem beside where it is.
        let json = serde_json::to_value(superseded).unwrap();
        assert_eq!(json["problem"], "superseded");
        assert_eq!(json["line"], 4);
        assert_eq!(json["superseded_by"], "rfc9110@sha256:911000000000");
    }

    #[test]
    fn check_allows_superseded_versions_when_asked() {
        let f = fixture();
        let options = CheckOptions {
            allow_superseded: true,
            ..CheckOptions::default()
        };
        let text = format!("rfc2616@sha256:261600000000#section-14.9 and {PINNED}#section-9.9\n");
        let (citations, problems) = checked(&f, &options, &text);
        assert_eq!(citations, 2);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].citation, format!("{PINNED}#section-9.9"));
        // The older pinned version of RFC 9449 is superseded by the current one.
        let older = format!("rfc9449@sha256:{}#section-4.3", "a".repeat(12));
        let (_, problems) = checked(&f, &CheckOptions::default(), &older);
        assert_eq!(problem_kind(&problems[0].problem), "superseded");
        let (_, problems) = checked(&f, &options, &older);
        assert_eq!(problems, []);
    }

    #[test]
    fn check_skips_links_that_are_no_citation_and_counts_repeats() {
        let f = fixture();
        let text = "[see §2](https://example.com/page#top)\n\
                    app@0123456:src/gone.rs#L1 app@0123456:src/gone.rs#L1\n";
        let (citations, problems) = checked(&f, &CheckOptions::default(), text);
        assert_eq!(citations, 2);
        let at: Vec<(usize, usize)> = problems.iter().map(|p| (p.line, p.column)).collect();
        assert_eq!(at, [(2, 1), (2, 28)]);
    }

    #[test]
    fn check_refuses_a_stale_index_once_unless_allowed() {
        let f = fixture();
        rusqlite::Connection::open(f.store.path())
            .unwrap()
            .execute(
                "UPDATE meta SET value = '2000-01-01T00:00:00Z' WHERE key = 'indexed_at'",
                [],
            )
            .unwrap();
        assert!(matches!(
            f.store.citation_checker(&CheckOptions::default()),
            Err(Error::StaleIndex { .. })
        ));
        let options = CheckOptions {
            allow_stale: true,
            ..CheckOptions::default()
        };
        let (citations, problems) = checked(&f, &options, "app@0123456:src/lib.rs#L1");
        assert_eq!((citations, problems.len()), (1, 0));
    }
}
