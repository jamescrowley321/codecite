//! Line bookkeeping, 60-line windows, Markdown and HTML heading sections
//! and their anchors.

use std::collections::HashSet;
use std::sync::LazyLock;

use scraper::{Html, Selector};

use super::{Chunk, ChunkKind, WINDOW_LINES};

/// A file's text and the byte offset where each line starts. Every offset
/// sits just after a `\n`, so slicing at them never splits a character.
pub(super) struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        // A final newline ends the last line; it doesn't start an empty one.
        if starts.last() == Some(&text.len()) {
            starts.pop();
        }
        Self { text, starts }
    }

    /// The whole text.
    pub(super) fn text(&self) -> &'a str {
        self.text
    }

    /// The number of lines.
    pub(super) fn len(&self) -> usize {
        self.starts.len()
    }

    /// Line `n` (from 1), without its line ending.
    pub(super) fn line(&self, n: u32) -> &'a str {
        self.span(n, n)
    }

    /// Lines `start..=end` (from 1), without the last line's ending.
    pub(super) fn span(&self, start: u32, end: u32) -> &'a str {
        let from = self.starts[start as usize - 1];
        let to = self
            .starts
            .get(end as usize)
            .copied()
            .unwrap_or(self.text.len());
        let span = &self.text[from..to];
        let span = span.strip_suffix('\n').unwrap_or(span);
        span.strip_suffix('\r').unwrap_or(span)
    }

    fn is_blank(&self, n: u32) -> bool {
        self.line(n).trim().is_empty()
    }

    /// True when line `n` has no letter or digit: blank, or only braces
    /// and punctuation.
    fn has_no_words(&self, n: u32) -> bool {
        !self.line(n).chars().any(char::is_alphanumeric)
    }
}

/// Windows over every run of lines that `covered` marks false, cut every
/// [`WINDOW_LINES`] lines. A window never starts or ends on a line without
/// a letter or digit, so a lone `}` between definitions makes no window.
pub(super) fn windows<'a>(lines: &Lines<'a>, covered: &[bool]) -> Vec<Chunk<'a>> {
    let mut out = Vec::new();
    let mut n = 1;
    let last = lines.len() as u32;
    while n <= last {
        if covered[n as usize - 1] || lines.has_no_words(n) {
            n += 1;
            continue;
        }
        // `n` starts a window: take up to WINDOW_LINES uncovered lines,
        // then drop the trailing lines without words.
        let mut end = n;
        while end < last && end - n + 1 < WINDOW_LINES && !covered[end as usize] {
            end += 1;
        }
        let next = end + 1;
        while lines.has_no_words(end) {
            end -= 1;
        }
        out.push(Chunk {
            kind: ChunkKind::Window,
            start_line: n,
            end_line: end,
            text: lines.span(n, end),
            symbol: None,
            title: None,
            anchor: None,
        });
        n = next;
    }
    out
}

/// One section per ATX heading (`#` to `######`), running to the line
/// before the next heading. Headings inside fenced code blocks don't count.
/// Trailing blank lines are dropped from each section.
pub(super) fn sections<'a>(lines: &Lines<'a>) -> Vec<Chunk<'a>> {
    let headings = headings(lines);
    let mut slugs = Slugs::default();
    let mut out = Vec::new();
    for (i, heading) in headings.iter().enumerate() {
        let end = section_end(lines, &headings, i);
        out.push(Chunk {
            kind: ChunkKind::Section,
            start_line: heading.line,
            end_line: end,
            text: lines.span(heading.line, end),
            symbol: None,
            title: Some(heading.title.clone()),
            anchor: Some(slugs.slug(&heading.title)),
        });
    }
    out
}

/// One section per HTML heading tag (`<h1>` to `<h6>`), running to the
/// line before the next one, like [`sections`] for Markdown. Tags inside
/// `<pre>`, `<script>`, `<style>` or a comment don't count. The anchor is
/// the tag's `id`, or else the title's slug.
pub(super) fn html_sections<'a>(lines: &Lines<'a>) -> Vec<Chunk<'a>> {
    let found = html_headings(lines);
    let headings: Vec<Heading> = found.iter().map(|(h, _)| h.clone()).collect();
    let mut slugs = Slugs::default();
    let mut out = Vec::new();
    for (i, (heading, id)) in found.into_iter().enumerate() {
        let end = section_end(lines, &headings, i);
        let anchor = id.unwrap_or_else(|| slugs.slug(&heading.title));
        out.push(Chunk {
            kind: ChunkKind::Section,
            start_line: heading.line,
            end_line: end,
            text: lines.span(heading.line, end),
            symbol: None,
            title: Some(heading.title),
            anchor: Some(anchor),
        });
    }
    out
}

