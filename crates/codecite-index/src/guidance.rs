//! Guidance documents: a team's rules, written in Markdown, split into
//! numbered sections as a spec is, each with the scope its own comment
//! gives it.

use std::collections::HashSet;

use codecite_core::{RuleScope, SectionNumber, requirement_keywords};

use crate::SpecSection;
use crate::chunk::{Heading, markdown_headings};

/// The comment that opens a rule's scope: `<!-- codecite-scope`, then TOML
/// up to `-->`.
const SCOPE_OPEN: &str = "<!-- codecite-scope";

/// One rule of a guidance document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceRule {
    /// The rule as a spec section: its number, title, anchor, parent,
    /// lines, text and requirement keywords. It has no steps.
    pub section: SpecSection,
    /// The scope its `<!-- codecite-scope ... -->` comment gives, if any.
    pub scope: Option<RuleScope>,
}

/// Why a guidance document could not be split into rules.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GuidanceError {
    /// The document has no Markdown heading, so no rule.
    #[error("no Markdown headings found; each rule is a heading and the text under it")]
    NoRules,

    /// A rule's scope comment is not TOML with `paths`, `languages` and
    /// `imports`, or is never closed.
    #[error("rule {number}: bad codecite-scope comment: {message}")]
    Scope {
        /// The rule's number.
        number: String,
        /// What was wrong.
        message: String,
    },
}

/// Splits a Markdown guidance document into its rules.
///
/// Each ATX heading outside fenced code is a section, running to the line
/// before the next heading. When some heading starts with a section number
/// (`## 3.1 Read settings once`, or `3.1.`), only the numbered headings are
/// rules, numbered as written, and a number written twice keeps its first.
/// Otherwise every heading is a rule, numbered by its place in the outline
/// (1, 1.1, 1.2, 2, …); a first heading that is the only one at the
/// shallowest level, with every other heading below it, is the document's
/// title and not a rule. A rule's parent is the rule whose number it
/// extends by one part. Its anchor is the heading's GitHub-style slug.
pub fn split_guidance(markdown: &str) -> Result<Vec<GuidanceRule>, GuidanceError> {
    let headings = markdown_headings(markdown);
    if headings.is_empty() {
        return Err(GuidanceError::NoRules);
    }
    let lines: Vec<&str> = markdown.split('\n').collect();
    let numbered = numbered(&headings);
    let mut slugs = Slugs::default();
    let mut rules: Vec<GuidanceRule> = Vec::new();
    for (heading, number) in headings.iter().zip(numbered) {
        let anchor = slugs.slug(&heading.title);
        let Some((number, title)) = number else {
            continue;
        };
        let text = span(&lines, heading.line, heading.end_line);
        let parent_number = number.rsplit_once('.').map(|(p, _)| p);
        let parent = parent_number.and_then(|p| rules.iter().rposition(|r| r.section.number == p));
        let scope = scope(&text).map_err(|message| GuidanceError::Scope {
            number: number.clone(),
            message,
        })?;
        rules.push(GuidanceRule {
            section: SpecSection {
                strength: requirement_keywords(&text),
                number,
                title,
                anchor,
                parent,
                start_line: heading.line,
                end_line: heading.end_line,
                text,
                steps: Vec::new(),
            },
            scope,
        });
    }
    Ok(rules)
}

/// Each heading's number and title when it is a rule, `None` when it isn't.
fn numbered(headings: &[Heading]) -> Vec<Option<(String, String)>> {
    let written: Vec<Option<(String, String)>> =
        headings.iter().map(|h| written_number(&h.title)).collect();
    if written.iter().any(Option::is_some) {
        let mut seen = HashSet::new();
        return written
            .into_iter()
            .map(|w| w.filter(|(number, _)| seen.insert(number.clone())))
            .collect();
    }
    let shallowest = headings.iter().map(|h| h.level).min().unwrap_or(1);
    let is_title = headings.len() > 1
        && headings[0].level == shallowest
        && headings[1..].iter().all(|h| h.level > shallowest);
    let base = if is_title {
        headings[1..]
            .iter()
            .map(|h| h.level)
            .min()
            .unwrap_or(shallowest)
    } else {
        shallowest
    };
    let mut counters: Vec<u32> = Vec::new();
    headings
        .iter()
        .enumerate()
        .map(|(i, h)| {
            if is_title && i == 0 {
                return None;
            }
            // A heading that skips a level nests one level deeper, no more.
            let depth = (h.level - base).min(counters.len());
            counters.truncate(depth + 1);
            match counters.get_mut(depth) {
                Some(n) => *n += 1,
                None => counters.push(1),
            }
            let number: Vec<String> = counters.iter().map(u32::to_string).collect();
            Some((number.join("."), h.title.clone()))
        })
        .collect()
}

