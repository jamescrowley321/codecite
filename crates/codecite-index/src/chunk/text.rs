//! Line bookkeeping, 60-line windows, Markdown heading sections and their
//! anchors.

use std::collections::HashSet;

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

    #[test]
    fn a_fence_closes_only_with_its_own_marker() {
        let text = "~~~\n```\n# inside\n~~~\n# Outside\n";
        let chunks = sections(&Lines::new(text));
        assert_eq!(spans(&chunks), [(5, 5)]);
    }
}