/// The most lines a heading element may span before its title is cut.
const MAX_HEADING_LINES: u32 = 20;

/// Elements whose contents are never headings, with their closing tags.
const RAW_ELEMENTS: [(&str, &str); 3] = [
    ("<pre", "</pre"),
    ("<script", "</script"),
    ("<style", "</style"),
];

/// Every heading tag outside comments and raw elements, in order, each with
/// its `id` when it has one. Scans raw lines, so a heading's line is the
/// line its tag opens on.
fn html_headings(lines: &Lines<'_>) -> Vec<(Heading, Option<String>)> {
    let mut out = Vec::new();
    // The closing marker we're waiting for: `-->` or a raw element's tag.
    let mut until: Option<&str> = None;
    for n in 1..=lines.len() as u32 {
        // ASCII lowercasing keeps byte offsets, so `lower` indexes `line`.
        let lower = lines.line(n).to_ascii_lowercase();
        let mut at = 0;
        let mut heading = None;
        while at < lower.len() {
            let rest = &lower[at..];
            if let Some(close) = until {
                match rest.find(close) {
                    Some(i) => {
                        at += i + close.len();
                        until = None;
                    }
                    None => break,
                }
                continue;
            }
            let Some(i) = rest.find('<') else { break };
            let tag = &rest[i..];
            at += i + 1;
            if tag.starts_with("<!--") {
                until = Some("-->");
                at += 3;
            } else if let Some((_, close)) = RAW_ELEMENTS.iter().find(|(open, _)| opens(tag, open))
            {
                until = Some(close);
            } else if heading.is_none()
                && let Some(level) = heading_level(tag)
            {
                heading = Some((at - 1, level));
            }
        }
        if let Some((offset, level)) = heading {
            out.push(html_heading(lines, n, offset, level));
        }
    }
    out
}

/// True when `tag` opens the element `open` (`<pre`): the name is followed
/// by `>`, whitespace or `/`, so `<pre` doesn't match `<prefix>`.
fn opens(tag: &str, open: &str) -> bool {
    tag.strip_prefix(open)
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c == '>' || c == '/' || c.is_ascii_whitespace())
}

/// The level of the heading tag `tag` opens with (`<h2 ...`), lowercased.
fn heading_level(tag: &str) -> Option<usize> {
    let level = tag.strip_prefix("<h")?.chars().next()?.to_digit(10)?;
    ((1..=6).contains(&level) && opens(tag, &tag[..3])).then_some(level as usize)
}

/// The level-`level` heading whose tag opens at byte `offset` of line `n`:
/// its title is the element's text, tags stripped and whitespace collapsed,
/// read from the lines up to its closing tag.
fn html_heading(
    lines: &Lines<'_>,
    n: u32,
    offset: usize,
    level: usize,
) -> (Heading, Option<String>) {
    let first = &lines.line(n)[offset..];
    let close = format!("</h{level}");
    let last = (n..=lines.len() as u32)
        .take(MAX_HEADING_LINES as usize)
        .find(|&m| {
            let line = if m == n { first } else { lines.line(m) };
            line.to_ascii_lowercase().contains(&close)
        })
        .unwrap_or_else(|| (n + MAX_HEADING_LINES - 1).min(lines.len() as u32));
    let mut source = first.to_string();
    if last > n {
        source.push('\n');
        source.push_str(lines.span(n + 1, last));
    }
    static HEADING: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse("h1, h2, h3, h4, h5, h6").expect("valid selector"));
    let fragment = Html::parse_fragment(&source);
    let element = fragment.select(&HEADING).next();
    let title = element
        .map(|e| e.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let id = element
        .and_then(|e| e.value().id())
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let heading = Heading {
        line: n,
        level,
        title,
        end_line: n,
    };
    (heading, id)
}

/// GitHub-style heading anchors, with `-1`, `-2`, … on a repeated one.
#[derive(Default)]
pub(crate) struct Slugs {
    seen: HashSet<String>,
}

