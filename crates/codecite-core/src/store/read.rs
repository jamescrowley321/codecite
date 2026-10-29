//! Reading a citation: the exact text it points at, with the pinned
//! citation that names it.

use std::fmt;

use rusqlite::{OptionalExtension, params};
use schemars::JsonSchema;
use serde::Serialize;

use super::Store;
use super::search::code_citation;
use crate::{
    Citation, CitationError, CodeCitation, Error, Result, SectionNumber, SpecAliases, SpecCitation,
    SpecReference, SpecTarget,
};

/// How many lines `read` returns when the caller doesn't say.
pub const DEFAULT_MAX_LINES: u32 = 80;

/// The most lines a read may ask for: the CLI's `--max-lines` and the MCP
/// `read` tool's `max_lines` refuse more, which keeps one answer's size
/// bounded.
pub const MAX_READ_LINES: u32 = 500;

/// The most bytes of text a read returns, after the line limit: one
/// minified or generated line can hold a whole file. The text stops at the
/// last character that fits, and the result says it is truncated.
pub const MAX_READ_BYTES: usize = 64 * 1024;

/// The most candidates an error lists.
const MAX_CANDIDATES: usize = 5;

/// How to read a citation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOptions {
    /// The most lines of text to return; 0 reads as 1.
    pub max_lines: u32,
    /// Read an index older than its maximum age instead of refusing it.
    pub allow_stale: bool,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            max_lines: DEFAULT_MAX_LINES,
            allow_stale: false,
        }
    }
}

/// What a `quote` read tells the agent to do with its text.
pub const QUOTE_INSTRUCTION: &str =
    "Quote this text exactly, with its citation; never paraphrase it.";

/// What a `quote` read tells the agent when its text stops before the end
/// of what was cited, so a partial quote is never passed off as the whole.
pub const QUOTE_TRUNCATED_INSTRUCTION: &str = "Quote this text exactly, with its citation; never paraphrase it. It stops before the end of what the citation names, so say the quote is partial.";

/// The text a citation points at, in the shape the CLI prints as JSON and
/// the MCP server returns.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ReadResult {
    /// The canonical citation of the text, always with its pinned version:
    /// `repo@<full sha>:path#L10-L42`, or
    /// `doc@sha256:<12 hex>#section-4.3[/step-2]`.
    pub citation: String,
    /// `code` for lines of a file, `section` or `step` for a spec.
    pub kind: String,
    /// A spec section's heading; a step has its section's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A spec section's or step's requirement keywords, such as `MUST`.
    /// Code has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Vec<String>>,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    pub license_class: String,
    /// When the source was last indexed.
    pub indexed_at: String,
    /// For code, whether the file differed from the cited commit when it
    /// was indexed. Spec results have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    /// The exact text, at most the requested number of lines and at most
    /// `MAX_READ_BYTES` (64 KiB): a prefix of the cited lines joined with
    /// `\n`, cut at a character boundary, with no marker in it.
    pub text: String,
    /// Whether the text stops before the end of what was cited, at the
    /// line limit or the byte budget. The citation still names it all.
    pub truncated: bool,
    /// What the license asks of the agent, when it asks more than
    /// `license_class` says: a `quote` source's text is to be quoted
    /// exactly, with its citation, and never paraphrased, and said to be
    /// partial when `truncated` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

impl fmt::Display for ReadResult {
    /// The human form: the citation, a line saying what it is, a blank
    /// line, and the text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.citation)?;
        write!(f, "{}", self.kind)?;
        if let Some(title) = &self.title {
            write!(f, " · {title}")?;
        }
        if let Some(strength) = self.strength.as_ref().filter(|s| !s.is_empty()) {
            write!(f, " · {}", strength.join(", "))?;
        }
        write!(f, " · {} · indexed {}", self.license_class, self.indexed_at)?;
        if self.dirty == Some(true) {
            write!(f, " · dirty")?;
        }
        writeln!(f, "\n")?;
        writeln!(f, "{}", self.text)?;
        if self.truncated {
            writeln!(f, "… (truncated)")?;
        }
        Ok(())
    }
}

/// A source row, as `read` needs it.
pub(super) struct SourceRow {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) revision: String,
    pub(super) license_class: String,
    pub(super) indexed_at: String,
}

/// A spec section or step row.
struct SpecChunk {
    id: i64,
    number: String,
    title: Option<String>,
    strength: Option<String>,
    text: String,
}

