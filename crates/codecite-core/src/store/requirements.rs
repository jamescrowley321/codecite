//! The requirements governing a spec section: every normative statement of
//! the section, its subsections and the sections linked to it by
//! reference, each with its pinned citation.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::Serialize;

use super::Store;
use super::read::{MAX_READ_BYTES, ReadOptions, SourceRow, instruction, pinned};
use super::search::{Scope, SearchQuery};
use crate::{
    Citation, Error, Level, Result, SectionNumber, StatementKind, normative_level,
    normative_statements, requirement_keywords,
};

/// How many search hits a topic looks through for one with a citation
/// form; appendices have none.
const TOPIC_HITS: u32 = 10;

/// How to gather requirements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequirementsOptions {
    /// Read an index older than its maximum age instead of refusing it.
    pub allow_stale: bool,
}

/// Every rule governing a spec section, grouped by the section that states
/// it: what `codecite requirements --json` prints and the MCP
/// `requirements` tool returns.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Requirements {
    /// The section asked about, pinned: `doc@sha256:<12 hex>#section-2.6`.
    /// A step citation asks about its section.
    pub citation: String,
    /// The section's heading.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How agents may use the document: `quote` for a spec.
    pub license_class: String,
    /// When the document was last indexed.
    pub indexed_at: String,
    /// The governing sections: the one asked about, its subsections, then
    /// the sections linked to it by reference, in document order.
    pub sections: Vec<RequirementSection>,
    /// Whether statements were left out to keep the statements' text
    /// within 64 KiB (`MAX_READ_BYTES`).
    pub truncated: bool,
    /// What the license asks: a `quote` source's statements are quoted
    /// exactly, with their citations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

/// One governing section and its normative statements.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct RequirementSection {
    /// The section, pinned.
    pub citation: String,
    /// Its heading.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Why it governs: `asked`; `subsection` of it; `referenced` by it or
    /// a subsection (`Section 2.4`, or a section whose title its title
    /// holds); `referring` to it or a subsection; or `context`, the
    /// enclosing section of a referring subsection.
    pub relation: String,
    /// Its statements: `MUST`-level first, then its steps in order, then
    /// `SHOULD`-level. Empty when it states no rule.
    pub statements: Vec<Requirement>,
}

/// One normative statement.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Requirement {
    /// The section or step that holds it, pinned.
    pub citation: String,
    /// `sentence`, `row` (a claim or parameter table row, `term: value`)
    /// or `step`.
    pub kind: String,
    /// Its requirement keywords, such as `MUST NOT`, and `PROHIBITED` for
    /// a row that says so. A step of the section asked about may have
    /// none: every step of it is listed.
    pub keywords: Vec<String>,
    /// The statement: a step as stored, a sentence or row with each run of
    /// whitespace one space.
    pub text: String,
}

impl fmt::Display for Requirements {
    /// The human form: the section asked about, then each governing
    /// section and its statements, one per line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.citation)?;
        if let Some(title) = &self.title {
            write!(f, " · {title}")?;
        }
        writeln!(f, " · {} · indexed {}", self.license_class, self.indexed_at)?;
        for section in &self.sections {
            write!(f, "\n{}", section.citation)?;
            if let Some(title) = &section.title {
                write!(f, " · {title}")?;
            }
            writeln!(f, " ({})", section.relation)?;
            for statement in &section.statements {
                if statement.keywords.is_empty() {
                    writeln!(f, "  - {}", statement.text)?;
                } else {
                    let keywords = statement.keywords.join(", ");
                    writeln!(f, "  [{keywords}] {}", statement.text)?;
                }
            }
        }
        if self.truncated {
            writeln!(f, "\n… (truncated)")?;
        }
        Ok(())
    }
}

/// A section or step row of one document version.
struct Chunk {
    id: i64,
    number: String,
    title: Option<String>,
    parent: Option<i64>,
    start_line: i64,
    end_line: i64,
    text: String,
}

/// Why a section governs, in the order a section's first reason wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Relation {
    Asked,
    Subsection,
    Referenced,
    Referring,
    Context,
}

impl Relation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Asked => "asked",
            Self::Subsection => "subsection",
            Self::Referenced => "referenced",
            Self::Referring => "referring",
            Self::Context => "context",
        }
    }
}