impl Slugs {
    /// The anchor for a heading with `title`: lowercase, letters, digits,
    /// `-` and `_` kept, spaces as `-`, anything else dropped.
    pub(crate) fn slug(&mut self, title: &str) -> String {
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

/// One ATX heading of a Markdown file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Heading {
    /// The heading's line, counting from 1.
    pub(crate) line: u32,
    /// How many `#`s it has: 1 to 6.
    pub(crate) level: usize,
    /// Its text, without the `#`s.
    pub(crate) title: String,
    /// Its section's last line: the line before the next heading, or the
    /// file's last, with trailing blank lines dropped.
    pub(crate) end_line: u32,
}

/// The ATX headings of Markdown `text`, as [`sections`] splits at them,
/// each with its section's last line.
pub(crate) fn markdown_headings(text: &str) -> Vec<Heading> {
    let lines = Lines::new(text);
    let mut headings = headings(&lines);
    let ends: Vec<u32> = (0..headings.len())
        .map(|i| section_end(&lines, &headings, i))
        .collect();
    for (heading, end) in headings.iter_mut().zip(ends) {
        heading.end_line = end;
    }
    headings
}

/// The last line of heading `i`'s section, before any trailing blank line.
fn section_end(lines: &Lines<'_>, headings: &[Heading], i: usize) -> u32 {
    let start = headings[i].line;
    let mut end = headings
        .get(i + 1)
        .map_or(lines.len() as u32, |next| next.line - 1);
    while end > start && lines.is_blank(end) {
        end -= 1;
    }
    end
}

/// Every ATX heading outside fenced code blocks, in order, with no end
/// line yet.
fn headings(lines: &Lines<'_>) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for n in 1..=lines.len() as u32 {
        let line = lines.line(n);
        let indented = line.len() - line.trim_start_matches(' ').len();
        let body = line.trim_start_matches(' ');
        if indented > 3 {
            continue;
        }
        if let Some(marker @ ('`' | '~')) = body.chars().next() {
            let run = body.chars().take_while(|&c| c == marker).count();
            // A backtick run with a backtick after it is inline code, not a
            // fence (CommonMark forbids backticks in a fence's info string).
            let inline = marker == '`' && fence.is_none() && body[run..].contains('`');
            if run >= 3 && !inline {
                match fence {
                    None => fence = Some((marker, run)),
                    Some((open, len))
                        if open == marker && run >= len && body[run..].trim().is_empty() =>
                    {
                        fence = None
                    }
                    Some(_) => {}
                }
                continue;
            }
        }
        if fence.is_none()
            && let Some(title) = atx_heading(body)
        {
            headings.push(Heading {
                line: n,
                level: body.chars().take_while(|&c| c == '#').count(),
                title,
                end_line: n,
            });
        }
    }
    headings
}

