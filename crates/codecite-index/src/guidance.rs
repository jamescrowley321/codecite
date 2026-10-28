//! Guidance documents: a team's rules, written in Markdown, split into
//! numbered sections as a spec is, each with the scope its own comment
//! gives it.

use std::collections::HashSet;

use codecite_core::{RuleScope, SectionNumber, is_invisible_format, requirement_keywords};
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

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

    /// The document numbers its rules, but a heading without a number holds
    /// a requirement keyword, so its rule would be lost.
    #[error(
        "the heading {title:?} on line {line} holds a requirement keyword but no number; number it like the other rules"
    )]
    UnnumberedRule {
        /// The heading's text.
        title: String,
        /// Its line, counting from 1.
        line: u32,
    },

    /// Two rules have one number, so a citation of it would be a guess.
    #[error(
        "rule {number} on line {line} repeats the number of the rule on line {first}; renumber one"
    )]
    DuplicateRule {
        /// The number both write.
        number: String,
        /// The later heading's line.
        line: u32,
        /// The first heading's line.
        first: u32,
    },

    /// The document holds a character that renders as nothing, which could
    /// hide text from a person reviewing the rules but not from an agent.
    #[error("line {line} holds the invisible character U+{codepoint:04X}; remove it")]
    InvisibleCharacter {
        /// Its line, counting from 1.
        line: u32,
        /// The character's code point.
        codepoint: u32,
    },

    /// The document holds text that doesn't render, outside fenced code
    /// and code spans: an HTML comment other than a well-formed scope
    /// comment, raw HTML, or a link reference definition that hides text.
    /// A person reviewing the rendered rules wouldn't see it, but agents
    /// would.
    #[error("line {line} holds {what}, which doesn't render; write rules in plain Markdown")]
    HiddenText {
        /// Its line, counting from 1.
        line: u32,
        /// What was found.
        what: &'static str,
    },

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
/// before the next heading. A first heading that is the only one at the
/// shallowest level, with every other heading below it, is the document's
/// title and never a rule. When more than half of the other headings start
/// with a section number (`## 3.1 Read settings once`, or `3.1.`), only the
/// numbered headings are rules, numbered as written, and a number written
/// twice keeps its first; an unnumbered heading whose text holds a
/// requirement keyword is then [`GuidanceError::UnnumberedRule`], so no
/// rule is lost unseen. Otherwise every heading is a rule, numbered by its
/// place in the outline (1, 1.1, 1.2, 2, …). A rule's parent is the rule
/// whose number it extends by one part. Its anchor is the heading's
/// GitHub-style slug.
pub fn split_guidance(markdown: &str) -> Result<Vec<GuidanceRule>, GuidanceError> {
    let checked = check_hidden(markdown)?;
    let headings = markdown_headings(markdown);
    if headings.is_empty() {
        return Err(GuidanceError::NoRules);
    }
    // The splitter and the parser must agree on every heading, so no rule
    // boundary is one where the other sees none.
    let split_at: Vec<u32> = headings.iter().map(|h| h.line).collect();
    if split_at != checked.headings {
        let line = split_at
            .iter()
            .zip(&checked.headings)
            .find(|(a, b)| a != b)
            .map_or_else(
                || *split_at.iter().chain(&checked.headings).max().unwrap_or(&1),
                |(a, b)| *a.min(b),
            );
        return Err(GuidanceError::HiddenText {
            line,
            what: "a heading the rule splitter and the Markdown parser read differently",
        });
    }
    let comments = checked.comments;
    let mut used = vec![false; comments.len()];
    let lines: Vec<&str> = markdown.split('\n').collect();
    let (numbered, written) = numbered(&headings);
    let mut slugs = Slugs::default();
    let mut rules: Vec<GuidanceRule> = Vec::new();
    for (i, (heading, number)) in headings.iter().zip(numbered).enumerate() {
        let anchor = slugs.slug(&heading.title);
        let text = span(&lines, heading.line, heading.end_line);
        let Some((number, title)) = number else {
            if written
                && !(i == 0 && is_title(&headings))
                && !requirement_keywords(&text).is_empty()
            {
                return Err(GuidanceError::UnnumberedRule {
                    title: heading.title.clone(),
                    line: heading.line,
                });
            }
            continue;
        };
        let parent_number = number.rsplit_once('.').map(|(p, _)| p);
        let parent = parent_number.and_then(|p| rules.iter().rposition(|r| r.section.number == p));
        if let Some(first) = rules.iter().find(|r| r.section.number == number) {
            if requirement_keywords(&text).is_empty() {
                continue;
            }
            return Err(GuidanceError::DuplicateRule {
                number,
                line: heading.line,
                first: first.section.start_line,
            });
        }
        if let Some(i) = comments
            .iter()
            .position(|c| (heading.line..=heading.end_line).contains(&c.line))
        {
            used[i] = true;
        }
        let scope = scope(&comments, heading.line, heading.end_line).map_err(|message| {
            GuidanceError::Scope {
                number: number.clone(),
                message,
            }
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
    // A scope comment under the title, an unnumbered heading or no
    // heading would scope nothing, so it could only hide text.
    if let Some(i) = used.iter().position(|u| !u) {
        return Err(GuidanceError::HiddenText {
            line: comments[i].line,
            what: "a scope comment outside any rule",
        });
    }
    Ok(rules)
}

/// Whether the first heading is the document's title: the only one at the
/// shallowest level, with every other heading below it.
fn is_title(headings: &[Heading]) -> bool {
    let shallowest = headings.iter().map(|h| h.level).min().unwrap_or(1);
    headings.len() > 1
        && headings[0].level == shallowest
        && headings[1..].iter().all(|h| h.level > shallowest)
}

/// Each heading's number and title when it is a rule, `None` when it
/// isn't, and whether the numbers are the ones the document writes.
fn numbered(headings: &[Heading]) -> (Vec<Option<(String, String)>>, bool) {
    let is_title = is_title(headings);
    let skip = usize::from(is_title);
    let mut written: Vec<Option<(String, String)>> =
        headings.iter().map(|h| written_number(&h.title)).collect();
    let numbered = written[skip..].iter().filter(|w| w.is_some()).count();
    if numbered * 2 > headings.len() - skip {
        if is_title {
            written[0] = None;
        }
        return (written, true);
    }
    let shallowest = headings.iter().map(|h| h.level).min().unwrap_or(1);
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
    let rules = headings
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
        .collect();
    (rules, false)
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

/// A scope comment that [`check_hidden`] found: its first line and the
/// TOML between `codecite-scope` and `-->`.
struct ScopeComment<'a> {
    line: u32,
    body: &'a str,
}

/// The scope a rule gives in the scope comment among `comments` that
/// starts between its lines `start` and `end`, or `None` when it has none.
fn scope(comments: &[ScopeComment<'_>], start: u32, end: u32) -> Result<Option<RuleScope>, String> {
    let Some(comment) = comments.iter().find(|c| (start..=end).contains(&c.line)) else {
        return Ok(None);
    };
    let scope: RuleScope =
        toml::from_str(comment.body).map_err(|e| e.to_string().trim_end().replace('\n', " "))?;
    Ok(Some(scope))
}

/// Refuses text in `markdown` that renders as nothing, since a person
/// reviewing the rendered rules wouldn't see it but agents would, and
/// returns the scope comments and the headings. Markdown is read by a
/// CommonMark parser, and only what renders as visible text is allowed:
/// paragraphs, `#` headings at the top level, quotes, lists, emphasis,
/// code spans and blocks, rules, and links with no title to a plain URL.
/// Everything else is refused, an invisible character anywhere included:
/// raw HTML (comments, tags, processing instructions, declarations,
/// CDATA), images, setext headings and headings inside quotes or lists,
/// and link reference definitions. The one HTML allowed is a top-level
/// block that is only a scope comment, `<!-- codecite-scope` then TOML
/// with no `#` comment then `-->`, at most one under each heading.
fn check_hidden(markdown: &str) -> Result<Checked<'_>, GuidanceError> {
    let line_of = |offset: usize| markdown[..offset].matches('\n').count() as u32 + 1;
    let hidden = |offset, what| {
        Err(GuidanceError::HiddenText {
            line: line_of(offset),
            what,
        })
    };
    for (n, line) in (1..).zip(markdown.split('\n')) {
        if let Some(c) = line.chars().find(|&c| is_invisible_format(c)) {
            return Err(GuidanceError::InvisibleCharacter {
                line: n,
                codepoint: c.into(),
            });
        }
    }
    let mut checked = Checked {
        comments: Vec::new(),
        headings: Vec::new(),
    };
    let mut under_heading = 0;
    // How many blocks deep the parser is inside quotes and lists.
    let mut depth = 0usize;
    // Inside a scope comment's block, whose lines come as `Html` events.
    let mut in_scope_block = false;
    let mut events = Parser::new_ext(markdown, Options::empty()).into_offset_iter();
    for (event, range) in events.by_ref() {
        // Every kind of event is named, so a kind a newer parser adds is
        // refused until someone decides it renders.
        match event {
            Event::Start(Tag::Heading { .. }) => {
                if depth > 0 || !markdown[range.clone()].trim_start().starts_with('#') {
                    return hidden(
                        range.start,
                        "a setext heading, or a heading inside a quote or list (write `## ` headings at the top level)",
                    );
                }
                under_heading = 0;
                checked.headings.push(line_of(range.start));
            }
            Event::Start(Tag::HtmlBlock) => {
                let block = markdown[range.clone()].trim();
                let Some(body) = scope_body(block).filter(|_| depth == 0) else {
                    let what = if block.starts_with("<!--") {
                        "an HTML comment"
                    } else {
                        "raw HTML"
                    };
                    return hidden(range.start, what);
                };
                under_heading += 1;
                if under_heading > 1 {
                    return hidden(range.start, "a second scope comment under one heading");
                }
                checked.comments.push(ScopeComment {
                    line: line_of(range.start),
                    body,
                });
                in_scope_block = true;
            }
            Event::End(TagEnd::HtmlBlock) => in_scope_block = false,
            Event::Html(_) if in_scope_block => {}
            Event::Start(Tag::BlockQuote(_) | Tag::List(_) | Tag::Item) => depth += 1,
            Event::End(TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item) => depth -= 1,
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                let shown = matches!(
                    link_type,
                    LinkType::Inline | LinkType::Autolink | LinkType::Email
                );
                if !shown || !title.is_empty() || !is_plain_url(&dest_url) {
                    return hidden(
                        range.start,
                        "a link with a title or a destination that isn't a plain URL",
                    );
                }
            }
            Event::Start(Tag::Image { .. }) => {
                return hidden(
                    range.start,
                    "an image, whose alt text and title don't render",
                );
            }
            Event::InlineHtml(html) | Event::Html(html) => {
                let what = if html.starts_with("<!--") {
                    "an HTML comment"
                } else {
                    "raw HTML"
                };
                return hidden(range.start, what);
            }
            Event::Start(Tag::Paragraph | Tag::CodeBlock(_) | Tag::Emphasis | Tag::Strong)
            | Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::CodeBlock
                | TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Link,
            )
            | Event::Text(_)
            | Event::Code(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::Rule => {}
            // Extensions codecite doesn't turn on, so these never come; if
            // one did, it would be refused.
            Event::Start(
                Tag::FootnoteDefinition(_)
                | Tag::DefinitionList
                | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::TableCell
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::MetadataBlock(_),
            )
            | Event::End(
                TagEnd::Image
                | TagEnd::FootnoteDefinition
                | TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::TableCell
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::MetadataBlock(_),
            )
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_) => {
                return hidden(
                    range.start,
                    "Markdown that codecite doesn't read in guidance",
                );
            }
        }
    }
    if let Some(start) = events
        .reference_definitions()
        .iter()
        .map(|(_, def)| def.span.start)
        .min()
    {
        return hidden(start, "a link reference definition (write the link inline)");
    }
    Ok(checked)
}

/// What [`check_hidden`] found: the scope comments, and the line of each
/// heading as the parser reads them.
struct Checked<'a> {
    comments: Vec<ScopeComment<'a>>,
    headings: Vec<u32>,
}

