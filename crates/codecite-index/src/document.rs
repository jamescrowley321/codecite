//! A fetched page in no spec layout: rendered to text lines when it is
//! HTML, then split at its own headings like a repository file, with the
//! lines no heading covers in 60-line windows.

use scraper::Html;

use crate::chunk::{self, Chunk, Heading, Lines};
use crate::spec::line_number;
use crate::spec::render::{self, Layout};

/// How a fetched page's bytes are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Html,
    Markdown,
    Text,
}

impl Format {
    /// The value stored in `document.language`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Markdown => "markdown",
            Self::Text => "text",
        }
    }

    /// HTML when the first non-blank character is `<`, whatever the URL
    /// says; else Markdown when the URL's path ends in `.md` or
    /// `.markdown`; else plain text.
    fn detect(body: &str, url: &str) -> Self {
        if body.trim_start().starts_with('<') {
            return Self::Html;
        }
        let path = url.split(['?', '#']).next().unwrap_or_default();
        let path = path.to_ascii_lowercase();
        if path.ends_with(".md") || path.ends_with(".markdown") {
            Self::Markdown
        } else {
            Self::Text
        }
    }
}

/// A page split as a generic document.
pub(crate) struct GenericDocument {
    /// The text lines are counted in, joined with `\n`.
    pub(crate) text: String,
    pub(crate) format: Format,
    /// An HTML page's headings in the rendered text, each with its anchor.
    /// Markdown finds its own; plain text has none.
    headings: Vec<(Heading, String)>,
}

/// Spec layouts are not checked here; the caller tries `split_spec` first.
pub(crate) fn split_document(body: &str, url: &str) -> GenericDocument {
    let format = Format::detect(body, url);
    if format != Format::Html {
        return GenericDocument {
            text: body.to_owned(),
            format,
            headings: Vec::new(),
        };
    }
    let rendered = render::render(&Html::parse_document(body), Layout::Generic);
    let mut headings: Vec<(Heading, String)> = Vec::new();
    for h in rendered.headings {
        // A heading that rendered no line of its own records the line the
        // next text lands on, which the next heading may take too, or which
        // may be past the end.
        if h.line >= rendered.lines.len() {
            continue;
        }
        let Some(numbered) = h.numbered else { continue };
        let line = line_number(h.line + 1);
        if headings.last().is_some_and(|(prev, _)| prev.line == line) {
            headings.pop();
        }
        // Sections end at the next heading of any level, so the level
        // isn't kept.
        let heading = Heading {
            line,
            level: 1,
            title: numbered.title,
            end_line: line,
        };
        headings.push((heading, numbered.anchor));
    }
    let mut text = rendered.lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    GenericDocument {
        text,
        format,
        headings,
    }
}

