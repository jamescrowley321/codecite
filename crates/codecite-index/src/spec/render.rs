//! Renders a parsed spec page as plain-text lines and finds its headings,
//! in one walk over the HTML tree.

use ego_tree::NodeId;
use ego_tree::iter::Edge;
use scraper::{CaseSensitivity, ElementRef, Html, Node, Selector};

use super::SpecFormat;

/// The page as lines, and every heading in it in document order.
pub(super) struct Rendered {
    /// Lines without line endings. Blank lines are empty strings, and two
    /// never follow each other.
    pub(super) lines: Vec<String>,
    pub(super) headings: Vec<Heading>,
}

/// A heading: the start of a section, or of an unnumbered part such as
/// the table of contents, which ends the section before it.
pub(super) struct Heading {
    /// The heading's line in [`Rendered::lines`], counting from 0.
    pub(super) line: usize,
    /// The number, title and anchor, for a numbered section.
    pub(super) numbered: Option<Numbered>,
}

pub(super) struct Numbered {
    pub(super) number: String,
    pub(super) title: String,
    pub(super) anchor: String,
}

/// Which layout the page uses, from the markup each one puts around its
/// section numbers.
pub(super) fn detect(html: &Html) -> Option<SpecFormat> {
    let has = |css: &str| {
        let selector = Selector::parse(css).expect("a valid selector");
        html.select(&selector).next().is_some()
    };
    if has("section > :is(h1, h2, h3, h4, h5, h6) > a.section-number") {
        Some(SpecFormat::RfcV3)
    } else if has("a[name^=\"rfc.section.\"]") {
        Some(SpecFormat::OpenId)
    } else if has("pre span a.selflink") {
        Some(SpecFormat::RfcText)
    } else {
        None
    }
}

