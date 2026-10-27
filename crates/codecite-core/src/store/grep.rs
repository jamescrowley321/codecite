//! Grepping a corpus: the lines of its stored text that match an exact
//! string or a regular expression, each cited, in index order.

use std::collections::HashMap;

use regex::{Regex, RegexBuilder};
use rusqlite::params;
use schemars::JsonSchema;
use serde::Serialize;

use super::Store;
use super::read::{byte_prefix, file_lines};
use super::search::{MAX_SNIPPET_BYTES, Scope, code_citation, spec_citation};
use crate::{Error, Result, Role, SectionNumber};

/// A grep: the pattern, how to read it, the filters and how many matches to
/// return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepQuery<'a> {
    /// A regular expression (the `regex` crate's syntax), or with `fixed`
    /// the exact text to find. It is matched against one line at a time, so
    /// `^` and `$` are the line's ends.
    pub pattern: &'a str,
    /// Read the pattern as literal text rather than a regular expression.
    pub fixed: bool,
    /// Match letters whatever their case.
    pub ignore_case: bool,
    /// Only documents in this language, as the index names it (`rust`).
    pub language: Option<&'a str>,
    /// Only documents whose path matches this glob (SQLite `GLOB`: `*`
    /// matches any run of characters, `/` included, and case matters).
    pub path: Option<&'a str>,
    /// Only specs or only code. Without it, a grep covers both.
    pub only: Option<Scope>,
    /// The most matches to return.
    pub limit: u32,
    /// Grep an index older than its maximum age instead of refusing it.
    pub allow_stale: bool,
}

/// The matching lines, in index order: what `codecite grep --json` prints
/// and the MCP `grep` tool returns, as the same JSON.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct GrepResults {
    /// The matching lines; none when nothing matched.
    pub matches: Vec<GrepMatch>,
}

/// One matching line, cited.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct GrepMatch {
    /// Where the line is: `repo@sha:path#L12` for code; for a spec, the
    /// step or section that holds it, `doc@sha256:<12 hex>#section-4.3[/step-2]`.
    pub citation: String,
    /// `code` for a line of a file, `section` or `step` for a spec, as
    /// `read` names them.
    pub kind: String,
    /// What the line's document is for: `source`, `test`, `example`,
    /// `docs` or `spec`.
    pub role: &'static str,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    pub license_class: String,
    /// When the line's source was last indexed.
    pub indexed_at: String,
    /// For code, whether the file differed from the cited commit when it
    /// was indexed. Spec matches have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    /// The line, at most 4 KiB of it (`MAX_SNIPPET_BYTES`, a snippet's
    /// budget): a minified file can hold a line of a megabyte. A cut keeps a
    /// prefix ending on a character boundary, with no marker in it; `read`
    /// the citation for the rest.
    pub text: String,
    /// Whether the byte budget cut `text` short of the whole line.
    pub truncated: bool,
}

/// A document row, as `grep` needs it.
struct DocRow {
    id: i64,
    path: String,
    dirty: bool,
    source: String,
    source_kind: String,
    revision: String,
    license_class: String,
    indexed_at: String,
}

impl DocRow {
    /// A match on this document's line, cited as `citation`.
    fn matched(&self, citation: String, kind: &str, line: &str) -> GrepMatch {
        let is_spec = self.source_kind == "document";
        let (text, truncated) = byte_prefix(line, MAX_SNIPPET_BYTES);
        GrepMatch {
            citation,
            kind: kind.into(),
            role: Role::of(&self.path, &self.source_kind).as_str(),
            license_class: self.license_class.clone(),
            indexed_at: self.indexed_at.clone(),
            dirty: (!is_spec).then_some(self.dirty),
            text: text.to_owned(),
            truncated,
        }
    }
}

/// A spec section's or step's place, as `grep` cites it.
struct SpecSpan {
    start: u32,
    end: u32,
    number: String,
}