impl Store {
    /// Every normative statement governing the section `citation` names:
    /// a canonical spec citation or one as people write it, as
    /// [`Store::read`] takes it; a step asks about its section.
    ///
    /// The governing sections are, in one document version: the section,
    /// with every one of its steps; its subsections, with theirs; the
    /// sections it or a subsection names (`Section 2.4`, but not
    /// `Section 2 of [JWT]`) or whose title of two or more words its title
    /// holds; the sections that name it or a subsection; and the enclosing
    /// section of each such referring subsection. Elsewhere only steps
    /// with a requirement keyword count. Statements come from
    /// [`crate::normative_statements`].
    ///
    /// A citation `read` refuses is the same error, and a code citation is
    /// [`Error::NotASpecCitation`].
    pub fn requirements(
        &self,
        citation: &str,
        options: &RequirementsOptions,
    ) -> Result<Requirements> {
        let read = self.read(
            citation,
            &ReadOptions {
                max_lines: 1,
                allow_stale: options.allow_stale,
            },
        )?;
        let Ok(Citation::Spec(spec)) = read.citation.parse::<Citation>() else {
            return Err(Error::NotASpecCitation {
                text: citation.trim().into(),
            });
        };
        let source = self
            .sources(spec.doc(), true)?
            .into_iter()
            .find(|s| s.revision.starts_with(spec.hash().as_str()))
            .ok_or_else(|| Error::UnknownVersion {
                name: spec.doc().into(),
                version: format!("sha256:{}", spec.hash()),
                known: Vec::new(),
            })?;
        self.requirements_of(&source, &spec.section().to_string())
    }

    /// The requirements of the section of `document` (a name or alias)
    /// that a spec search for `topic` ranks first in its current version,
    /// among the first ten hits with a citation form.
    /// No such section is [`Error::NoSectionOnTopic`].
    pub fn requirements_on(
        &self,
        topic: &str,
        document: &str,
        options: &RequirementsOptions,
    ) -> Result<Requirements> {
        self.check_fresh(options.allow_stale)?;
        let aliases = self.aliases()?;
        let name = aliases.get(document.trim()).unwrap_or(document.trim());
        let Some(source) = self.current_version(name)? else {
            let versions = self.sources(name, true)?;
            if versions.is_empty() {
                return Err(Error::UnknownSource {
                    name: name.into(),
                    known: self.source_names(true)?,
                });
            }
            return Err(Error::UnknownVersion {
                name: name.into(),
                version: "the current version".into(),
                known: versions.iter().map(pinned).collect(),
            });
        };
        let glob = glob_literal(name);
        let results = self.search(&SearchQuery {
            text: topic,
            language: None,
            path: Some(&glob),
            only: Some(Scope::Spec),
            limit: TOPIC_HITS,
            allow_stale: options.allow_stale,
        })?;
        // An appendix is indexed and found, but has no citation form.
        let citable = |citation: &str| matches!(citation.parse(), Ok(Citation::Spec(_)));
        let Some(top) = results.iter().find(|r| citable(&r.citation)) else {
            return Err(Error::NoSectionOnTopic {
                document: pinned(&source),
                topic: topic.into(),
            });
        };
        self.requirements(&top.citation, options)
    }

    /// The requirements of section `number` of one document version.
    fn requirements_of(&self, source: &SourceRow, number: &str) -> Result<Requirements> {
        let document = pinned(source);
        let sections = self.version_chunks(source.id, "section")?;
        let steps = self.version_chunks(source.id, "step")?;
        let Some(asked) = sections.iter().position(|s| s.number == number) else {
            return Err(Error::NoSuchSection {
                document,
                section: number.into(),
                candidates: Vec::new(),
            });
        };
        let relations = governing(&sections, asked);

        let mut order: Vec<usize> = (0..sections.len())
            .filter(|i| relations.contains_key(i))
            .collect();
        // The asked section, its subsections, then the rest, each in
        // document order.
        order.sort_by_key(|i| (relations[i].min(Relation::Referenced), *i));

        let mut budget = MAX_READ_BYTES;
        let mut truncated = false;
        let mut groups = Vec::new();
        for i in order {
            let section = &sections[i];
            let relation = relations[&i];
            let all_steps = matches!(relation, Relation::Asked | Relation::Subsection);
            let citation = format!("{document}#section-{}", section.number);
            let mut statements = statements_of(section, &steps, &citation, all_steps);
            let fits = statements
                .iter()
                .take_while(|s| {
                    let fits = s.text.len() <= budget;
                    if fits {
                        budget -= s.text.len();
                    }
                    fits
                })
                .count();
            if fits < statements.len() {
                statements.truncate(fits);
                truncated = true;
            }
            groups.push(RequirementSection {
                citation,
                title: section.title.clone(),
                relation: relation.as_str().into(),
                statements,
            });
            if truncated {
                break;
            }
        }
        let asked = &sections[asked];
        Ok(Requirements {
            citation: format!("{document}#section-{}", asked.number),
            title: asked.title.clone(),
            license_class: source.license_class.clone(),
            indexed_at: source.indexed_at.clone(),
            sections: groups,
            truncated,
            instruction: instruction(&source.license_class, truncated),
        })
    }