/// The text of an ATX heading line, or `None` when `line` isn't one.
fn atx_heading(line: &str) -> Option<String> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let rest = &line[level..];
    if !(1..=6).contains(&level) || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    // A closing run of `#`s is dropped when a space precedes it.
    let rest = rest.trim();
    let without_close = rest.trim_end_matches('#');
    let title = if without_close.is_empty() || without_close.ends_with([' ', '\t']) {
        without_close.trim_end()
    } else {
        rest
    };
    Some(title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(chunks: &[Chunk<'_>]) -> Vec<(u32, u32)> {
        chunks.iter().map(|c| (c.start_line, c.end_line)).collect()
    }

    #[test]
    fn lines_slice_without_line_endings() {
        let lines = Lines::new("one\r\ntwo\n\nfour");
        assert_eq!(lines.len(), 4);
        assert_eq!(lines.line(1), "one");
        assert_eq!(lines.line(3), "");
        assert_eq!(lines.span(1, 2), "one\r\ntwo");
        assert_eq!(lines.span(4, 4), "four");
        // A final newline doesn't add an empty line.
        assert_eq!(Lines::new("a\nb\n").len(), 2);
        assert_eq!(Lines::new("").len(), 0);
    }

    #[test]
    fn long_text_splits_into_sixty_line_windows() {
        let text: String = (1..=130).map(|i| format!("line {i}\n")).collect();
        let lines = Lines::new(&text);
        let chunks = windows(&lines, &vec![false; lines.len()]);
        assert_eq!(spans(&chunks), [(1, 60), (61, 120), (121, 130)]);
        assert!(chunks.iter().all(|c| c.kind == ChunkKind::Window));
        assert_eq!(chunks[2].text.lines().next(), Some("line 121"));
        assert!(chunks[2].text.ends_with("line 130"));
    }

    #[test]
    fn windows_skip_covered_lines_and_trim_lines_without_words() {
        let text = "a\n\n}\nb\nc\n\n";
        let lines = Lines::new(text);
        let covered = [false, false, false, true, false, false];
        assert_eq!(spans(&windows(&lines, &covered)), [(1, 1), (5, 5)]);
        assert!(windows(&Lines::new("\n  \n}\n"), &[false; 3]).is_empty());
    }

    #[test]
    fn markdown_splits_at_headings() {
        let text = "Intro.\n\n# Title\n\nText.\n\n## Install ##\n\n```sh\n# not a heading\n```\n\n#hashtag\n### Use\nRun it.\n";
        let lines = Lines::new(text);
        let chunks = sections(&lines);
        let got: Vec<_> = chunks
            .iter()
            .map(|c| (c.start_line, c.end_line, c.title.as_deref().unwrap()))
            .collect();
        assert_eq!(got, [(3, 5, "Title"), (7, 13, "Install"), (14, 15, "Use")]);
        assert!(chunks.iter().all(|c| c.kind == ChunkKind::Section));
        assert_eq!(chunks[2].text, "### Use\nRun it.");
    }

    #[test]
    fn markdown_sections_carry_github_anchors() {
        let text = "# Getting Started\n\n## Install (Linux)\n\n## Install (Linux)\n";
        let chunks = sections(&Lines::new(text));
        let anchors: Vec<_> = chunks
            .iter()
            .map(|c| c.anchor.as_deref().unwrap())
            .collect();
        assert_eq!(
            anchors,
            ["getting-started", "install-linux", "install-linux-1"]
        );
    }

    #[test]
    fn atx_heading_rules() {
        assert_eq!(atx_heading("# A").as_deref(), Some("A"));
        assert_eq!(atx_heading("###### Six").as_deref(), Some("Six"));
        assert_eq!(atx_heading("####### Seven"), None);
        assert_eq!(atx_heading("#NoSpace"), None);
        assert_eq!(atx_heading("#").as_deref(), Some(""));
        assert_eq!(atx_heading("## C# ##").as_deref(), Some("C#"));
        assert_eq!(atx_heading("## Issue #12").as_deref(), Some("Issue #12"));
    }

    #[test]
    fn headings_allow_three_leading_spaces_and_a_tab() {
        let text = "   # Three\n    # Four is code\n#\tTab\n";
        let chunks = sections(&Lines::new(text));
        let titles: Vec<_> = chunks.iter().map(|c| c.title.as_deref().unwrap()).collect();
        assert_eq!(titles, ["Three", "Tab"]);
        assert_eq!(spans(&chunks), [(1, 2), (3, 3)]);
    }

    #[test]
    fn a_shorter_fence_does_not_close() {
        let text = "````\n```\n# inside\n````\n# Outside\n";
        let chunks = sections(&Lines::new(text));
        assert_eq!(spans(&chunks), [(5, 5)]);
    }

    /// Each section as (start, end, title, anchor).
    fn html_outline(text: &str) -> Vec<(u32, u32, String, String)> {
        html_sections(&Lines::new(text))
            .into_iter()
            .map(|c| {
                assert_eq!(c.kind, ChunkKind::Section);
                (
                    c.start_line,
                    c.end_line,
                    c.title.unwrap(),
                    c.anchor.unwrap(),
                )
            })
            .collect()
    }

    fn row(start: u32, end: u32, title: &str, anchor: &str) -> (u32, u32, String, String) {
        (start, end, title.to_string(), anchor.to_string())
    }

    #[test]
    fn html_sections_start_at_heading_tags() {
        let text = "<!doctype html>\n<p>Before.</p>\n<h1 id=\"intro\">Intro</h1>\n<p>One.</p>\n<p>Two.</p>\n<p>Three.</p>\n<p>Four.</p>\n<p>Five.</p>\n<H2 class=\"x\">Set <em>up</em></H2>\n<p>Run.</p>\n\n";
        assert_eq!(
            html_outline(text),
            [row(3, 8, "Intro", "intro"), row(9, 10, "Set up", "set-up")]
        );
    }

    #[test]
    fn an_html_heading_split_across_lines_takes_its_whole_title() {
        let text = "<h2>\n  Set\n  up\n</h2>\n<p>Run.</p>\n";
        assert_eq!(html_outline(text), [row(1, 5, "Set up", "set-up")]);
    }

    #[test]
    fn html_headings_in_pre_comments_and_scripts_do_not_count() {
        let text = "<pre>\n<h1>Code</h1>\n</pre>\n<!--\n<h1>Gone</h1>\n-->\n<script>\nlet s = \"<h1>x</h1>\";\n</script>\n<!-- <h1>inline</h1> --><h2>Real</h2>\n<header>not a heading</header>\n<h3>Real</h3>\n";
        assert_eq!(
            html_outline(text),
            [row(10, 11, "Real", "real"), row(12, 12, "Real", "real-1")]
        );
    }

    #[test]
    fn html_headings_keep_their_level_and_survive_a_missing_close() {
        let text = "<h3>\nDeep\n</h3>\n<p>x</p>\n<h2>Open\n<p>never closed</p>\n";
        let found = html_headings(&Lines::new(text));
        let got: Vec<_> = found
            .iter()
            .map(|(h, _)| (h.line, h.level, h.title.as_str()))
            .collect();
        assert_eq!(got, [(1, 3, "Deep"), (5, 2, "Open never closed")]);
    }

    #[test]
    fn a_fence_closes_only_with_its_own_marker() {
        let text = "~~~\n```\n# inside\n~~~\n# Outside\n";
        let chunks = sections(&Lines::new(text));
        assert_eq!(spans(&chunks), [(5, 5)]);
    }
}