impl Store {
    /// Returns at most `query.limit` lines that match `query.pattern`, in
    /// index order: sources in the order they were stored, then their
    /// documents by path, then lines. Nothing is scored.
    ///
    /// A code file's stored text, which `read` cites, is matched line by
    /// line, and each match cites its one line. A spec has no file text, so
    /// its sections are matched instead; a line inside one of a section's
    /// steps cites the step, any other line the section. A section whose
    /// number has no canonical citation, like an appendix's, is skipped.
    /// Only the current version of each document is grepped.
    ///
    /// A pattern that is empty, holds a newline (or `\n`, `\r`) or is not a
    /// valid regular expression is [`Error::BadPattern`]. An index older than
    /// its maximum age is [`Error::StaleIndex`] unless `query.allow_stale`
    /// is set.
    pub fn grep(&self, query: &GrepQuery<'_>) -> Result<Vec<GrepMatch>> {
        let re = pattern(query)?;
        self.check_fresh(query.allow_stale)?;
        let only_spec = query.only.map(|scope| scope == Scope::Spec);
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
        let mut matches = Vec::new();
        if limit == 0 {
            return Ok(matches);
        }
        // Without the text, which a sort would otherwise copy for every
        // file before the first one is scanned.
        let mut docs = self.conn.prepare_cached(
            "SELECT d.id, d.path, d.dirty, s.name, s.kind, s.revision,
               s.license_class, s.indexed_at
             FROM document d JOIN source s ON s.id = d.source_id
             WHERE s.is_current
               AND (?1 IS NULL OR d.language = ?1)
               AND (?2 IS NULL OR d.path GLOB ?2)
               AND (?3 IS NULL OR (s.kind = 'document') = ?3)
             ORDER BY s.id, d.path",
        )?;
        let mut rows = docs.query(params![query.language, query.path, only_spec])?;
        while let Some(row) = rows.next()? {
            let doc = DocRow {
                id: row.get(0)?,
                path: row.get(1)?,
                dirty: row.get(2)?,
                source: row.get(3)?,
                source_kind: row.get(4)?,
                revision: row.get(5)?,
                license_class: row.get(6)?,
                indexed_at: row.get(7)?,
            };
            if doc.source_kind == "document" {
                self.grep_spec(&doc, &re, limit, &mut matches)?;
            } else {
                self.grep_code(&doc, &re, limit, &mut matches)?;
            }
            if matches.len() >= limit {
                break;
            }
        }
        Ok(matches)
    }

    /// Adds the matching lines of a code file to `matches`, up to `limit`.
    fn grep_code(
        &self,
        doc: &DocRow,
        re: &Regex,
        limit: usize,
        matches: &mut Vec<GrepMatch>,
    ) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT text FROM document WHERE id = ?1")?;
        let text: Option<String> = stmt.query_row([doc.id], |row| row.get(0))?;
        let text = text.ok_or_else(|| Error::InvalidCorpus {
            path: self.path.clone(),
            reason: format!("it holds no text for {}; re-run codecite index", doc.path),
        })?;
        for (n, line) in (1..).zip(file_lines(&text)) {
            if matches.len() >= limit {
                break;
            }
            if re.is_match(line) {
                let citation = code_citation(&doc.source, &doc.revision, &doc.path, n, n);
                matches.push(doc.matched(citation, "code", line));
            }
        }
        Ok(())
    }

    /// Adds the matching lines of a spec's sections to `matches`, up to
    /// `limit`, each cited as the step that holds it or else its section.
    fn grep_spec(
        &self,
        doc: &DocRow,
        re: &Regex,
        limit: usize,
        matches: &mut Vec<GrepMatch>,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, kind, start_line, end_line, number, parent_id, text
             FROM chunk WHERE document_id = ?1 AND kind IN ('section', 'step')
             ORDER BY start_line, id",
        )?;
        let mut sections: Vec<(i64, SpecSpan, String)> = Vec::new();
        let mut steps: HashMap<i64, Vec<SpecSpan>> = HashMap::new();
        let mut rows = stmt.query([doc.id])?;
        while let Some(row) = rows.next()? {
            let kind: String = row.get(1)?;
            let span = SpecSpan {
                start: row.get(2)?,
                end: row.get(3)?,
                number: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            };
            let parent: Option<i64> = row.get(5)?;
            match (kind.as_str(), parent) {
                ("step", Some(parent)) => steps.entry(parent).or_default().push(span),
                ("section", _) => sections.push((row.get(0)?, span, row.get(6)?)),
                _ => {}
            }
        }
        // A section whose number no canonical citation can hold, like an
        // appendix's `A.1`, is left out: its match would carry a citation
        // `read` refuses.
        let citable = sections
            .iter()
            .filter(|(_, section, _)| section.number.parse::<SectionNumber>().is_ok());
        for (id, section, text) in citable {
            let section_steps = steps.get(id).map_or(&[][..], Vec::as_slice);
            for (n, line) in (section.start..).zip(file_lines(text)) {
                if matches.len() >= limit {
                    return Ok(());
                }
                if !re.is_match(line) {
                    continue;
                }
                let step = section_steps.iter().find(|s| s.start <= n && n <= s.end);
                let (kind, number, parent) = match step {
                    Some(step) => ("step", &step.number, Some(section.number.clone())),
                    None => ("section", &section.number, None),
                };
                let citation = spec_citation(
                    &doc.source,
                    &doc.revision,
                    kind,
                    Some(number.clone()),
                    parent,
                );
                matches.push(doc.matched(citation, kind, line));
            }
        }
        Ok(())
    }
}