impl Store {
    /// Reads the text `citation` points at.
    ///
    /// `citation` is a canonical code or spec citation, or a spec citation
    /// as people write it (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`, a
    /// spec URL), with the corpus's document aliases. A spec citation that
    /// names no version reads the current one, and one that names no
    /// document reads the one current document that has its section and
    /// step. The result's citation always names the pinned version.
    ///
    /// Anything that does not resolve to exactly one span is a typed error,
    /// most of them with candidates: never a guess, and never an empty
    /// success. An index older than its maximum age is
    /// [`Error::StaleIndex`] unless `options.allow_stale` is set.
    pub fn read(&self, citation: &str, options: &ReadOptions) -> Result<ReadResult> {
        self.check_fresh(options.allow_stale)?;
        let text = citation.trim();
        let bad = |source| Error::BadCitation {
            text: text.into(),
            source,
        };
        // A canonical citation is tried first: a code path may hold `§` or
        // the word "section", which the written forms would claim.
        let canonical = match text.parse() {
            Ok(Citation::Code(c)) => return self.read_code(&c, options),
            Ok(Citation::Spec(c)) => return self.read_spec(&c, options),
            Err(e) => e,
        };
        match SpecReference::parse(text, &self.aliases()?) {
            Ok(Some(reference)) => self.read_reference(text, &reference, options),
            Ok(None) => match canonical {
                CitationError::MissingAt => Err(Error::NotACitation { text: text.into() }),
                e => Err(bad(e)),
            },
            Err(CitationError::UnknownDocument { found }) => Err(Error::UnknownSource {
                name: found,
                known: self.source_names(true)?,
            }),
            Err(e) => Err(bad(e)),
        }
    }

    /// The document aliases the last index run stored.
    pub fn aliases(&self) -> Result<SpecAliases> {
        let mut stmt = self.conn.prepare_cached("SELECT alias, name FROM alias")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut aliases = SpecAliases::new();
        for row in rows {
            let (alias, name): (String, String) = row?;
            aliases.insert(&alias, &name);
        }
        Ok(aliases)
    }