    /// The `kind` chunks of a document version, in document order.
    fn version_chunks(&self, source_id: i64, kind: &str) -> Result<Vec<Chunk>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.id, c.number, c.title, c.parent_id, c.start_line, c.end_line, c.text
             FROM chunk c JOIN document d ON d.id = c.document_id
             WHERE d.source_id = ?1 AND c.kind = ?2 AND c.number IS NOT NULL
             ORDER BY c.id",
        )?;
        let rows = stmt.query_map(rusqlite::params![source_id, kind], |row| {
            Ok(Chunk {
                id: row.get(0)?,
                number: row.get(1)?,
                title: row.get(2)?,
                parent: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                text: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// The governing sections of `sections[asked]`, by index, with why each
/// governs. Sections without a canonical number are left out.
fn governing(sections: &[Chunk], asked: usize) -> HashMap<usize, Relation> {
    let index: HashMap<i64, usize> = sections
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, i))
        .collect();
    let by_number: HashMap<&str, usize> = sections
        .iter()
        .enumerate()
        .rev()
        .map(|(i, s)| (s.number.as_str(), i))
        .collect();
    let citable = |i: &usize| sections[*i].number.parse::<SectionNumber>().is_ok();
    let mut relations = HashMap::new();
    let add = |relations: &mut HashMap<usize, Relation>, i: usize, relation: Relation| {
        if citable(&i) {
            relations.entry(i).or_insert(relation);
        }
    };
    add(&mut relations, asked, Relation::Asked);

    // The subsections, at any depth.
    let mut family = vec![asked];
    let mut k = 0;
    while k < family.len() {
        let id = sections[family[k]].id;
        for (i, s) in sections.iter().enumerate() {
            if s.parent == Some(id) && !family.contains(&i) {
                family.push(i);
                add(&mut relations, i, Relation::Subsection);
            }
        }
        k += 1;
    }
    let family_set: HashSet<usize> = family.iter().copied().collect();
    let family_numbers: HashSet<&str> = family
        .iter()
        .map(|&i| sections[i].number.as_str())
        .collect();

    // What the asked section and its subsections name.
    for &i in &family {
        for number in section_references(&sections[i].text) {
            if let Some(&j) = by_number.get(number.as_str())
                && !family_set.contains(&j)
            {
                add(&mut relations, j, Relation::Referenced);
            }
        }
    }
    if let Some(title) = sections[asked].title.as_deref() {
        for (j, s) in sections.iter().enumerate() {
            if let Some(other) = s.title.as_deref()
                && !family_set.contains(&j)
                && other.split_whitespace().count() >= 2
                && holds_phrase(title, other)
            {
                add(&mut relations, j, Relation::Referenced);
            }
        }
    }

    // What names them, and the enclosing section of a referring
    // subsection.
    for (j, s) in sections.iter().enumerate() {
        if family_set.contains(&j) {
            continue;
        }
        let refers = section_references(&s.text)
            .iter()
            .any(|n| family_numbers.contains(n.as_str()));
        if refers && citable(&j) {
            add(&mut relations, j, Relation::Referring);
            if let Some(&parent) = s.parent.as_ref().and_then(|p| index.get(p))
                && !family_set.contains(&parent)
            {
                add(&mut relations, parent, Relation::Context);
            }
        }
    }
    relations
}

/// The normative statements of `section`: its sentences and rows, with
/// its steps' lines left to the steps, and its steps (every one with
/// `all_steps`, else those with a requirement keyword). `MUST`-level
/// sentences and rows come first, then the steps, then `SHOULD`-level.
fn statements_of(
    section: &Chunk,
    steps: &[Chunk],
    citation: &str,
    all_steps: bool,
) -> Vec<Requirement> {
    let own: Vec<&Chunk> = steps
        .iter()
        .filter(|s| s.parent == Some(section.id) && s.number.parse::<u32>().is_ok())
        .collect();
    let in_step = |line: usize| {
        let line = section.start_line + i64::try_from(line).unwrap_or(i64::MAX);
        own.iter()
            .any(|s| s.start_line <= line && line <= s.end_line)
    };
    let text: Vec<&str> = section
        .text
        .lines()
        .enumerate()
        .map(|(i, line)| if in_step(i) { "" } else { line })
        .collect();

    let mut ranked: Vec<(u8, Requirement)> = Vec::new();
    for statement in normative_statements(&text.join("\n")) {
        let mut keywords: Vec<String> = statement
            .keywords
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect();
        if statement.prohibited {
            keywords.push("PROHIBITED".into());
        }
        let tier = if statement.level() == Level::Must {
            0
        } else {
            2
        };
        let kind = match statement.kind {
            StatementKind::Sentence => "sentence",
            StatementKind::Row => "row",
        };
        ranked.push((
            tier,
            Requirement {
                citation: citation.into(),
                kind: kind.into(),
                keywords,
                text: statement.text,
            },
        ));
    }
    for step in own {
        let keywords = requirement_keywords(&step.text);
        if !all_steps && normative_level(&keywords).is_none() {
            continue;
        }
        ranked.push((
            1,
            Requirement {
                citation: format!("{citation}/step-{}", step.number),
                kind: "step".into(),
                keywords: keywords.iter().map(|k| k.as_str().to_owned()).collect(),
                text: step.text.trim().to_owned(),
            },
        ));
    }
    // A stable sort keeps document order within a tier.
    ranked.sort_by_key(|(tier, _)| *tier);
    ranked.into_iter().map(|(_, r)| r).collect()
}

/// `Section 2.4`, `Sections 4.1 and 4.2`, `Section 4.1, 4.2 or 5`.
static SECTION_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bSections?\s+(\d+(?:\.\d+)*(?:(?:\s*,\s*|\s*,?\s+(?:and|or)\s+)\d+(?:\.\d+)*)*)")
        .expect("a valid pattern")
});

