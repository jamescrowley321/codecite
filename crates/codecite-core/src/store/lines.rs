//! A chunk's text as a line range of its document's stored text.
//!
//! A code file or generic document keeps its whole text in
//! `document.text`, and each of its chunks is a run of whole lines of it,
//! so `chunk.text` is NULL for them and is cut from the document's text
//! when a reader needs it. Lines are counted as the index's chunker counts
//! them: each `\n` ends a line, a final `\n` starts no empty one, and a
//! span drops its last line's `\n` and then a `\r`, keeping every other
//! line's ending as written.

/// Lines `start..=end` (from 1) of `text`, as the chunker cut them, or
/// `None` when `text` has no line `start` or the range is backwards.
///
/// Scans only as far as line `end`, so a reader that cuts one chunk out of
/// a large file stops early. [`LineStarts`] cuts many chunks of one text.
pub(crate) fn line_span(text: &str, start: u32, end: u32) -> Option<&str> {
    if start == 0 || end < start {
        return None;
    }
    let mut starts = text.match_indices('\n').map(|(i, _)| i + 1);
    let from = if start == 1 {
        0
    } else {
        starts.nth(start as usize - 2)?
    };
    if from >= text.len() && start > 1 {
        return None;
    }
    let to = starts.nth((end - start) as usize).unwrap_or(text.len());
    Some(trim_ending(&text[from..to]))
}

/// The byte offset of every line of a text, for cutting many spans out of
/// it. Cuts exactly what [`line_span`] cuts.
pub(crate) struct LineStarts<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineStarts<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        // A final newline ends the last line; it doesn't start an empty one.
        if starts.len() > 1 && starts.last() == Some(&text.len()) {
            starts.pop();
        }
        Self { text, starts }
    }

    /// Lines `start..=end` (from 1), as [`line_span`] cuts them.
    pub(crate) fn span(&self, start: u32, end: u32) -> Option<&'a str> {
        if start == 0 || end < start {
            return None;
        }
        let from = *self.starts.get(start as usize - 1)?;
        let to = self
            .starts
            .get(end as usize)
            .copied()
            .unwrap_or(self.text.len());
        Some(trim_ending(&self.text[from..to]))
    }
}

/// `span` without its last line's ending: a `\n`, then a `\r`.
fn trim_ending(span: &str) -> &str {
    let span = span.strip_suffix('\n').unwrap_or(span);
    span.strip_suffix('\r').unwrap_or(span)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXTS: &[&str] = &[
        "",
        "\n",
        "\n\n",
        "a",
        "a\n",
        "a\nb",
        "a\nb\n",
        "a\r\nb\r\n",
        "a\r\n\r\nb",
        "fn a() {}\n\n/// Doc.\nfn b() {\n    c();\n}\n",
        "é\n\u{1F600}\nz",
    ];

    #[test]
    fn spans_are_whole_lines_without_the_last_ending() {
        let text = "fn a() {\r\n    b();\r\n}\n\nfn c() {}\n";
        assert_eq!(line_span(text, 1, 3), Some("fn a() {\r\n    b();\r\n}"));
        assert_eq!(line_span(text, 2, 2), Some("    b();"));
        assert_eq!(line_span(text, 4, 4), Some(""));
        assert_eq!(line_span(text, 5, 5), Some("fn c() {}"));
        // A final newline starts no line 6.
        assert_eq!(line_span(text, 6, 6), None);
        assert_eq!(line_span(text, 0, 1), None);
        assert_eq!(line_span(text, 3, 2), None);
        // An end past the last line runs to the end, as the chunker's does.
        assert_eq!(line_span(text, 5, 9), Some("fn c() {}"));
    }

    #[test]
    fn line_starts_cut_what_line_span_cuts() {
        for text in TEXTS {
            let starts = LineStarts::new(text);
            for start in 0..6 {
                for end in 0..7 {
                    assert_eq!(
                        starts.span(start, end),
                        line_span(text, start, end),
                        "{text:?} {start}..={end}"
                    );
                }
            }
        }
    }
}