impl GenericDocument {
    /// The page's sections and windows, ordered by start line, then
    /// longest first.
    pub(crate) fn chunks(&self) -> Vec<Chunk<'_>> {
        let lines = Lines::new(&self.text);
        let sections = match self.format {
            Format::Html => {
                let (headings, anchors): (Vec<Heading>, Vec<String>) =
                    self.headings.iter().cloned().unzip();
                chunk::sections_at(&lines, &headings, anchors)
            }
            Format::Markdown => chunk::sections(&lines),
            Format::Text => Vec::new(),
        };
        chunk::with_windows(&lines, sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChunkKind;

    const PLAIN: &str = include_str!("../tests/fixtures/document/plain.html");
    const RFC_V3: &str = include_str!("../tests/fixtures/spec/rfc-v3.html");
    const RFC_OLD: &str = include_str!("../tests/fixtures/spec/rfc6749.html");

    /// The sections as (title, anchor), in order.
    fn sections(doc: &GenericDocument) -> Vec<(String, String)> {
        doc.chunks()
            .into_iter()
            .filter(|c| c.kind == ChunkKind::Section)
            .map(|c| (c.title.unwrap(), c.anchor.unwrap()))
            .collect()
    }

    /// Lines `start..=end` of `text`, counting from 1.
    fn lines_of(text: &str, start: u32, end: u32) -> String {
        let lines: Vec<&str> = text.lines().collect();
        lines[start as usize - 1..end as usize].join("\n")
    }

    #[test]
    fn a_plain_page_splits_at_its_headings_without_nav_or_script() {
        let doc = split_document(PLAIN, "https://example.com/notes");
        assert_eq!(doc.format, Format::Html);
        assert!(!doc.text.contains("Home"), "{}", doc.text);
        assert!(!doc.text.contains("not a heading"), "{}", doc.text);
        assert!(!doc.text.contains("Release notes\n\nRelease notes"));
        assert_eq!(
            sections(&doc),
            [
                ("Release notes".into(), "release-notes".into()),
                ("Version 2".into(), "v2".into()),
                ("Version 1".into(), "v1".into()),
            ]
        );
        for c in doc.chunks() {
            assert_eq!(c.text, lines_of(&doc.text, c.start_line, c.end_line));
        }
        let v2 = doc
            .chunks()
            .into_iter()
            .find(|c| c.title.as_deref() == Some("Version 2"))
            .unwrap();
        assert_eq!(v2.text, "Version 2\n\nAdds caching.\n\nDrops the old flag.");
    }

    #[test]
    fn an_rfc_v3_page_splits_at_its_section_headings() {
        let doc = split_document(RFC_V3, "https://www.rfc-editor.org/rfc/rfc9449.html");
        let sections = sections(&doc);
        assert!(
            sections.contains(&(
                "4.3. Checking DPoP Proofs".into(),
                "name-checking-dpop-proofs".into()
            )),
            "{sections:?}"
        );
        assert!(sections.contains(&(
            "Appendix A. Document History".into(),
            "name-document-history".into()
        )));
        let toc = doc
            .chunks()
            .into_iter()
            .find(|c| c.title.as_deref() == Some("Table of Contents"))
            .unwrap();
        assert_eq!(toc.text, "Table of Contents", "the TOC <nav> is dropped");
    }

    #[test]
    fn an_older_rfc_without_heading_tags_is_all_windows() {
        let doc = split_document(RFC_OLD, "https://www.rfc-editor.org/rfc/rfc6749.html");
        assert_eq!(doc.format, Format::Html);
        let chunks = doc.chunks();
        assert!(chunks.len() > 10);
        assert!(chunks.iter().all(|c| c.kind == ChunkKind::Window));
        assert!(
            chunks
                .iter()
                .all(|c| c.end_line - c.start_line < chunk::WINDOW_LINES)
        );
    }

    #[test]
    fn markdown_at_a_md_url_splits_like_a_repository_file() {
        let body = "Intro.\n\n# Getting Started\nText.\n\n## Install (Linux)\nRun it.\n";
        let doc = split_document(body, "https://example.com/docs/README.md?raw=1");
        assert_eq!(doc.format, Format::Markdown);
        assert_eq!(doc.text, body);
        let mut chunker = chunk::Chunker::new().unwrap();
        assert_eq!(doc.chunks(), chunker.chunk("README.md", body));
        assert_eq!(
            sections(&doc),
            [
                ("Getting Started".into(), "getting-started".into()),
                ("Install (Linux)".into(), "install-linux".into()),
            ]
        );
    }

    #[test]
    fn text_is_windows() {
        let body = "# not a heading here\nplain text\n";
        let doc = split_document(body, "https://example.com/notes.txt");
        assert_eq!(doc.format, Format::Text);
        let chunks = doc.chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            (chunks[0].kind, chunks[0].start_line, chunks[0].end_line),
            (ChunkKind::Window, 1, 2)
        );
    }

    #[test]
    fn a_body_starting_with_a_tag_is_html_whatever_the_url() {
        assert_eq!(
            Format::detect("\n  <html><h1>A</h1>", "https://x/a.md"),
            Format::Html
        );
        assert_eq!(
            Format::detect("# A", "https://x/A.MARKDOWN"),
            Format::Markdown
        );
        assert_eq!(Format::detect("# A", "https://x/a.md.txt"), Format::Text);
        assert_eq!(Format::detect("", "https://x/"), Format::Text);
        let names = [Format::Html, Format::Markdown, Format::Text].map(Format::as_str);
        assert_eq!(names, ["html", "markdown", "text"]);
    }

    #[test]
    fn an_empty_page_has_no_chunks() {
        assert!(split_document("", "https://x/").chunks().is_empty());
        assert!(
            split_document("<html></html>", "https://x/")
                .chunks()
                .is_empty()
        );
    }
}