/// The section number a heading starts with, in its canonical form, and
/// the title after it: `("3.1", "Read settings once")` for `3.1. Read
/// settings once`. `None` without one, or with a number written another
/// way (`03`, `3..1`).
fn written_number(title: &str) -> Option<(String, String)> {
    let (first, rest) = title.split_once(char::is_whitespace)?;
    let number = first.strip_suffix('.').unwrap_or(first);
    let parsed: SectionNumber = number.parse().ok()?;
    let rest = rest.trim();
    (parsed.to_string() == number && !rest.is_empty()).then(|| (number.to_owned(), rest.to_owned()))
}

/// Lines `start..=end` (from 1) of `lines`, joined with `\n`, without a
/// `\r` at a line's end.
fn span(lines: &[&str], start: u32, end: u32) -> String {
    let (from, to) = (start as usize - 1, (end as usize).min(lines.len()));
    lines[from..to]
        .iter()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The scope a rule's text gives in its first `<!-- codecite-scope ... -->`
/// comment, or `None` when it has none. The comment's body is TOML.
fn scope(text: &str) -> Result<Option<RuleScope>, String> {
    let Some(start) = text.find(SCOPE_OPEN) else {
        return Ok(None);
    };
    let body = &text[start + SCOPE_OPEN.len()..];
    let end = body
        .find("-->")
        .ok_or_else(|| "the comment has no closing `-->`".to_owned())?;
    let scope: RuleScope =
        toml::from_str(&body[..end]).map_err(|e| e.to_string().trim_end().replace('\n', " "))?;
    Ok(Some(scope))
}

/// GitHub-style heading anchors, with `-1`, `-2`, … on a repeated one.
#[derive(Default)]
struct Slugs {
    seen: HashSet<String>,
}

impl Slugs {
    /// The anchor for a heading with `title`: lowercase, letters, digits,
    /// `-` and `_` kept, spaces as `-`, anything else dropped.
    fn slug(&mut self, title: &str) -> String {
        let base: String = title
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .flat_map(char::to_lowercase)
            .collect();
        let mut slug = base.clone();
        let mut n = 0;
        while !self.seen.insert(slug.clone()) {
            n += 1;
            slug = format!("{base}-{n}");
        }
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecite_core::Keyword;

    /// A rule as (number, title, anchor, parent's number, lines).
    type Row = (String, String, String, Option<String>, (u32, u32));

    /// Each rule as a [`Row`].
    fn outline(rules: &[GuidanceRule]) -> Vec<Row> {
        rules
            .iter()
            .map(|r| {
                let s = &r.section;
                (
                    s.number.clone(),
                    s.title.clone(),
                    s.anchor.clone(),
                    s.parent.map(|p| rules[p].section.number.clone()),
                    (s.start_line, s.end_line),
                )
            })
            .collect()
    }

    fn row(
        number: &str,
        title: &str,
        anchor: &str,
        parent: Option<&str>,
        lines: (u32, u32),
    ) -> Row {
        (
            number.into(),
            title.into(),
            anchor.into(),
            parent.map(Into::into),
            lines,
        )
    }

    #[test]
    fn written_numbers_name_the_rules() {
        let text = "# House rules\n\nIntro.\n\n## 1. Settings\n\nCode MUST read settings through `get_settings`.\n\n\
                    ### 1.1 No os.environ\n\nHandlers MUST NOT read `os.environ`.\n\n## Background\n\nWhy.\n\n\
                    ## 2 Logging\n\nHandlers SHOULD use `audit_log`.\n\n## 2. Again\n\nDropped.\n";
        let rules = split_guidance(text).unwrap();
        assert_eq!(
            outline(&rules),
            [
                row("1", "Settings", "1-settings", None, (5, 7)),
                row(
                    "1.1",
                    "No os.environ",
                    "11-no-osenviron",
                    Some("1"),
                    (9, 11)
                ),
                row("2", "Logging", "2-logging", None, (17, 19)),
            ]
        );
        assert_eq!(rules[0].section.strength, [Keyword::Must]);
        assert_eq!(rules[1].section.strength, [Keyword::MustNot]);
        assert_eq!(rules[2].section.strength, [Keyword::Should]);
        assert_eq!(
            rules[1].section.text,
            "### 1.1 No os.environ\n\nHandlers MUST NOT read `os.environ`."
        );
        assert!(
            rules
                .iter()
                .all(|r| r.scope.is_none() && r.section.steps.is_empty())
        );
    }

    #[test]
    fn unnumbered_headings_are_numbered_by_the_outline_under_a_title() {
        let text = "# Conventions\n\n## Settings\n\nMUST.\n\n### Environment\n\nMUST NOT.\n\n#### Deeper\n\n\
                    ### Files\n\n## Logging\n\n#### Skipped a level\n\nSHOULD.\n\n## Settings\n";
        let rules = split_guidance(text).unwrap();
        let got: Vec<(String, Option<String>)> = outline(&rules)
            .into_iter()
            .map(|(n, _, _, p, _)| (n, p))
            .collect();
        let want = [
            ("1", None),
            ("1.1", Some("1")),
            ("1.1.1", Some("1.1")),
            ("1.2", Some("1")),
            ("2", None),
            ("2.1", Some("2")),
            ("3", None),
        ];
        let want: Vec<(String, Option<String>)> = want
            .iter()
            .map(|(n, p)| (n.to_string(), p.map(Into::into)))
            .collect();
        assert_eq!(got, want);
        // A repeated heading gets its own anchor.
        assert_eq!(rules[0].section.anchor, "settings");
        assert_eq!(rules[6].section.anchor, "settings-1");
    }

    #[test]
    fn without_a_title_every_heading_is_a_rule() {
        let rules = split_guidance("# One\nMUST.\n# Two\n## Two point one\n").unwrap();
        let numbers: Vec<&str> = rules.iter().map(|r| r.section.number.as_str()).collect();
        assert_eq!(numbers, ["1", "2", "2.1"]);
        // A lone heading is a rule, not a title.
        assert_eq!(
            split_guidance("# Only\nMUST.\n").unwrap()[0].section.number,
            "1"
        );
    }

    #[test]
    fn written_numbers_must_be_canonical() {
        assert_eq!(
            written_number("3.1 Title"),
            Some(("3.1".into(), "Title".into()))
        );
        assert_eq!(
            written_number("3. Title"),
            Some(("3".into(), "Title".into()))
        );
        assert_eq!(written_number("03 Title"), None);
        assert_eq!(written_number("3..1 Title"), None);
        assert_eq!(
            written_number("2026 roadmap"),
            Some(("2026".into(), "roadmap".into()))
        );
        assert_eq!(written_number("3.1"), None);
        assert_eq!(written_number("Title 3"), None);
    }

    #[test]
    fn a_scope_comment_scopes_its_rule() {
        let text = "## 1 Outbound calls\n\n<!-- codecite-scope\npaths = [\"app/**/*.py\"]\nimports = [\"requests\", \"httpx\"]\n-->\n\n\
                    Code MUST call partners through `partner_client`.\n\n## 2 Other\n\n\
                    <!-- codecite-scope languages = [\"python\"] -->\nMAY.\n";
        let rules = split_guidance(text).unwrap();
        let scope = rules[0].scope.as_ref().unwrap();
        assert_eq!(scope.paths, ["app/**/*.py"]);
        assert_eq!(scope.imports, ["requests", "httpx"]);
        assert!(scope.languages.is_empty());
        assert_eq!(rules[1].scope.as_ref().unwrap().languages, ["python"]);
    }

    #[test]
    fn a_bad_scope_comment_is_an_error_naming_the_rule() {
        let unknown = "## 4 Rule\n<!-- codecite-scope\nglobs = [\"x\"]\n-->\n";
        let err = split_guidance(unknown).unwrap_err();
        assert!(
            matches!(&err, GuidanceError::Scope { number, message } if number == "4" && message.contains("unknown field `globs`")),
            "{err}"
        );
        let unclosed = "## 4 Rule\n<!-- codecite-scope\npaths = []\n";
        assert!(
            split_guidance(unclosed)
                .unwrap_err()
                .to_string()
                .contains("no closing `-->`")
        );
    }

    #[test]
    fn headings_in_code_fences_are_not_rules() {
        let text = "## 1 Rule\n\n```md\n## 2 Not a rule\n```\nMUST.\n";
        let rules = split_guidance(text).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].section.end_line, 6);
    }

    #[test]
    fn a_document_without_headings_has_no_rules() {
        assert_eq!(split_guidance("Just text.\n"), Err(GuidanceError::NoRules));
    }

    #[test]
    fn crlf_line_endings_are_dropped() {
        let rules = split_guidance("## 1 Rule\r\nYou MUST.\r\n").unwrap();
        assert_eq!(rules[0].section.text, "## 1 Rule\nYou MUST.");
    }
}