/// Walks the tree once, writing text and recording headings as they come.
///
/// `traverse` yields an `Open` edge when the walk enters a node and a
/// `Close` edge when it leaves, so this needs no recursion and a deeply
/// nested page can't overflow the stack.
pub(super) fn render(html: &Html, format: SpecFormat) -> Rendered {
    let mut out = Writer::default();
    let mut headings = Vec::new();
    let mut lists: Vec<List> = Vec::new();
    let mut pre_depth = 0usize;
    let mut skipping: Option<NodeId> = None;

    for edge in html.tree.root().traverse() {
        match edge {
            Edge::Open(node) => {
                if skipping.is_some() {
                    continue;
                }
                match node.value() {
                    Node::Text(text) if pre_depth > 0 => out.pre_text(text),
                    Node::Text(text) => out.text(text),
                    Node::Element(_) => {
                        let el = ElementRef::wrap(node).expect("an element node");
                        if is_skipped(el) {
                            skipping = Some(node.id());
                            continue;
                        }
                        open(el, &mut out, &mut lists, &mut pre_depth);
                        if let Some(numbered) = heading(format, el) {
                            headings.push(Heading {
                                line: out.next_line(),
                                numbered,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Edge::Close(node) => {
                if skipping == Some(node.id()) {
                    skipping = None;
                } else if skipping.is_none()
                    && let Some(el) = ElementRef::wrap(node)
                {
                    close(el, &mut out, &mut lists, &mut pre_depth);
                }
            }
        }
    }
    let mut lines = out.finish();
    if format == SpecFormat::RfcText {
        lines = drop_page_footers(lines, &mut headings);
        add_plain_headings(&lines, &mut headings);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    Rendered { lines, headings }
}

/// Elements whose text is not part of the spec: page furniture, scripts,
/// and the links each layout adds beside its paragraphs and headings.
fn is_skipped(el: ElementRef) -> bool {
    let e = el.value();
    let class = |name| e.has_class(name, CaseSensitivity::AsciiCaseInsensitive);
    match e.name() {
        "head" | "script" | "style" | "noscript" | "template" => true,
        // The ¶ after each paragraph of an xml2rfc v3 page.
        "a" => class("pilcrow"),
        // The "TOC" link box before each heading of an OpenID page.
        "table" => class("TOCbug"),
        // The page header and footer at each page break of an older RFC.
        "span" => class("grey"),
        _ => false,
    }
}

/// A list being written, for numbering its items.
enum List {
    /// `<ol>`: the next item's number and whether it is written as a letter.
    /// `start` is read as a `u32`, and the count is a `u64`, so adding one
    /// per item cannot overflow.
    Ordered { next: u64, letters: Option<u8> },
    /// `<ul>`.
    Unordered,
}

fn open(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut usize) {
    let e = el.value();
    match e.name() {
        "pre" => {
            out.end_paragraph();
            *pre_depth += 1;
        }
        "ol" => {
            out.end_paragraph();
            let next = e
                .attr("start")
                .and_then(|s| s.parse::<u32>().ok())
                .map_or(1, u64::from);
            let letters = match e.attr("type") {
                Some("a") => Some(b'a'),
                Some("A") => Some(b'A'),
                _ => None,
            };
            lists.push(List::Ordered { next, letters });
        }
        "ul" => {
            out.end_paragraph();
            lists.push(List::Unordered);
        }
        "li" => {
            out.end_line();
            match lists.last_mut() {
                Some(List::Ordered { next, letters }) => {
                    let label = match *letters {
                        Some(base) if (1..=26).contains(next) => {
                            char::from(base + (*next - 1) as u8).to_string()
                        }
                        _ => next.to_string(),
                    };
                    out.label = Some(format!("{label}. "));
                    *next += 1;
                }
                Some(List::Unordered) | None => out.label = Some("- ".to_owned()),
            }
        }
        "br" => out.break_line(pre_depth),
        "dt" | "dd" | "tr" => out.end_line(),
        "td" | "th" => out.text(" "),
        name if is_block(name) => out.end_paragraph(),
        _ => {}
    }
}

fn close(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut usize) {
    match el.value().name() {
        "pre" => {
            *pre_depth = pre_depth.saturating_sub(1);
            out.end_paragraph();
        }
        "ol" | "ul" => {
            lists.pop();
            out.end_paragraph();
        }
        "li" => {
            // An item with no text never wrote its label; drop it here so
            // it can't land on the text after the list.
            out.label = None;
            out.end_line();
        }
        "dt" | "dd" | "tr" => out.end_line(),
        name if is_block(name) => out.end_paragraph(),
        _ => {}
    }
}

/// Elements that start and end a paragraph.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "dl"
            | "table"
            | "blockquote"
            | "figure"
            | "figcaption"
            | "address"
            | "aside"
            | "nav"
            | "header"
            | "footer"
            | "hr"
            | "center"
    )
}

/// `Some` when `el` is a heading in this layout, holding its number, title
/// and anchor when it starts a numbered section.
fn heading(format: SpecFormat, el: ElementRef) -> Option<Option<Numbered>> {
    let e = el.value();
    let is_h = matches!(e.name(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    match format {
        // <section id="section-4.3"><h3><a class="section-number">4.3. </a>
        //   <a class="section-name">Checking DPoP Proofs</a></h3>
        SpecFormat::RfcV3 if is_h => Some(rfc_v3_heading(el)),
        // <span class="h3"><a class="selflink" id="section-1.2">1.2</a>.  Protocol Flow</span>
        SpecFormat::RfcText
            if e.name() == "span"
                && ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .any(|h| e.has_class(h, CaseSensitivity::CaseSensitive)) =>
        {
            Some(rfc_text_heading(el))
        }
        // <a name="IDTokenValidation"></a><br /><hr />
        // <a name="rfc.section.3.1.3.7"></a><h3>3.1.3.7.&nbsp;ID Token Validation</h3>
        SpecFormat::OpenId if is_h => Some(openid_heading(el)),
        _ => None,
    }
}

fn rfc_v3_heading(el: ElementRef) -> Option<Numbered> {
    let section = el.parent().and_then(ElementRef::wrap)?;
    let anchor = section.value().id()?;
    let number = section_number(anchor)?;
    let child = |class| {
        el.child_elements()
            .find(|c| c.value().has_class(class, CaseSensitivity::CaseSensitive))
    };
    child("section-number")?;
    let title = match child("section-name") {
        Some(name) => collapse(&name.text().collect::<String>()),
        None => title_after(el, number),
    };
    Some(Numbered {
        number: number.to_owned(),
        title,
        anchor: anchor.to_owned(),
    })
}

fn rfc_text_heading(el: ElementRef) -> Option<Numbered> {
    let link = el.child_elements().find(|c| {
        c.value()
            .has_class("selflink", CaseSensitivity::CaseSensitive)
    })?;
    let anchor = link.value().id()?;
    let number = section_number(anchor)?;
    Some(Numbered {
        number: number.to_owned(),
        title: title_after(el, number),
        anchor: anchor.to_owned(),
    })
}

fn openid_heading(el: ElementRef) -> Option<Numbered> {
    let mut before = el.prev_siblings().filter_map(ElementRef::wrap);
    let marker = before.next()?;
    let number = marker.value().attr("name")?.strip_prefix("rfc.section.")?;
    if !is_section_number(number) {
        return None;
    }
    // The author's anchor sits before the <br /><hr /> that precedes the
    // TOC box and the rfc.section marker. Sections without one have only
    // the marker.
    let anchor = before
        .find(|e| !matches!(e.value().name(), "br" | "hr" | "table"))
        .filter(|e| e.value().name() == "a")
        .and_then(|e| e.value().attr("name"))
        .filter(|name| !name.starts_with("rfc."))
        .map_or_else(|| format!("rfc.section.{number}"), str::to_owned);
    Some(Numbered {
        number: number.to_owned(),
        title: title_after(el, number),
        anchor,
    })
}

/// The section number in an RFC anchor: `4.3` in `section-4.3`, `A.1` in
/// `appendix-A.1`. `None` for anchors such as `section-abstract`.
fn section_number(anchor: &str) -> Option<&str> {
    let number = anchor
        .strip_prefix("section-")
        .or_else(|| anchor.strip_prefix("appendix-"))?;
    is_section_number(number).then_some(number)
}

/// `4`, `4.3.1`, `A` or `A.1`: digits, or one capital letter, then
/// dot-separated digits.
fn is_section_number(number: &str) -> bool {
    let mut parts = number.split('.');
    let first = parts.next().unwrap_or_default();
    let first_ok = (!first.is_empty() && first.bytes().all(|b| b.is_ascii_digit()))
        || (first.len() == 1 && first.bytes().all(|b| b.is_ascii_uppercase()));
    first_ok && parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The heading's text after `Appendix`, the number and its dot.
fn title_after(el: ElementRef, number: &str) -> String {
    let text = collapse(&el.text().collect::<String>());
    let rest = text.strip_prefix("Appendix ").unwrap_or(&text);
    match rest.strip_prefix(number) {
        Some(rest) => rest.trim_start_matches('.').trim().to_owned(),
        None => text,
    }
}

/// `text` with each run of whitespace, no-break spaces included, as one
/// space, and none at either end.
fn collapse(text: &str) -> String {
    text.split(|c: char| c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Drops the page footers an older RFC writes outside a `grey` span, as
/// rfc-editor.org does on the last page: an unindented line ending in
/// `[Page 76]`. A blank line left after another blank line goes too, and
/// each heading moves to its line's new index.
fn drop_page_footers(lines: Vec<String>, headings: &mut [Heading]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    let mut new_index = Vec::with_capacity(lines.len());
    for line in lines {
        new_index.push(kept.len());
        let blank_after_blank = line.is_empty() && kept.last().is_none_or(String::is_empty);
        if !is_page_footer(&line) && !blank_after_blank {
            kept.push(line);
        }
    }
    for heading in headings {
        heading.line = new_index.get(heading.line).copied().unwrap_or(kept.len());
    }
    kept
}

/// `Hardt   Standards Track   [Page 76]`: unindented, ending in a page
/// number in brackets.
fn is_page_footer(line: &str) -> bool {
    let unindented = line.starts_with(|c: char| !c.is_whitespace());
    let page = line
        .rsplit_once("[Page ")
        .and_then(|(_, rest)| rest.strip_suffix(']'));
    unindented && page.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// In an older RFC an unnumbered heading is a line that starts in the
/// first column, as "Author's Address" does; body text is indented. Each
/// such line after the first section heading ends the section before it.
fn add_plain_headings(lines: &[String], headings: &mut Vec<Heading>) {
    let Some(first) = headings
        .iter()
        .find(|h| h.numbered.is_some())
        .map(|h| h.line)
    else {
        return;
    };
    let mut known: Vec<usize> = headings.iter().map(|h| h.line).collect();
    for (n, line) in lines.iter().enumerate().skip(first + 1) {
        let starts_unindented = line.chars().next().is_some_and(|c| !c.is_whitespace());
        if starts_unindented && !known.contains(&n) {
            headings.push(Heading {
                line: n,
                numbered: None,
            });
            known.push(n);
        }
    }
    headings.sort_by_key(|h| h.line);
}

/// Builds the lines: collapses whitespace outside `<pre>`, keeps it inside,
/// and never writes two blank lines in a row.
#[derive(Default)]
struct Writer {
    lines: Vec<String>,
    line: String,
    /// A list item's `3. ` or `- `, written before the item's first text,
    /// so it stays on that text's line when a `<p>` inside the item starts
    /// a new paragraph.
    label: Option<String>,
}

impl Writer {
    /// Text outside `<pre>`: each run of whitespace becomes one space, and
    /// a line never starts with one.
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_whitespace() {
                if !self.line.is_empty() && !self.line.ends_with(' ') {
                    self.line.push(' ');
                }
            } else {
                self.write_label();
                self.line.push(c);
            }
        }
    }

    fn write_label(&mut self) {
        if let Some(label) = self.label.take() {
            self.line.push_str(&label);
        }
    }

    /// Text inside `<pre>`, kept as written apart from no-break spaces.
    fn pre_text(&mut self, text: &str) {
        let mut segments = text.split('\n');
        if let Some(first) = segments.next() {
            self.push_str_pre(first);
        }
        for segment in segments {
            self.push_line();
            self.push_str_pre(segment);
        }
    }

    fn push_str_pre(&mut self, segment: &str) {
        if !segment.trim().is_empty() {
            self.write_label();
        }
        self.line
            .extend(segment.chars().map(|c| if c == '\u{a0}' { ' ' } else { c }));
    }

    /// `<br>`: a new line, which inside `<pre>` may be blank.
    fn break_line(&mut self, pre_depth: &usize) {
        if *pre_depth > 0 {
            self.push_line();
        } else {
            self.end_line();
        }
    }

    /// Ends the current line, if it has any text.
    fn end_line(&mut self) {
        if !self.line.trim().is_empty() {
            self.push_line();
        } else {
            self.line.clear();
        }
    }

    /// Ends the current line and leaves a blank line after it.
    fn end_paragraph(&mut self) {
        self.end_line();
        self.push_line();
    }

    /// The index of the line the next text lands on.
    fn next_line(&self) -> usize {
        self.lines.len()
    }

    /// Writes the current line, trimmed at the end. A blank line is
    /// dropped at the start and after another blank line.
    fn push_line(&mut self) {
        let line = std::mem::take(&mut self.line);
        let line = line.trim_end();
        if line.is_empty() && self.lines.last().is_none_or(String::is_empty) {
            return;
        }
        self.lines.push(line.to_owned());
    }

    fn finish(mut self) -> Vec<String> {
        self.end_line();
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_numbers() {
        for good in ["1", "4.3", "3.1.3.7", "12", "A", "A.1", "B.10"] {
            assert!(is_section_number(good), "{good}");
        }
        for bad in [
            "",
            "abstract",
            "boilerplate.1",
            "toc.1",
            "4.",
            ".4",
            "4..3",
            "AB",
            "a",
            "4.a",
        ] {
            assert!(!is_section_number(bad), "{bad}");
        }
        assert_eq!(section_number("section-4.3"), Some("4.3"));
        assert_eq!(section_number("appendix-A.1"), Some("A.1"));
        assert_eq!(section_number("section-abstract"), None);
        assert_eq!(section_number("name-introduction"), None);
    }

    #[test]
    fn page_footers_outside_grey_spans_are_dropped() {
        let html = Html::parse_document(
            "<pre><span class=\"h2\"><a class=\"selflink\" id=\"section-1\">1</a>.  Intro</span>\n\
             \n   Text.\n\nHardt     Standards Track     [Page 76]\n\n   See [Page 2].\n</pre>",
        );
        let rendered = render(&html, SpecFormat::RfcText);
        assert_eq!(
            rendered.lines,
            ["1.  Intro", "", "   Text.", "", "   See [Page 2]."]
        );
        assert_eq!(rendered.headings[0].line, 0);
        assert!(is_page_footer(
            "Hardt (editor)    Standards Track    [Page 3]"
        ));
        assert!(!is_page_footer("   indented text [Page 3]"));
        assert!(!is_page_footer("Hardt [Page ]"));
        assert!(!is_page_footer("Hardt [Page 3] and more"));
    }

    #[test]
    fn collapse_handles_no_break_spaces() {
        assert_eq!(
            collapse("3.1.3.7.\u{a0}\n  ID Token\tValidation "),
            "3.1.3.7. ID Token Validation"
        );
    }

    #[test]
    fn lists_are_numbered_from_their_start() {
        let html = Html::parse_document(
            "<section id=\"section-1\"><h2><a class=\"section-number\">1. </a>Lists</h2>\
             <ol start=\"3\"><li>three</li><li>four</li></ol>\
             <ol type=\"a\"><li>first</li><li>second</li></ol>\
             <ul><li>dot</li></ul></section>",
        );
        let lines = render(&html, SpecFormat::RfcV3).lines;
        assert_eq!(
            lines,
            [
                "1. Lists",
                "",
                "3. three",
                "4. four",
                "",
                "a. first",
                "b. second",
                "",
                "- dot"
            ]
        );
    }

    #[test]
    fn list_numbers_past_u32_max_keep_counting() {
        let html = Html::parse_document(
            "<section id=\"section-1\"><h2><a class=\"section-number\">1. </a>Big</h2>\
             <ol start=\"4294967295\"><li>max</li><li>past</li></ol></section>",
        );
        let lines = render(&html, SpecFormat::RfcV3).lines;
        assert_eq!(lines, ["1. Big", "", "4294967295. max", "4294967296. past"]);
    }

    #[test]
    fn an_empty_item_label_does_not_leak_out_of_its_list() {
        let html = Html::parse_document(
            "<section id=\"section-1\"><h2><a class=\"section-number\">1. </a>Empty</h2>\
             <ol><li></li></ol><p>MUST validate.</p></section>",
        );
        let lines = render(&html, SpecFormat::RfcV3).lines;
        assert_eq!(lines, ["1. Empty", "", "MUST validate."]);
    }
}