/// A section number in a [`SECTION_REFERENCE`] match.
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)*").expect("a valid pattern"));

/// A document named just before a reference: `RFC 7519`, `BCP 14`, `STD 68`.
static DOCUMENT_BEFORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:RFC|BCP|STD)\s*\d+$").expect("a valid pattern"));

/// A document named just after a reference: `of [JWT]`, `in RFC 7519`,
/// but not `of this specification`.
static DOCUMENT_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:of|in)\s+(?:\[|(?:RFC|BCP|STD)\s*\d|[A-Z][A-Za-z0-9.-]*\s*\[)")
        .expect("a valid pattern")
});

/// The section numbers `text` names in its own document. A reference into
/// another document is left out: one with a document right before it
/// (`[RFC3986] Section 3.4`, `[RFC6749], Section 4.1`, `[RFC2617]
/// (Section 2)`, `RFC 7519 Section 4`) or right after it (`Section 2 of
/// [JWT]`, `Section 7.1 in [RFC7519]`, `Section 3 of RFC 7519`).
pub(crate) fn section_references(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for m in SECTION_REFERENCE.find_iter(text) {
        let before = text[..m.start()]
            .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ',' | '('));
        let after = text[m.end()..].trim_start();
        if before.ends_with(']')
            || DOCUMENT_BEFORE.is_match(before)
            || DOCUMENT_AFTER.is_match(after)
        {
            continue;
        }
        found.extend(NUMBER.find_iter(m.as_str()).map(|n| n.as_str().to_owned()));
    }
    found
}

/// Whether `phrase` is a run of whole words in `text`.
fn holds_phrase(text: &str, phrase: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    let want: Vec<&str> = phrase.split_whitespace().collect();
    !want.is_empty() && words.windows(want.len()).any(|w| w == want.as_slice())
}