    /// Reads lines of a code file.
    fn read_code(&self, c: &CodeCitation, options: &ReadOptions) -> Result<ReadResult> {
        let versions = self.sources(c.repo(), false)?;
        if versions.is_empty() {
            return Err(Error::UnknownSource {
                name: c.repo().into(),
                known: self.source_names(false)?,
            });
        }
        // A citation may abbreviate the commit, as git does.
        let Some(source) = versions
            .iter()
            .find(|s| s.revision.starts_with(c.commit().as_str()))
        else {
            return Err(Error::UnknownVersion {
                name: c.repo().into(),
                version: c.commit().to_string(),
                known: versions
                    .iter()
                    .map(|s| format!("{}@{}", s.name, s.revision))
                    .collect(),
            });
        };
        let row: Option<(Option<String>, bool)> = self
            .conn
            .query_row(
                "SELECT text, dirty FROM document WHERE source_id = ?1 AND path = ?2",
                params![source.id, c.path()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((text, dirty)) = row else {
            return Err(Error::NoSuchFile {
                repo: format!("{}@{}", source.name, c.commit()),
                path: c.path().into(),
                candidates: self.same_file_name(source.id, c.path())?,
            });
        };
        let text = text.ok_or_else(|| Error::InvalidCorpus {
            path: self.path.clone(),
            reason: format!("it holds no text for {}; re-run codecite index", c.path()),
        })?;
        let (start, end) = (c.lines().start(), c.lines().end());
        let citation = code_citation(&source.name, &source.revision, c.path(), start, end);
        let lines = file_lines(&text);
        let count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        if end > count {
            return Err(Error::LinesPastEnd {
                citation,
                lines: count,
            });
        }
        let cited = &lines[start as usize - 1..end as usize];
        let (text, truncated) = cap(cited, options.max_lines);
        Ok(ReadResult {
            citation,
            kind: "code".into(),
            title: None,
            strength: None,
            license_class: source.license_class.clone(),
            indexed_at: source.indexed_at.clone(),
            dirty: Some(dirty),
            text,
            truncated,
            instruction: instruction(&source.license_class, truncated),
        })
    }

    /// Reads a canonical spec citation, at the version its hash names.
    fn read_spec(&self, c: &SpecCitation, options: &ReadOptions) -> Result<ReadResult> {
        let versions = self.sources(c.doc(), true)?;
        if versions.is_empty() {
            return Err(Error::UnknownSource {
                name: c.doc().into(),
                known: self.source_names(true)?,
            });
        }
        let Some(source) = versions
            .iter()
            .find(|s| s.revision.starts_with(c.hash().as_str()))
        else {
            return Err(Error::UnknownVersion {
                name: c.doc().into(),
                version: format!("sha256:{}", c.hash()),
                known: versions.iter().map(pinned).collect(),
            });
        };
        let target = SpecTarget::Section(c.section().clone());
        self.read_target(source, &target, c.step(), options)
    }

    /// Reads a spec citation as people write it.
    pub(super) fn read_reference(
        &self,
        text: &str,
        r: &SpecReference,
        options: &ReadOptions,
    ) -> Result<ReadResult> {
        let Some(doc) = r.doc() else {
            // Only the written forms with `§` or `Section` omit the
            // document, and they always name a section.
            let SpecTarget::Section(section) = r.target() else {
                return Err(Error::NotACitation { text: text.into() });
            };
            let source = self.only_document_with(text, section, r.step())?;
            return self.read_target(&source, r.target(), r.step(), options);
        };
        let versions = self.sources(doc, true)?;
        if versions.is_empty() {
            return Err(Error::UnknownSource {
                name: doc.into(),
                known: self.source_names(true)?,
            });
        }
        let current: Option<SourceRow> = self.current_version(doc)?;
        let Some(source) = current else {
            return Err(Error::UnknownVersion {
                name: doc.into(),
                version: "the current version".into(),
                known: versions.iter().map(pinned).collect(),
            });
        };
        self.read_target(&source, r.target(), r.step(), options)
    }

    /// Reads a section, or a step in it, of one document version.
    fn read_target(
        &self,
        source: &SourceRow,
        target: &SpecTarget,
        step: Option<u32>,
        options: &ReadOptions,
    ) -> Result<ReadResult> {
        let document = pinned(source);
        let section = match target {
            SpecTarget::Section(n) => self.section(source.id, "number", &n.to_string())?,
            SpecTarget::Anchor(a) => self.section(source.id, "anchor", a)?,
        };
        let Some(section) = section else {
            let (asked, candidates) = match target {
                SpecTarget::Section(n) => (n.to_string(), self.nearest_sections(source, n)?),
                SpecTarget::Anchor(a) => (format!("#{a}"), Vec::new()),
            };
            return Err(Error::NoSuchSection {
                document,
                section: asked,
                candidates,
            });
        };
        // An anchor can find a section, like an appendix, whose number no
        // canonical citation can hold.
        if section.number.parse::<SectionNumber>().is_err() {
            return Err(Error::UncitableSection {
                document,
                section: section.number,
            });
        }
        let section_citation = format!("{document}#section-{}", section.number);
        let (chunk, citation) = match step {
            None => (section, section_citation),
            Some(k) => {
                let Some(found) = self.step(section.id, k)? else {
                    let mut candidates = self.nearest_steps(section.id, k, &section_citation)?;
                    if candidates.is_empty() {
                        candidates.push(section_citation.clone());
                    }
                    return Err(Error::NoSuchStep {
                        section: section_citation,
                        step: k,
                        candidates,
                    });
                };
                let citation = format!("{section_citation}/step-{k}");
                // A step has its section's title.
                let title = section.title;
                (SpecChunk { title, ..found }, citation)
            }
        };
        let lines: Vec<&str> = chunk.text.lines().collect();
        let (text, truncated) = cap(&lines, options.max_lines);
        Ok(ReadResult {
            citation,
            kind: if step.is_some() { "step" } else { "section" }.into(),
            title: chunk.title,
            strength: Some(
                chunk
                    .strength
                    .map(|s| s.split(',').map(str::to_owned).collect())
                    .unwrap_or_default(),
            ),
            license_class: source.license_class.clone(),
            indexed_at: source.indexed_at.clone(),
            dirty: None,
            text,
            truncated,
            instruction: instruction(&source.license_class, truncated),
        })
    }

    /// The one current document that has `section`, and `step` in it when
    /// one is given.
    ///
    /// Several such documents are [`Error::AmbiguousDocument`] with each
    /// one's citation, and so is none. When none has the step but exactly
    /// one has the section, the error is [`Error::NoSuchStep`] in it.
    fn only_document_with(
        &self,
        text: &str,
        section: &SectionNumber,
        step: Option<u32>,
    ) -> Result<SourceRow> {
        let number = section.to_string();
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.id, s.name, s.revision, s.license_class, s.indexed_at, c.id
             FROM source s
             JOIN document d ON d.source_id = s.id
             JOIN chunk c ON c.document_id = d.id
             WHERE s.kind = 'document' AND s.is_current
               AND c.kind = 'section' AND c.number = ?1
             GROUP BY s.id
             ORDER BY s.name",
        )?;
        let rows = stmt.query_map([&number], |row| {
            Ok((
                SourceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    revision: row.get(2)?,
                    license_class: row.get(3)?,
                    indexed_at: row.get(4)?,
                },
                row.get::<_, i64>(5)?,
            ))
        })?;
        let mut with_section: Vec<(SourceRow, i64)> = rows.collect::<rusqlite::Result<_>>()?;
        let cited = |s: &SourceRow| match step {
            Some(k) => format!("{}#section-{number}/step-{k}", pinned(s)),
            None => format!("{}#section-{number}", pinned(s)),
        };
        // The indexes, in `with_section`, of the documents that have the
        // step too.
        let mut with_step = Vec::new();
        for (i, (_, section_id)) in with_section.iter().enumerate() {
            if step.map_or(Ok(true), |k| self.step(*section_id, k).map(|s| s.is_some()))? {
                with_step.push(i);
            }
        }
        match (with_step.as_slice(), with_section.len()) {
            (&[i], _) => Ok(with_section.swap_remove(i).0),
            // `read_target` reports the missing step, with the nearest ones.
            ([], 1) => Ok(with_section.swap_remove(0).0),
            ([], _) => Err(Error::AmbiguousDocument {
                reference: text.into(),
                candidates: with_section
                    .iter()
                    .map(|(s, _)| format!("{}#section-{number}", pinned(s)))
                    .collect(),
            }),
            (several, _) => Err(Error::AmbiguousDocument {
                reference: text.into(),
                candidates: several.iter().map(|&i| cited(&with_section[i].0)).collect(),
            }),
        }
    }