/// Whether `url` is a plain URL a reader sees on hover: no spaces, quotes,
/// angle brackets or other characters a URL would escape.
fn is_plain_url(url: &str) -> bool {
    !url.is_empty()
        && url
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c))
}

/// The TOML of an HTML block that is only a scope comment, or `None` when
/// the block is anything else: `codecite-scope` must end at whitespace or
/// `-->`, the block must end at its one `-->`, and the TOML may hold no
/// comment.
fn scope_body(block: &str) -> Option<&str> {
    let rest = block.strip_prefix(SCOPE_OPEN)?;
    if !(rest.starts_with(char::is_whitespace) || rest.starts_with("-->")) {
        return None;
    }
    let body = rest.strip_suffix("-->")?;
    (!body.contains("-->") && !body.contains("<!--") && !has_toml_comment(body)).then_some(body)
}

/// Whether TOML `text` holds a `#` comment: a `#` outside a string.
fn has_toml_comment(text: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' => return true,
            None => {}
        }
    }
    false
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
        // An unclosed one runs to the end, so it hides everything after it.
        let unclosed = "## 4 Rule\n<!-- codecite-scope\npaths = []\n";
        assert_eq!(
            split_guidance(unclosed),
            Err(GuidanceError::HiddenText {
                line: 2,
                what: "an HTML comment"
            })
        );
    }

    #[test]
    fn a_scope_comment_in_a_fence_or_mid_line_is_not_a_scope() {
        let text = "## 1 Writing scopes\n\n```markdown\n<!-- codecite-scope paths = [\"a/[\"] -->\n```\n\n\
                    Say `<!-- codecite-scope languages = [\"go\"] -->` above a rule. You MAY.\n";
        let rules = split_guidance(text).unwrap();
        assert_eq!(rules[0].scope, None);
        // `codecite-scopes` is not the scope comment, so it is a hidden one.
        let text = "## 1 Rule\n<!-- codecite-scopes paths = [] -->\nMUST.\n";
        assert_eq!(
            split_guidance(text),
            Err(GuidanceError::HiddenText {
                line: 2,
                what: "an HTML comment"
            })
        );
        // After the fence closes, a comment on its own line counts.
        let text =
            "## 1 Rule\n```\nx\n```\n  <!-- codecite-scope languages = [\"go\"] -->\nMUST.\n";
        assert_eq!(
            split_guidance(text).unwrap()[0]
                .scope
                .as_ref()
                .unwrap()
                .languages,
            ["go"]
        );
    }

    #[test]
    fn one_numbered_heading_does_not_drop_the_others() {
        // A numbered title and a heading that starts with a number are a
        // minority: the outline numbers every rule.
        let text = "# 2024 Engineering Standards\n\n## Settings\nMUST.\n## 2 factor auth\nMUST.\n## Logging\nSHOULD.\n";
        let rules = split_guidance(text).unwrap();
        let got: Vec<(&str, &str)> = rules
            .iter()
            .map(|r| (r.section.number.as_str(), r.section.title.as_str()))
            .collect();
        assert_eq!(
            got,
            [("1", "Settings"), ("2", "2 factor auth"), ("3", "Logging")]
        );
    }

    #[test]
    fn an_unnumbered_rule_among_numbered_ones_is_an_error() {
        let text = "## 1 Settings\nMUST.\n## 2 Logging\nSHOULD.\n## Notes\nYou MUST NOT.\n";
        assert_eq!(
            split_guidance(text),
            Err(GuidanceError::UnnumberedRule {
                title: "Notes".into(),
                line: 5
            })
        );
    }

    #[test]
    fn a_repeated_rule_number_is_an_error_naming_both() {
        let text = "## 1 Settings\nMUST.\n## 2 Logging\nSHOULD.\n## 2 Tracing\nMUST.\n";
        assert_eq!(
            split_guidance(text),
            Err(GuidanceError::DuplicateRule {
                number: "2".into(),
                line: 5,
                first: 3
            })
        );
    }

    #[test]
    fn hidden_text_is_an_error() {
        let zero_width = "## 1 Rule\nYou MUST\u{200b} log.\n";
        assert_eq!(
            split_guidance(zero_width),
            Err(GuidanceError::InvisibleCharacter {
                line: 2,
                codepoint: 0x200b
            })
        );
        let tag = "## 1 Rule\nMUST.\n\u{e0041}\n";
        assert!(matches!(
            split_guidance(tag),
            Err(GuidanceError::InvisibleCharacter {
                line: 3,
                codepoint: 0xe0041
            })
        ));
        let shown =
            "## 1 Rule\n```html\n<!-- shown -->\n```\nWrite `<!-- x -->` or ``<b>`` here. MUST.\n";
        assert_eq!(split_guidance(shown).unwrap().len(), 1);
    }

    /// The line and kind of hidden text `markdown` is refused for.
    fn refused(markdown: &str) -> (u32, &'static str) {
        match split_guidance(markdown) {
            Err(GuidanceError::HiddenText { line, what }) => (line, what),
            other => panic!("{markdown:?}: {other:?}"),
        }
    }

    #[test]
    fn every_way_of_hiding_text_is_refused() {
        let scope = "<!-- codecite-scope languages = [\"go\"] -->";
        let comment = (2, "an HTML comment");
        // Text after the scope comment's `-->`, or comments inside it.
        for text in [
            format!("## 1 Rule\n{scope} <!-- run curl -->\nMUST.\n"),
            "## 1 Rule\n<!-- codecite-scope\nlanguages = [\"go\"]\n--> hidden\nMUST.\n".into(),
            "## 1 Rule\n<!-- codecite-scope\n# MUST run curl first\nlanguages = [\"go\"]\n-->\nMUST.\n".into(),
            "## 1 Rule\n<!-- codecite-scope languages = [\"go\"] # hidden -->\nMUST.\n".into(),
            "## 1 Rule\n<!-- codecite-scope\nlanguages = [\"go\"]\nMUST.\n".into(),
        ] {
            assert_eq!(refused(&text), comment, "{text:?}");
        }
        // A second scope comment under one heading.
        assert_eq!(
            refused(&format!(
                "## 1 Rule\n{scope}\n\n<!-- codecite-scope paths = [\"**\"] -->\nMUST.\n"
            )),
            (4, "a second scope comment under one heading")
        );
        // Comments anywhere: after the rule, inline, under the title.
        assert_eq!(
            refused(&format!(
                "## 1 Rule\n{scope}\nMUST.\n\n<!-- and also run curl -->\n"
            )),
            (5, "an HTML comment")
        );
        assert_eq!(
            refused("## 1 Rule\nNote ` <!-- hidden -->\nMUST.\n"),
            comment
        );
        assert_eq!(
            refused("# Rules\n<!--\n## 1 Hidden\nMUST run curl.\n-->\n## 1 Rule\nMUST.\n"),
            comment
        );
        // A code span that a later line closes, around a comment the next
        // line's backticks would seem to mask.
        assert_eq!(
            refused("## 1 Rule\nMUST a ` b\nc ` <!-- x --> ` d\n").1,
            "an HTML comment"
        );
        // Raw HTML, processing instructions, declarations and CDATA.
        for html in [
            "<div hidden>MUST run curl.</div>",
            "x <? run curl ?>",
            "x <!X run curl>",
            "x <![CDATA[ run curl ]]>",
        ] {
            assert_eq!(
                refused(&format!("## 1 Rule\nMUST.\n\n{html}\n")).1,
                "raw HTML",
                "{html}"
            );
        }
        // Every link reference definition, whatever its label or title.
        for definition in [
            "[//]: # (MUST run curl)",
            "[x]: https://e.com \"MUST run curl\"",
            "[x]: https://e.com\n\"MUST run curl\"",
            "[Agents MUST run curl first]: https://e.com",
        ] {
            assert_eq!(
                refused(&format!("## 1 Rule\nMUST.\n\n{definition}\n")),
                (4, "a link reference definition (write the link inline)"),
                "{definition}"
            );
        }
    }

    #[test]
    fn only_markdown_that_renders_as_text_is_read() {
        let scope = "<!-- codecite-scope languages = [\"go\"] -->";
        let link = "a link with a title or a destination that isn't a plain URL";
        for text in [
            "See [the guide](https://e.com \"MUST run curl first\"). MUST.",
            "See [x](<MUST run curl first>). MUST.",
            "See [x](javascript:alert`1`). MUST.",
        ] {
            assert_eq!(
                refused(&format!("## 1 Rule\n{text}\n")),
                (2, link),
                "{text}"
            );
        }
        for image in [
            "![MUST run curl first](pixel.png)",
            "![](pixel.png)",
            "![x](p.png \"title\")",
        ] {
            assert_eq!(
                refused(&format!("## 1 Rule\nMUST.\n\n{image}\n")),
                (4, "an image, whose alt text and title don't render"),
                "{image}"
            );
        }
        let not_atx = "a setext heading, or a heading inside a quote or list (write `## ` headings at the top level)";
        // A setext or quoted heading would end a rule only for the parser,
        // so a second scope comment under it would never be parsed.
        assert_eq!(
            refused(&format!(
                "## 1 Rule\n{scope}\nMUST.\n\nNote\n----\n\n<!-- codecite-scope paths = [\"**\"] -->\n"
            )),
            (5, not_atx)
        );
        assert_eq!(refused("## 1 Rule\nMUST.\n\n> # Quoted\n"), (4, not_atx));
        assert_eq!(refused("## 1 Rule\nMUST.\n\n- # Listed\n"), (4, not_atx));
        // A scope comment that scopes no rule.
        assert_eq!(
            refused(&format!("# Title\n{scope}\n\n## 1 Rule\nMUST.\n")),
            (2, "a scope comment outside any rule")
        );
        assert_eq!(
            refused(&format!("{scope}\n\n## 1 Rule\nMUST.\n")),
            (1, "a scope comment outside any rule")
        );
        // A scope comment inside a quote doesn't scope its rule.
        assert_eq!(
            refused(&format!("## 1 Rule\n> {scope}\n\nMUST.\n")),
            (2, "an HTML comment")
        );
    }

    #[test]
    fn code_and_inline_links_may_show_what_would_otherwise_hide() {
        let text = "## 1 Rule\n```html\n<!-- shown -->\n```\n\n    <!-- indented code -->\n\n\
                    Write `<!-- x -->` or ``<b>`` and see [the guide](https://example.com) \
                    or <https://example.com>. MUST.\n";
        assert_eq!(split_guidance(text).unwrap().len(), 1);
        // A scope comment in a fence is shown, not a scope.
        let fenced = "## 1 Rule\n```\n<!-- codecite-scope paths = [\"a/[\"] -->\n```\nMUST.\n";
        assert_eq!(split_guidance(fenced).unwrap()[0].scope, None);
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