/// The query's pattern as a regex: escaped when `fixed`, and matching any
/// case when `ignore_case`.
fn pattern(query: &GrepQuery<'_>) -> Result<Regex> {
    let bad = |reason: &str| Error::BadPattern {
        pattern: query.pattern.into(),
        reason: reason.into(),
    };
    if query.pattern.is_empty() {
        return Err(bad("it is empty"));
    }
    // Each line is matched without its line ending, so a pattern that
    // names one could never match; ripgrep refuses it too.
    if names_newline(query.pattern, query.fixed) {
        return Err(bad(
            "it holds a newline, and grep matches one line at a time",
        ));
    }
    let source = if query.fixed {
        regex::escape(query.pattern)
    } else {
        query.pattern.to_owned()
    };
    RegexBuilder::new(&source)
        .case_insensitive(query.ignore_case)
        .build()
        .map_err(|err| Error::BadPattern {
            pattern: query.pattern.into(),
            reason: err.to_string(),
        })
}

/// Whether `pattern` holds a line break, or as a regex the escape `\n` or
/// `\r` (a backslash that is not itself escaped, then `n` or `r`).
fn names_newline(pattern: &str, fixed: bool) -> bool {
    if pattern.contains(['\n', '\r']) {
        return true;
    }
    if fixed {
        return false;
    }
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.next(), Some('n' | 'r')) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, Citation, DocumentRecord, Keyword, SourceRecord};
    use std::path::Path;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA256: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";
    const LIB: &str =
        "pub fn validate(jti: &str) -> bool {\n    // JTI must be unique\n    jti.len() > 8\n}\n";
    const CHECK: &str = "export function check(form) {\r\n  return form.token_type_hint === \"access_token\";\r\n}\r\n";
    const TEST: &str = "#[test]\nfn jti_is_checked() {}\n";

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

    fn code<'a>(
        path: &'a str,
        language: &'a str,
        dirty: bool,
        text: &'a str,
    ) -> DocumentRecord<'a> {
        DocumentRecord {
            path,
            language: Some(language),
            content_hash: path.as_bytes(),
            size_bytes: text.len() as u64,
            dirty,
            text: Some(text),
            // One chunk over the whole file: grep reads the file's text,
            // not its chunks.
            chunks: vec![chunk("window", (1, 1), text, None, &[])],
        }
    }

    fn chunk<'a>(
        kind: &'a str,
        lines: (u32, u32),
        text: &'a str,
        number: Option<&'a str>,
        strength: &'a [Keyword],
    ) -> ChunkRecord<'a> {
        ChunkRecord {
            kind,
            start_line: lines.0,
            end_line: lines.1,
            text,
            number,
            anchor: None,
            title: None,
            parent: None,
            strength,
            symbol: None,
        }
    }

    struct Fixture {
        store: Store,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn grep(&self, query: &GrepQuery<'_>) -> Vec<GrepMatch> {
            self.store.grep(query).unwrap()
        }

        fn citations(&self, query: &GrepQuery<'_>) -> Vec<String> {
            self.grep(query).into_iter().map(|m| m.citation).collect()
        }
    }

    fn query(pattern: &str) -> GrepQuery<'_> {
        GrepQuery {
            pattern,
            fixed: false,
            ignore_case: false,
            language: None,
            path: None,
            only: None,
            limit: 50,
            allow_stale: false,
        }
    }

    /// Writes a version of `rfc9449`: section 4.3 on lines 10-14, whose
    /// step 2 is lines 13-14 and starts with `step`, then section 5 on
    /// lines 15-16.
    fn write_spec(store: &mut Store, revision: &str, current: bool, step: &str) {
        let id = store
            .upsert_source(&source("rfc9449", "document", revision, current))
            .unwrap();
        let step_text = format!("{step}\nThe jti is kept.");
        let section_text = format!("4.3.  Checking DPoP Proofs\n\nTo check a proof:\n{step_text}");
        let mut step_chunk = chunk("step", (13, 14), &step_text, Some("2"), &[Keyword::Must]);
        step_chunk.parent = Some(0);
        let doc = DocumentRecord {
            path: "rfc9449",
            language: None,
            content_hash: revision.as_bytes(),
            size_bytes: 1,
            dirty: false,
            text: None,
            chunks: vec![
                chunk(
                    "section",
                    (10, 14),
                    &section_text,
                    Some("4.3"),
                    &[Keyword::Must],
                ),
                step_chunk,
                chunk(
                    "section",
                    (15, 16),
                    "5.  Security\nA jti MUST NOT repeat.",
                    Some("5"),
                    &[],
                ),
                // An appendix: no canonical citation can name it.
                chunk("section", (17, 17), "A.1.  Example jti", Some("A.1"), &[]),
            ],
        };
        store.write_document(id, &doc).unwrap();
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store
            .upsert_source(&source("app", "local", HEAD, true))
            .unwrap();
        for doc in [
            code("src/lib.rs", "rust", true, LIB),
            code("web/check.ts", "typescript", false, CHECK),
            code("tests/jti.rs", "rust", false, TEST),
        ] {
            store.write_document(app, &doc).unwrap();
        }
        write_spec(&mut store, SHA256, true, "2. The jti MUST be unique.");
        write_spec(&mut store, &"a".repeat(64), false, "2. The old jti text.");
        store.set_indexed_at(30).unwrap();
        drop(store);
        let store = Store::open_read_only(&dir.path().join("demo.db")).unwrap();
        Fixture { store, dir }
    }

    fn at(path: &str, line: u32) -> String {
        format!("app@{HEAD}:{path}#L{line}")
    }

    #[test]
    fn a_regex_matches_lines_in_index_order() {
        let f = fixture();
        let matches = f.grep(&query(r"jti\.len|jti_is"));
        let citations: Vec<&str> = matches.iter().map(|m| m.citation.as_str()).collect();
        assert_eq!(citations, [at("src/lib.rs", 3), at("tests/jti.rs", 2)]);
        let first = &matches[0];
        assert_eq!(first.kind, "code");
        assert_eq!(first.role, "source");
        assert_eq!(first.license_class, "adapt");
        assert_eq!(first.dirty, Some(true));
        assert_eq!(first.text, "    jti.len() > 8");
        assert!(matches!(first.citation.parse(), Ok(Citation::Code(_))));
        assert_eq!(matches[1].role, "test");
        assert_eq!(matches[1].dirty, Some(false));
    }

    #[test]
    fn a_regex_class_and_anchors_apply_to_each_line() {
        let f = fixture();
        // `\w+` and the CRLF line ending, which is not part of the line.
        let matches = f.grep(&query(r#"token_type_hint === "\w+";$"#));
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(matches[0].citation, at("web/check.ts", 2));
        assert_eq!(
            matches[0].text,
            "  return form.token_type_hint === \"access_token\";"
        );
        // `^` is each line's start, not the file's.
        let starts = f.citations(&GrepQuery {
            only: Some(Scope::Code),
            ..query("^fn ")
        });
        assert_eq!(starts, [at("tests/jti.rs", 2)]);
    }

    #[test]
    fn fixed_reads_the_pattern_as_text() {
        let f = fixture();
        // As a regex, `.` matches any character and `(` opens a group.
        assert!(matches!(
            f.store.grep(&query("validate(")),
            Err(Error::BadPattern { .. })
        ));
        let fixed = GrepQuery {
            fixed: true,
            ..query("validate(jti")
        };
        assert_eq!(f.citations(&fixed), [at("src/lib.rs", 1)]);
        let dot = GrepQuery {
            fixed: true,
            ..query("jti.")
        };
        assert_eq!(f.citations(&dot), [at("src/lib.rs", 3)]);
        let code = GrepQuery {
            only: Some(Scope::Code),
            ..query("jti.")
        };
        assert_eq!(
            f.citations(&code).len(),
            3,
            "a regex `.` matches `:`, `.` and `_`"
        );
    }

    #[test]
    fn ignore_case_matches_any_case() {
        let f = fixture();
        let code = |pattern, ignore_case| GrepQuery {
            ignore_case,
            only: Some(Scope::Code),
            ..query(pattern)
        };
        assert_eq!(f.citations(&code("JTI", false)), [at("src/lib.rs", 2)]);
        assert_eq!(f.grep(&code("JTI", true)).len(), 4);
        let fixed = GrepQuery {
            fixed: true,
            ..code("Must Be", true)
        };
        assert_eq!(f.citations(&fixed), [at("src/lib.rs", 2)]);
    }

    #[test]
    fn a_spec_line_cites_its_step_or_section() {
        let f = fixture();
        let only = |pattern| GrepQuery {
            only: Some(Scope::Spec),
            ..query(pattern)
        };
        let matches = f.grep(&only("jti"));
        let citations: Vec<&str> = matches.iter().map(|m| m.citation.as_str()).collect();
        // Only the current version, a line in step 2 cited as the step, and
        // section 5's line cited as the section.
        assert_eq!(
            citations,
            [
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2",
                "rfc9449@sha256:1a2b3c4d5e6f#section-5",
            ]
        );
        assert_eq!(matches[0].text, "2. The jti MUST be unique.");
        assert_eq!(matches[0].kind, "step");
        assert_eq!(matches[0].role, "spec");
        assert_eq!(matches[0].license_class, "quote");
        assert_eq!(matches[0].dirty, None);
        assert_eq!(matches[2].kind, "section");
        assert!(matches!(matches[0].citation.parse(), Ok(Citation::Spec(_))));
        // A line of the section outside its step cites the section.
        let heading = f.grep(&only("Checking DPoP"));
        assert_eq!(heading.len(), 1);
        assert_eq!(
            heading[0].citation,
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3"
        );
        assert_eq!(heading[0].kind, "section");
    }

    #[test]
    fn filters_and_the_limit_narrow_the_matches() {
        let f = fixture();
        let by_language = GrepQuery {
            language: Some("typescript"),
            ..query("token")
        };
        assert_eq!(f.citations(&by_language), [at("web/check.ts", 2)]);
        let by_path = GrepQuery {
            path: Some("tests/*"),
            ..query("jti")
        };
        assert_eq!(f.citations(&by_path), [at("tests/jti.rs", 2)]);
        // A spec has no language, so a language filter leaves specs out.
        let rust = GrepQuery {
            language: Some("rust"),
            ..query("jti")
        };
        assert!(f.grep(&rust).iter().all(|m| m.role != "spec"));
        let code = GrepQuery {
            only: Some(Scope::Code),
            ..query("jti")
        };
        assert!(f.grep(&code).iter().all(|m| m.kind == "code"));
        // Code comes first here because its source was stored first, and
        // its files come by path, not in the order they were written.
        let paths: Vec<String> = f
            .citations(&GrepQuery {
                only: Some(Scope::Code),
                ..query("[a-z]")
            })
            .iter()
            .map(|c| {
                c.split(':')
                    .nth(1)
                    .unwrap()
                    .split('#')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
        assert_eq!(paths.first().map(String::as_str), Some("src/lib.rs"));
        assert_eq!(paths.last().map(String::as_str), Some("web/check.ts"));
        let all = f.citations(&query("jti"));
        assert_eq!(all.len(), 6, "{all:?}");
        let limited = GrepQuery {
            limit: 2,
            ..query("jti")
        };
        assert_eq!(f.citations(&limited), all[..2]);
        let limited = GrepQuery {
            limit: 5,
            ..query("jti")
        };
        assert_eq!(f.citations(&limited), all[..5]);
        assert!(
            f.grep(&GrepQuery {
                limit: 0,
                ..query("jti")
            })
            .is_empty()
        );
        assert!(f.grep(&query("nothing matches this")).is_empty());
    }

    #[test]
    fn a_bad_regex_is_an_error_naming_it() {
        let f = fixture();
        let err = f.store.grep(&query("token_type_hint=(\\w+")).unwrap_err();
        let Error::BadPattern { pattern, reason } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(pattern, "token_type_hint=(\\w+");
        assert!(reason.contains("unclosed group"), "{reason}");
        assert_eq!(err.kind(), "bad_pattern");
        assert!(
            err.to_string()
                .starts_with("invalid pattern \"token_type_hint=(\\\\w+\": ")
        );
        // Checked before the index's age.
        backdate(&f.dir.path().join("demo.db"), 31);
        assert!(matches!(
            f.store.grep(&query("(")),
            Err(Error::BadPattern { .. })
        ));
    }

    #[test]
    fn an_empty_pattern_is_an_error() {
        let f = fixture();
        let err = f.store.grep(&query("")).unwrap_err();
        assert!(matches!(err, Error::BadPattern { .. }), "{err:?}");
        assert_eq!(err.to_string(), "invalid pattern \"\": it is empty");
        // A space is something to find.
        assert!(
            !f.grep(&GrepQuery {
                fixed: true,
                ..query(" ")
            })
            .is_empty()
        );
    }

    #[test]
    fn a_pattern_that_names_a_newline_is_an_error() {
        let f = fixture();
        for (pattern, fixed) in [
            ("jti\nlen", false),
            (r"jti\n", false),
            (r"\r?$", false),
            ("a\nb", true),
        ] {
            let err = f
                .store
                .grep(&GrepQuery {
                    fixed,
                    ..query(pattern)
                })
                .unwrap_err();
            assert!(
                err.to_string()
                    .ends_with("it holds a newline, and grep matches one line at a time"),
                "{pattern:?}: {err}"
            );
        }
        // An escaped backslash before `n`, and `\n` as fixed text, are not
        // newlines.
        assert!(!names_newline(r"a\\n", false));
        assert!(!names_newline(r"a\n", true));
        assert!(names_newline(r"a\\\n", false));
    }

    #[test]
    fn a_section_no_citation_can_name_is_left_out() {
        let f = fixture();
        let spec = GrepQuery {
            only: Some(Scope::Spec),
            ..query("Example jti")
        };
        assert!(f.grep(&spec).is_empty());
        // Every spec match reads back.
        for m in f.grep(&GrepQuery {
            only: Some(Scope::Spec),
            ..query("jti")
        }) {
            assert!(!m.citation.contains("A.1"), "{}", m.citation);
        }
    }

    #[test]
    fn a_stale_index_is_refused_unless_allowed() {
        let f = fixture();
        backdate(&f.dir.path().join("demo.db"), 31);
        let err = f.store.grep(&query("jti")).unwrap_err();
        assert!(matches!(err, Error::StaleIndex { .. }), "{err:?}");
        let stale = GrepQuery {
            allow_stale: true,
            ..query("jti")
        };
        assert!(!f.grep(&stale).is_empty());
    }

    #[test]
    fn a_long_line_is_held_to_the_snippet_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store
            .upsert_source(&source("app", "local", HEAD, true))
            .unwrap();
        // A two-byte `é` across the budget, as in a minified file.
        let long = format!("jti{}\nshort jti\n", "é".repeat(MAX_SNIPPET_BYTES));
        store
            .write_document(app, &code("min.js", "javascript", false, &long))
            .unwrap();
        store.set_indexed_at(30).unwrap();
        let matches = store.grep(&query("jti")).unwrap();
        let cut = &matches[0];
        assert!(cut.truncated);
        // An exact prefix of the line, ending on a character boundary, with
        // no marker.
        assert_eq!(cut.text.len(), MAX_SNIPPET_BYTES - 1);
        assert!(long.starts_with(&cut.text));
        assert_eq!(matches[1].text, "short jti");
        assert!(!matches[1].truncated);
    }

    #[test]
    fn json_has_the_documented_shape() {
        let f = fixture();
        let code = serde_json::to_value(&f.grep(&query("jti\\.len"))[0]).unwrap();
        let keys: Vec<&String> = code.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "citation",
                "dirty",
                "indexed_at",
                "kind",
                "license_class",
                "role",
                "text",
                "truncated"
            ]
        );
        let spec = GrepQuery {
            only: Some(Scope::Spec),
            ..query("jti")
        };
        let spec = serde_json::to_value(&f.grep(&spec)[0]).unwrap();
        assert!(spec.get("dirty").is_none(), "{spec}");
        let results = serde_json::to_value(GrepResults { matches: vec![] }).unwrap();
        assert_eq!(results, serde_json::json!({"matches": []}));
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
}