    /// Every version of the source or document `name`, oldest row first.
    pub(super) fn sources(&self, name: &str, document: bool) -> Result<Vec<SourceRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, revision, license_class, indexed_at FROM source
             WHERE name = ?1 AND (kind = 'document') = ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![name, document], source_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The current version of the document `name`.
    pub(super) fn current_version(&self, name: &str) -> Result<Option<SourceRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, revision, license_class, indexed_at FROM source
                 WHERE name = ?1 AND kind = 'document' AND is_current",
                [name],
                source_row,
            )
            .optional()?)
    }

    /// The names of the corpus's documents, or of its other sources, sorted.
    pub(super) fn source_names(&self, document: bool) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT name FROM source WHERE (kind = 'document') = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([document], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The section of a document version whose `column` (`number` or
    /// `anchor`) is `value`.
    fn section(&self, source_id: i64, column: &str, value: &str) -> Result<Option<SpecChunk>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT c.id, c.number, c.title, c.strength, c.text
                     FROM chunk c JOIN document d ON d.id = c.document_id
                     WHERE d.source_id = ?1 AND c.kind = 'section' AND c.{column} = ?2
                     ORDER BY c.id LIMIT 1"
                ),
                params![source_id, value],
                spec_chunk,
            )
            .optional()?)
    }

    /// Step `k` of the section chunk `section_id`.
    fn step(&self, section_id: i64, k: u32) -> Result<Option<SpecChunk>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, number, title, strength, text FROM chunk
                 WHERE parent_id = ?1 AND kind = 'step' AND number = ?2
                 ORDER BY id LIMIT 1",
                params![section_id, k.to_string()],
                spec_chunk,
            )
            .optional()?)
    }

    /// The sections of a document version nearest to `wanted`: those that
    /// share the most leading numbers with it, then the closest in the
    /// first number that differs.
    fn nearest_sections(&self, source: &SourceRow, wanted: &SectionNumber) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.number FROM chunk c JOIN document d ON d.id = c.document_id
             WHERE d.source_id = ?1 AND c.kind = 'section' ORDER BY c.id",
        )?;
        let numbers = stmt.query_map([source.id], |row| row.get::<_, String>(0))?;
        let want = wanted.parts();
        let mut scored = Vec::new();
        for number in numbers {
            let number = number?;
            let Ok(parsed) = number.parse::<SectionNumber>() else {
                continue;
            };
            let have = parsed.parts();
            let shared = want.iter().zip(have).take_while(|(a, b)| a == b).count();
            let distance = match (want.get(shared), have.get(shared)) {
                (Some(a), Some(b)) => a.abs_diff(*b),
                _ => 0,
            };
            scored.push((std::cmp::Reverse(shared), distance, have.len(), number));
        }
        // A stable sort keeps document order among equals.
        scored.sort_by_key(|s| (s.0, s.1, s.2));
        let document = pinned(source);
        Ok(scored
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|(.., number)| format!("{document}#section-{number}"))
            .collect())
    }

    /// The steps of a section nearest to step `k`.
    fn nearest_steps(&self, section_id: i64, k: u32, section: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT number FROM chunk WHERE parent_id = ?1 AND kind = 'step' ORDER BY id",
        )?;
        let numbers = stmt.query_map([section_id], |row| row.get::<_, String>(0))?;
        let mut steps = Vec::new();
        for number in numbers {
            if let Ok(n) = number?.parse::<u32>() {
                steps.push(n);
            }
        }
        steps.sort_by_key(|&n| (n.abs_diff(k), n));
        Ok(steps
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|n| format!("{section}/step-{n}"))
            .collect())
    }

    /// Indexed paths in the source with the same file name as `path`.
    fn same_file_name(&self, source_id: i64, path: &str) -> Result<Vec<String>> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut stmt = self.conn.prepare_cached(
            "SELECT path FROM document WHERE source_id = ?1
               AND (path = ?2 OR substr(path, -length(?2) - 1) = '/' || ?2)
             ORDER BY path",
        )?;
        let rows = stmt.query_map(params![source_id, name], |row| row.get(0))?;
        Ok(rows.take(MAX_CANDIDATES).collect::<rusqlite::Result<_>>()?)
    }
}

fn source_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceRow> {
    Ok(SourceRow {
        id: row.get(0)?,
        name: row.get(1)?,
        revision: row.get(2)?,
        license_class: row.get(3)?,
        indexed_at: row.get(4)?,
    })
}

fn spec_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<SpecChunk> {
    Ok(SpecChunk {
        id: row.get(0)?,
        number: row.get(1)?,
        title: row.get(2)?,
        strength: row.get(3)?,
        text: row.get(4)?,
    })
}

/// A document version as citations name it: `rfc9449@sha256:1a2b3c4d5e6f`.
pub(super) fn pinned(source: &SourceRow) -> String {
    let hash = source.revision.get(..12).unwrap_or(&source.revision);
    format!("{}@sha256:{hash}", source.name)
}