/// `name` as a SQLite `GLOB` pattern that matches only itself.
fn glob_literal(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '*' | '?' | '[' => format!("[{c}]"),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, DocumentRecord, Keyword, SourceRecord, SpecAliases};

    const LOGOUT: &str = "1060000000000000000000000000000000000000000000000000000000000106";
    const PINNED: &str = "logout-1_0@sha256:106000000000";
    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    fn chunk<'a>(
        kind: &'a str,
        number: &'a str,
        title: Option<&'a str>,
        parent: Option<usize>,
        start: u32,
        text: &'a str,
    ) -> ChunkRecord<'a> {
        let end = start + u32::try_from(text.lines().count()).unwrap() - 1;
        ChunkRecord {
            kind,
            start_line: start,
            end_line: end,
            text,
            number: Some(number),
            anchor: None,
            title,
            parent,
            strength: &[Keyword::Must],
            symbol: None,
        }
    }

    fn section<'a>(
        number: &'a str,
        title: &'a str,
        parent: Option<usize>,
        start: u32,
        text: &'a str,
    ) -> ChunkRecord<'a> {
        chunk("section", number, Some(title), parent, start, text)
    }

    fn step<'a>(number: &'a str, parent: usize, line: u32, text: &'a str) -> ChunkRecord<'a> {
        chunk("step", number, None, Some(parent), line, text)
    }

    /// A logout spec. §2.2 is the one asked about: it has two steps and a
    /// subsection, names §3, and has §2.1's title in its own; §4.1 names
    /// §2.2, and its parent §4 holds no rule; §2's reference is into
    /// another document; §5 and the appendix are unrelated or uncitable.
    fn fixture() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let app = store
            .upsert_source(&SourceRecord {
                name: "app",
                kind: "local",
                url: ".",
                revision: HEAD,
                license_spdx: None,
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        let code = DocumentRecord {
            path: "src/lib.rs",
            language: Some("rust"),
            content_hash: b"lib",
            size_bytes: 1,
            dirty: false,
            text: Some("fn a() {}\n"),
            chunks: Vec::new(),
        };
        store.write_document(app, &code).unwrap();
        let logout = store
            .upsert_source(&SourceRecord {
                name: "logout-1_0",
                kind: "document",
                url: "https://example.com",
                revision: LOGOUT,
                license_spdx: None,
                license_class: "quote",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        let chunks = vec![
            section(
                "2",
                "Logout",
                None,
                10,
                "2. Logout\n\nThe OP SHOULD log out. See Section 2.2 of [Core].",
            ),
            section(
                "2.1",
                "Logout Token",
                Some(0),
                20,
                "2.1. Logout Token\n\nexp\nREQUIRED. Expiration time.\nsub\nOPTIONAL. Subject.\n\nA token MUST be signed.",
            ),
            section(
                "2.2",
                "Logout Token Validation",
                Some(0),
                30,
                "2.2. Logout Token Validation\n\nThe RP MUST validate the token as follows:\n1. Decrypt it. The RP SHOULD reject it otherwise.\n2. Validate the signature.\n\nThe RP SHOULD log failures. It MUST NOT accept tokens but per Section 3.",
            ),
            step(
                "1",
                2,
                33,
                "1. Decrypt it. The RP SHOULD reject it otherwise.",
            ),
            step("2", 2, 34, "2. Validate the signature."),
            section(
                "2.2.1",
                "Errors",
                Some(2),
                40,
                "2.2.1. Errors\n\nThe RP MUST return 400.",
            ),
            section(
                "3",
                "Endpoints",
                None,
                50,
                "3. Endpoints\n\nThe endpoint MUST use TLS.\n1. Connect.\n2. You MUST check the certificate.",
            ),
            step("1", 6, 53, "1. Connect."),
            step("2", 6, 54, "2. You MUST check the certificate."),
            section(
                "4",
                "Other",
                None,
                60,
                "4. Other\n\nNothing here MAY apply.",
            ),
            section(
                "4.1",
                "Use",
                Some(9),
                70,
                "4.1. Use\n\nAn RP MUST validate tokens as in Section 2.2.",
            ),
            section(
                "5",
                "Unrelated",
                None,
                80,
                "5. Unrelated\n\nA server MUST NOT do X.",
            ),
            section(
                "A.1",
                "Appendix",
                None,
                90,
                "A.1. Appendix\n\nIt MUST follow Section 2.2.",
            ),
        ];
        let doc = DocumentRecord {
            path: "logout-1_0",
            language: None,
            content_hash: b"logout",
            size_bytes: 1,
            dirty: false,
            text: None,
            chunks,
        };
        store.write_document(logout, &doc).unwrap();
        let aliases: SpecAliases = [("Logout", "logout-1_0")].into_iter().collect();
        store.set_aliases(&aliases).unwrap();
        store.set_indexed_at(30).unwrap();
        drop(store);
        (Store::open_read_only(&path).unwrap(), dir)
    }

    fn summary(r: &Requirements) -> Vec<(String, String, Vec<String>)> {
        r.sections
            .iter()
            .map(|s| {
                (
                    s.citation.trim_start_matches(PINNED).to_owned(),
                    s.relation.clone(),
                    s.statements.iter().map(|st| st.text.clone()).collect(),
                )
            })
            .collect()
    }

    fn owned(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| (*t).to_owned()).collect()
    }

    #[test]
    fn a_section_with_its_subsections_and_the_sections_linked_to_it() {
        let (store, _dir) = fixture();
        let options = RequirementsOptions::default();
        let r = store.requirements("Logout §2.2 step 1", &options).unwrap();
        assert_eq!(r.citation, format!("{PINNED}#section-2.2"));
        assert_eq!(r.title.as_deref(), Some("Logout Token Validation"));
        assert_eq!(r.license_class, "quote");
        assert_eq!(r.instruction.as_deref(), Some(crate::QUOTE_INSTRUCTION));
        assert!(!r.truncated);
        assert_eq!(
            summary(&r),
            [
                (
                    "#section-2.2".into(),
                    "asked".into(),
                    owned(&[
                        "The RP MUST validate the token as follows:",
                        "It MUST NOT accept tokens but per Section 3.",
                        "1. Decrypt it. The RP SHOULD reject it otherwise.",
                        "2. Validate the signature.",
                        "The RP SHOULD log failures.",
                    ])
                ),
                (
                    "#section-2.2.1".into(),
                    "subsection".into(),
                    owned(&["The RP MUST return 400."])
                ),
                (
                    "#section-2.1".into(),
                    "referenced".into(),
                    owned(&["exp: REQUIRED. Expiration time.", "A token MUST be signed."])
                ),
                (
                    "#section-3".into(),
                    "referenced".into(),
                    owned(&[
                        "The endpoint MUST use TLS.",
                        "2. You MUST check the certificate."
                    ])
                ),
                ("#section-4".into(), "context".into(), Vec::new()),
                (
                    "#section-4.1".into(),
                    "referring".into(),
                    owned(&["An RP MUST validate tokens as in Section 2.2."])
                ),
            ]
        );
        let asked = &r.sections[0].statements;
        assert_eq!(asked[0].kind, "sentence");
        assert_eq!(asked[0].citation, format!("{PINNED}#section-2.2"));
        assert_eq!(asked[0].keywords, ["MUST"]);
        assert_eq!(asked[2].kind, "step");
        assert_eq!(asked[2].citation, format!("{PINNED}#section-2.2/step-1"));
        assert_eq!(asked[2].keywords, ["SHOULD"]);
        assert_eq!(asked[3].keywords, Vec::<String>::new());
        assert_eq!(r.sections[2].statements[0].kind, "row");

        // The canonical citation and a URL-free written one agree.
        let canonical = store
            .requirements(&format!("{PINNED}#section-2.2"), &options)
            .unwrap();
        assert_eq!(canonical, r);
        let human = r.to_string();
        assert!(
            human.starts_with(&format!("{PINNED}#section-2.2 · Logout Token Validation")),
            "{human}"
        );
        assert!(
            human.contains("#section-2.1 · Logout Token (referenced)"),
            "{human}"
        );
        assert!(
            human.contains("  [REQUIRED] exp: REQUIRED. Expiration time."),
            "{human}"
        );
    }

    #[test]
    fn a_topic_in_a_document_picks_the_section() {
        let (store, _dir) = fixture();
        let options = RequirementsOptions::default();
        let r = store
            .requirements_on("decrypt signature", "Logout", &options)
            .unwrap();
        assert_eq!(r.citation, format!("{PINNED}#section-2.2"));
        let err = store
            .requirements_on("zebra", "logout-1_0", &options)
            .unwrap_err();
        assert_eq!(err.kind(), "no_section_on_topic");
        assert!(err.to_string().contains(PINNED), "{err}");
        let err = store
            .requirements_on("decrypt", "Nope", &options)
            .unwrap_err();
        assert_eq!(err.kind(), "unknown_source");
    }

    #[test]
    fn references_into_other_documents_are_not_sections_of_this_one() {
        let text = "See Section 2.4 and Sections 3, 4.1 or 5. Not [RFC6749], Section 4.1, \
            RFC 7519 Section 6, Section 7.1 in [RFC7519], [RFC2617] (Section 8), \
            Section 9 of [JWT], [RFC3986] Section 10, Section 11 of RFC 7519 \
            or BCP 14 Section 12, but Section 13 of this specification.";
        assert_eq!(section_references(text), ["2.4", "3", "4.1", "5", "13"]);
    }

    #[test]
    fn a_topic_skips_hits_that_have_no_citation_form() {
        let (store, _dir) = fixture();
        let options = RequirementsOptions::default();
        // Only the appendix holds "follow".
        let err = store
            .requirements_on("follow", "Logout", &options)
            .unwrap_err();
        assert_eq!(err.kind(), "no_section_on_topic", "{err}");
        // The appendix ranks first for "follow section", §4.1 next.
        let r = store
            .requirements_on("follow section", "Logout", &options)
            .unwrap();
        assert!(r.citation.starts_with(PINNED), "{}", r.citation);
    }

    #[test]
    fn what_is_not_a_spec_section_is_an_error() {
        let (store, _dir) = fixture();
        let options = RequirementsOptions::default();
        let code = format!("app@{HEAD}:src/lib.rs#L1");
        let err = store.requirements(&code, &options).unwrap_err();
        assert_eq!(err.kind(), "not_a_spec_citation");
        assert!(err.to_string().contains("requirements"), "{err}");
        for (citation, kind) in [
            ("Logout §9", "no_such_section"),
            ("hello", "not_a_citation"),
            ("RFC 1 §1", "unknown_source"),
        ] {
            let err = store.requirements(citation, &options).unwrap_err();
            assert_eq!(err.kind(), kind, "{citation}");
        }
    }

    #[test]
    fn a_stale_index_is_refused_unless_allowed() {
        let (store, _dir) = fixture();
        rusqlite::Connection::open(store.path())
            .unwrap()
            .execute(
                "UPDATE meta SET value = '2000-01-01T00:00:00Z' WHERE key = 'indexed_at'",
                [],
            )
            .unwrap();
        let err = store
            .requirements("Logout §2.2", &RequirementsOptions::default())
            .unwrap_err();
        assert_eq!(err.kind(), "stale_index");
        let err = store
            .requirements_on("decrypt", "Logout", &RequirementsOptions::default())
            .unwrap_err();
        assert_eq!(err.kind(), "stale_index");
        let allowed = RequirementsOptions { allow_stale: true };
        assert!(store.requirements("Logout §2.2", &allowed).is_ok());
    }

    #[test]
    fn a_huge_section_is_cut_at_the_byte_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.db");
        let mut store = Store::open_read_write(&path).unwrap();
        let id = store
            .upsert_source(&SourceRecord {
                name: "big",
                kind: "document",
                url: "https://example.com",
                revision: LOGOUT,
                license_spdx: None,
                license_class: "quote",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        let mut text = String::from("1. Big\n");
        for i in 0..2000 {
            text.push_str(&format!(
                "\nRule {i}: the server MUST do this one thing well."
            ));
        }
        let doc = DocumentRecord {
            path: "big",
            language: None,
            content_hash: b"big",
            size_bytes: 1,
            dirty: false,
            text: None,
            chunks: vec![section("1", "Big", None, 1, &text)],
        };
        store.write_document(id, &doc).unwrap();
        store.set_indexed_at(30).unwrap();
        let r = store
            .requirements(
                "big@sha256:106000000000#section-1",
                &RequirementsOptions::default(),
            )
            .unwrap();
        assert!(r.truncated);
        let bytes: usize = r.sections[0].statements.iter().map(|s| s.text.len()).sum();
        assert!(bytes <= crate::MAX_READ_BYTES, "{bytes}");
        assert!(r.sections[0].statements.len() > 100);
        assert_eq!(
            r.instruction.as_deref(),
            Some(crate::QUOTE_TRUNCATED_INSTRUCTION)
        );
    }
}