/// A file's lines as the chunker counts them: split at `\n`, a final
/// newline ending the last line rather than starting an empty one, and a
/// `\r` before a `\n` dropped.
pub(super) fn file_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// The instruction a result from a source of `license_class` carries, when
/// its text is `truncated` or whole.
pub(super) fn instruction(license_class: &str, truncated: bool) -> Option<String> {
    let text = if truncated {
        QUOTE_TRUNCATED_INSTRUCTION
    } else {
        QUOTE_INSTRUCTION
    };
    (license_class == "quote").then(|| text.into())
}

/// The first `max_lines` of `lines` joined, cut to [`MAX_READ_BYTES`], and
/// whether anything was left out.
fn cap(lines: &[&str], max_lines: u32) -> (String, bool) {
    let max = usize::try_from(max_lines.max(1)).unwrap_or(usize::MAX);
    let kept = &lines[..lines.len().min(max)];
    let text = kept.join("\n");
    let (cut, over) = byte_prefix(&text, MAX_READ_BYTES);
    (cut.to_owned(), over || kept.len() < lines.len())
}

/// The longest prefix of `text` that fits in `max` bytes and ends on a
/// character boundary, and whether it is shorter than `text`.
pub(super) fn byte_prefix(text: &str, max: usize) -> (&str, bool) {
    let end = text.floor_char_boundary(max);
    (&text[..end], end < text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, DocumentRecord, Keyword, SourceRecord};

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    const RFC9449: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";
    const CORE: &str = "c0de00000000000000000000000000000000000000000000000000000000c0de";
    /// Five lines; the chunker gives the lone `}` no chunk.
    const LIB: &str = "pub fn validate() {\n    check();\n}\n\npub fn check() {}\n";

    fn source<'a>(
        name: &'a str,
        kind: &'a str,
        revision: &'a str,
        current: bool,
    ) -> SourceRecord<'a> {
        SourceRecord {
            name,
            kind,
            url: "https://example.com",
            revision,
            license_spdx: None,
            license_class: if kind == "document" { "quote" } else { "adapt" },
            is_current: current,
            obsoleted_by: None,
            updated_by: None,
        }
    }

    fn section<'a>(
        number: &'a str,
        anchor: &'a str,
        title: &'a str,
        text: &'a str,
    ) -> ChunkRecord<'a> {
        ChunkRecord {
            kind: "section",
            start_line: 1,
            end_line: 1,
            text,
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
            number: Some(number),
            anchor: None,
            title: None,
            parent: Some(parent),
            strength: &[Keyword::Must],
            ..section(number, "", "", text)
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

    fn code<'a>(path: &'a str, text: &'a str) -> DocumentRecord<'a> {
        DocumentRecord {
            path,
            language: Some("rust"),
            content_hash: path.as_bytes(),
            size_bytes: 1,
            dirty: true,
            text: Some(text),
            chunks: Vec::new(),
        }
    }

    /// A corpus with a code source, RFC 9449 at two versions and OIDC Core.
    /// Both specs have a §4.3; only Core has §3.1.3.7.
    struct Fixture {
        store: Store,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn read(&self, citation: &str) -> Result<ReadResult> {
            self.store.read(citation, &ReadOptions::default())
        }

        fn ok(&self, citation: &str) -> ReadResult {
            self.read(citation)
                .unwrap_or_else(|e| panic!("{citation}: {e}"))
        }
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let app = store
            .upsert_source(&source("app", "local", HEAD, true))
            .unwrap();
        store.write_document(app, &code("src/lib.rs", LIB)).unwrap();
        store
            .write_document(app, &code("tests/lib.rs", "fn t() {}\n"))
            .unwrap();
        store
            .write_document(app, &code("docs/§4 section.md", "# Four\n"))
            .unwrap();

        let rfc = store
            .upsert_source(&source("rfc9449", "document", RFC9449, true))
            .unwrap();
        let checking = "4.3.  Checking DPoP Proofs\nTo validate a DPoP proof, the receiving server MUST ensure:";
        store
            .write_document(
                rfc,
                &spec(
                    "rfc9449",
                    vec![
                        section("4.3", "section-4.3", "Checking DPoP Proofs", checking),
                        step("1", 0, "1. There is not more than one DPoP HTTP request header field."),
                        step("2", 0, "2. The DPoP HTTP request header field value is a single well-formed JWT."),
                        step("12", 0, "12. The jti is unique."),
                        section("4.4", "section-4.4", "Signing", "4.4.  Signing"),
                        section("5", "section-5", "Protected Resources", "5.  Protected Resources"),
                        section("A", "appendix-A", "Acknowledgements", "Appendix A.  Acknowledgements"),
                    ],
                ),
            )
            .unwrap();
        let old = "a".repeat(64);
        let older = store
            .upsert_source(&source("rfc9449", "document", &old, false))
            .unwrap();
        store
            .write_document(
                older,
                &spec(
                    "rfc9449",
                    vec![section(
                        "4.3",
                        "section-4.3",
                        "Checking DPoP Proofs",
                        "the old text",
                    )],
                ),
            )
            .unwrap();

        let core = store
            .upsert_source(&source("openid-connect-core-1_0", "document", CORE, true))
            .unwrap();
        store
            .write_document(
                core,
                &spec(
                    "openid-connect-core-1_0",
                    vec![
                        section(
                            "3.1.3.7",
                            "IDTokenValidation",
                            "ID Token Validation",
                            "3.1.3.7.  ID Token Validation",
                        ),
                        step("12", 0, "12. If the acr Claim was requested, ..."),
                        step("13", 0, "13. If the auth_time Claim was requested, ..."),
                        section("4.3", "section-4.3", "Unrelated", "4.3.  Unrelated"),
                    ],
                ),
            )
            .unwrap();
        let aliases: SpecAliases = [
            ("OIDC Core", "openid-connect-core-1_0"),
            ("Core", "openid-connect-core-1_0"),
        ]
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
    fn a_code_citation_reads_the_exact_lines() {
        let f = fixture();
        // An abbreviated commit, and a `}` line that no chunk holds.
        let r = f.ok("app@0123456:src/lib.rs#L2-L3");
        assert_eq!(r.citation, format!("app@{HEAD}:src/lib.rs#L2-L3"));
        assert_eq!(r.kind, "code");
        assert_eq!(r.text, "    check();\n}");
        assert_eq!((r.dirty, r.truncated), (Some(true), false));
        assert_eq!(r.license_class, "adapt");
        assert_eq!(r.instruction, None);
        assert_eq!((r.title, r.strength), (None, None));
        let r = f.ok(&format!("app@{HEAD}:src/lib.rs#L5"));
        assert_eq!(r.text, "pub fn check() {}");
    }

    #[test]
    fn a_code_citation_past_the_end_of_the_file_is_an_error() {
        let f = fixture();
        for lines in ["L5-L6", "L9"] {
            let err = f
                .read(&format!("app@0123456:src/lib.rs#{lines}"))
                .unwrap_err();
            assert!(
                matches!(err, Error::LinesPastEnd { lines: 5, .. }),
                "{lines}: {err:?}"
            );
        }
    }

    #[test]
    fn max_lines_caps_the_text() {
        let f = fixture();
        let options = ReadOptions {
            max_lines: 2,
            ..ReadOptions::default()
        };
        let r = f
            .store
            .read("app@0123456:src/lib.rs#L1-L5", &options)
            .unwrap();
        assert_eq!(r.text, "pub fn validate() {\n    check();");
        assert!(r.truncated);
        assert_eq!(r.citation, format!("app@{HEAD}:src/lib.rs#L1-L5"));
        let r = f
            .store
            .read(
                "RFC 9449 §4.3",
                &ReadOptions {
                    max_lines: 1,
                    ..options
                },
            )
            .unwrap();
        assert_eq!(r.text, "4.3.  Checking DPoP Proofs");
        assert!(r.truncated);
        // A cut `quote` text says it is partial.
        assert_eq!(r.instruction.as_deref(), Some(QUOTE_TRUNCATED_INSTRUCTION));
    }

    #[test]
    fn the_byte_budget_caps_a_long_line_on_a_character_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let app = store
            .upsert_source(&source("app", "local", HEAD, true))
            .unwrap();
        // One line of a two-byte `é` past the budget, its first one across it.
        let line = format!("a{}", "é".repeat(MAX_READ_BYTES));
        let text = format!("{line}\nnext\n");
        store
            .write_document(app, &code("dist/min.js", &text))
            .unwrap();
        store.set_indexed_at(30).unwrap();
        let options = ReadOptions {
            max_lines: 1,
            ..ReadOptions::default()
        };
        let r = store.read("app@0123456:dist/min.js#L1", &options).unwrap();
        assert_eq!(r.text.len(), MAX_READ_BYTES - 1);
        assert!(line.starts_with(&r.text));
        assert!(r.truncated);
        // The citation still names the whole line, and reads the same back.
        assert_eq!(r.citation, format!("app@{HEAD}:dist/min.js#L1"));
        assert_eq!(store.read(&r.citation, &options).unwrap(), r);
        // A line within the budget is whole.
        let r = store.read("app@0123456:dist/min.js#L2", &options).unwrap();
        assert_eq!((r.text.as_str(), r.truncated), ("next", false));
    }

    #[test]
    fn crlf_lines_read_with_newline_endings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let app = store
            .upsert_source(&source("app", "local", HEAD, true))
            .unwrap();
        // The budget falls inside the second line.
        let second = "b".repeat(MAX_READ_BYTES);
        let text = format!("a\r\n{second}\r\n");
        store.write_document(app, &code("crlf.txt", &text)).unwrap();
        store.set_indexed_at(30).unwrap();
        let r = store
            .read("app@0123456:crlf.txt#L1-L2", &ReadOptions::default())
            .unwrap();
        assert!(r.truncated);
        assert_eq!(r.text.len(), MAX_READ_BYTES);
        assert!(format!("a\n{second}").starts_with(&r.text));
    }

    #[test]
    fn a_missing_file_commit_or_source_is_an_error_with_candidates() {
        let f = fixture();
        let err = f.read("app@0123456:src/nested/lib.rs#L1").unwrap_err();
        assert!(
            matches!(&err, Error::NoSuchFile { candidates, .. }
                if candidates == &["src/lib.rs", "tests/lib.rs"]),
            "{err:?}"
        );
        let err = f.read("app@fedcba9:src/lib.rs#L1").unwrap_err();
        assert!(
            matches!(&err, Error::UnknownVersion { known, .. } if known == &[format!("app@{HEAD}")]),
            "{err:?}"
        );
        let err = f.read("other@0123456:src/lib.rs#L1").unwrap_err();
        assert!(
            matches!(&err, Error::UnknownSource { known, .. } if known == &["app"]),
            "{err:?}"
        );
    }

    #[test]
    fn a_canonical_spec_citation_reads_its_version() {
        let f = fixture();
        let r = f.ok("rfc9449@sha256:1a2b3c4d5e6f#section-4.3");
        assert_eq!(r.kind, "section");
        assert_eq!(r.title.as_deref(), Some("Checking DPoP Proofs"));
        assert!(r.text.starts_with("4.3.  Checking DPoP Proofs\n"));
        assert_eq!(r.strength, Some(vec![]));
        assert_eq!((r.license_class.as_str(), r.dirty), ("quote", None));
        // A quote source's text comes with the instruction to quote it.
        assert_eq!(r.instruction.as_deref(), Some(QUOTE_INSTRUCTION));
        let r = f.ok("rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12");
        assert_eq!(
            r.citation,
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12"
        );
        assert_eq!(r.kind, "step");
        assert_eq!(r.text, "12. The jti is unique.");
        assert_eq!(r.title.as_deref(), Some("Checking DPoP Proofs"));
        assert_eq!(r.strength, Some(vec!["MUST".to_owned()]));
        // An older pinned version is read by its hash.
        let r = f.ok("rfc9449@sha256:aaaaaaaaaaaa#section-4.3");
        assert_eq!(r.text, "the old text");
        let err = f
            .read("rfc9449@sha256:bbbbbbbbbbbb#section-4.3")
            .unwrap_err();
        assert!(
            matches!(&err, Error::UnknownVersion { known, .. }
                if known == &["rfc9449@sha256:1a2b3c4d5e6f", "rfc9449@sha256:aaaaaaaaaaaa"]),
            "{err:?}"
        );
    }

    #[test]
    fn written_forms_read_the_current_version_and_name_it() {
        let f = fixture();
        let core = "openid-connect-core-1_0@sha256:c0de00000000#section-3.1.3.7";
        for (written, citation) in [
            (
                "RFC 9449 §4.3",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".to_owned(),
            ),
            (
                "RFC9449 Section 4.3",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".to_owned(),
            ),
            (
                "RFC 9449 §4.3 step 2",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2".to_owned(),
            ),
            ("OIDC Core §3.1.3.7 step 13", format!("{core}/step-13")),
            ("Core 3.1.3.7(13)", format!("{core}/step-13")),
            (
                "https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".to_owned(),
            ),
            (
                "https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation",
                core.to_owned(),
            ),
            ("§3.1.3.7 step 13", format!("{core}/step-13")),
        ] {
            assert_eq!(f.ok(written).citation, citation, "{written}");
        }
        assert_eq!(
            f.ok("Core 3.1.3.7(13)").text,
            "13. If the auth_time Claim was requested, ..."
        );
    }

    #[test]
    fn a_bare_section_in_two_documents_is_ambiguous() {
        let f = fixture();
        let err = f.read("§4.3").unwrap_err();
        assert!(
            matches!(&err, Error::AmbiguousDocument { reference, candidates }
            if reference == "§4.3" && candidates == &[
                "openid-connect-core-1_0@sha256:c0de00000000#section-4.3",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
            ]),
            "{err:?}"
        );
        // Only RFC 9449's §4.3 has a step 2, so it is the one document.
        assert_eq!(
            f.ok("§4.3 step 2").citation,
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2"
        );
        let err = f.read("§4.3 step 7").unwrap_err();
        assert!(
            matches!(&err, Error::AmbiguousDocument { candidates, .. } if candidates.len() == 2),
            "{err:?}"
        );
        let err = f.read("§9.9").unwrap_err();
        assert!(
            matches!(&err, Error::AmbiguousDocument { candidates, .. } if candidates.is_empty()),
            "{err:?}"
        );
    }

    #[test]
    fn a_missing_step_or_section_names_the_nearest() {
        let f = fixture();
        let err = f.read("RFC 9449 §4.3 step 14").unwrap_err();
        let pinned = "rfc9449@sha256:1a2b3c4d5e6f#section-4.3";
        assert!(
            matches!(&err, Error::NoSuchStep { section, step: 14, candidates }
                if section == pinned
                && candidates == &[format!("{pinned}/step-12"), format!("{pinned}/step-2"), format!("{pinned}/step-1")]),
            "{err:?}"
        );
        // The only document with §3.1.3.7 reports its missing step.
        let err = f.read("§3.1.3.7 step 14").unwrap_err();
        assert!(
            matches!(&err, Error::NoSuchStep { candidates, .. } if candidates[0].ends_with("/step-13")),
            "{err:?}"
        );
        let err = f.read("RFC 9449 §4.9").unwrap_err();
        assert!(
            matches!(&err, Error::NoSuchSection { document, section, candidates }
            if document == "rfc9449@sha256:1a2b3c4d5e6f" && section == "4.9"
            && candidates == &[
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.4",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
                "rfc9449@sha256:1a2b3c4d5e6f#section-5",
            ]),
            "{err:?}"
        );
        let err = f
            .read("https://openid.net/specs/openid-connect-core-1_0.html#NoSuchAnchor")
            .unwrap_err();
        assert!(
            matches!(&err, Error::NoSuchSection { section, .. } if section == "#NoSuchAnchor"),
            "{err:?}"
        );
    }

    #[test]
    fn text_that_is_no_citation_or_a_bad_one_is_an_error() {
        let f = fixture();
        assert!(matches!(
            f.read("hello world"),
            Err(Error::NotACitation { .. })
        ));
        assert!(matches!(
            f.read("RFC 9449 §04.3"),
            Err(Error::BadCitation {
                source: CitationError::BadSection { .. },
                ..
            })
        ));
        assert!(matches!(
            f.read("rfc9449@sha256:1a2b#section-4.3"),
            Err(Error::BadCitation {
                source: CitationError::BadHash { .. },
                ..
            })
        ));
        let err = f.read("Foo §3.1").unwrap_err();
        assert!(
            matches!(&err, Error::UnknownSource { name, known }
                if name == "Foo" && known == &["openid-connect-core-1_0", "rfc9449"]),
            "{err:?}"
        );
        assert!(matches!(
            f.read("RFC 9999 §1"),
            Err(Error::UnknownSource { .. })
        ));
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
        assert!(matches!(
            f.read("RFC 9449 §4.3"),
            Err(Error::StaleIndex { .. })
        ));
        let options = ReadOptions {
            allow_stale: true,
            ..ReadOptions::default()
        };
        assert!(f.store.read("RFC 9449 §4.3", &options).is_ok());
    }

    #[test]
    fn a_code_file_without_text_asks_for_a_reindex() {
        let f = fixture();
        rusqlite::Connection::open(f.store.path())
            .unwrap()
            .execute("UPDATE document SET text = NULL", [])
            .unwrap();
        let err = f.read("app@0123456:src/lib.rs#L1").unwrap_err();
        assert!(matches!(err, Error::InvalidCorpus { .. }), "{err:?}");
    }

    #[test]
    fn json_and_human_forms() {
        let f = fixture();
        let spec = serde_json::to_value(f.ok("Core 3.1.3.7(13)")).unwrap();
        let keys: Vec<&String> = spec.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "citation",
                "indexed_at",
                "instruction",
                "kind",
                "license_class",
                "strength",
                "text",
                "title",
                "truncated"
            ]
        );
        let code = serde_json::to_value(f.ok("app@0123456:src/lib.rs#L1")).unwrap();
        assert_eq!(code["dirty"], true);
        assert!(code.get("strength").is_none());
        assert!(code.get("instruction").is_none());

        let mut r = f.ok("Core 3.1.3.7(13)");
        r.indexed_at = "2026-10-20T06:00:00Z".into();
        assert_eq!(
            r.to_string(),
            "openid-connect-core-1_0@sha256:c0de00000000#section-3.1.3.7/step-13
step · ID Token Validation · MUST · quote · indexed 2026-10-20T06:00:00Z

13. If the auth_time Claim was requested, ...
"
        );
        let mut r = f
            .store
            .read(
                "app@0123456:src/lib.rs#L1-L2",
                &ReadOptions {
                    max_lines: 1,
                    allow_stale: false,
                },
            )
            .unwrap();
        r.indexed_at = "2026-10-20T06:00:00Z".into();
        assert_eq!(
            r.to_string(),
            format!(
                "app@{HEAD}:src/lib.rs#L1-L2
code · adapt · indexed 2026-10-20T06:00:00Z · dirty

pub fn validate() {{
… (truncated)
"
            )
        );
    }

    #[test]
    fn a_canonical_code_citation_wins_over_written_forms() {
        // `§` and the word "section" in a path are not a written spec form.
        let r = fixture().ok("app@0123456:docs/§4 section.md#L1");
        assert_eq!(r.citation, format!("app@{HEAD}:docs/§4 section.md#L1"));
        assert_eq!(r.text, "# Four");
    }

    #[test]
    fn a_section_without_a_canonical_citation_is_an_error() {
        // Appendix numbers like `A` have no canonical citation form, so
        // read never returns one it could not read back.
        let err = fixture()
            .read("https://www.rfc-editor.org/rfc/rfc9449#appendix-A")
            .unwrap_err();
        assert!(
            matches!(&err, Error::UncitableSection { document, section }
                if document == "rfc9449@sha256:1a2b3c4d5e6f" && section == "A"),
            "{err:?}"
        );
    }

    #[test]
    fn file_lines_count_as_the_chunker_does() {
        assert_eq!(file_lines("\n"), [""]);
        assert_eq!(file_lines("a\r\nb\n"), ["a", "b"]);
        assert_eq!(file_lines("a\n\n"), ["a", ""]);
        assert_eq!(file_lines("a"), ["a"]);
        assert!(file_lines("").is_empty());
    }
}
