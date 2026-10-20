//! Reads an RFC's relations to other RFCs from its header.

use scraper::{Html, Selector};

use super::{SpecFormat, SpecRelations};

/// The relations an RFC's header lists. OpenID specs have none.
pub(super) fn relations(html: &Html, format: SpecFormat) -> SpecRelations {
    match format {
        SpecFormat::RfcV3 => v3_relations(html),
        SpecFormat::RfcText => text_relations(html),
        SpecFormat::OpenId => SpecRelations::default(),
    }
}

/// An xml2rfc v3 RFC lists them in its identifiers block:
/// `<dl id="identifiers">…<dt class="label-updates">Updates:</dt>
/// <dd class="updates"><a href="…/rfc6749">6749</a>, …</dd>`.
fn v3_relations(html: &Html) -> SpecRelations {
    let field = |class: &str| {
        let selector =
            Selector::parse(&format!("dl#identifiers > dd.{class}")).expect("a valid selector");
        html.select(&selector)
            .flat_map(|dd| rfc_names(&dd.text().collect::<String>()))
            .collect()
    };
    SpecRelations {
        obsoletes: field("obsoletes"),
        updates: field("updates"),
        obsoleted_by: field("obsoleted-by"),
        updated_by: field("updated-by"),
    }
}

/// An older RFC writes them in the left column of the plain-text header,
/// the first `<pre>` on the page:
///
/// ```text
/// Request for Comments: 8252                                        Google
/// Updates: 6749                                              Ping Identity
/// ```
///
/// A list too long for one line goes on in the next lines, indented.
fn text_relations(html: &Html) -> SpecRelations {
    let mut relations = SpecRelations::default();
    let selector = Selector::parse("pre").expect("a valid selector");
    let Some(pre) = html.select(&selector).next() else {
        return relations;
    };
    let header = pre.text().collect::<String>();
    let mut field: Option<&mut Vec<String>> = None;
    for line in header.lines() {
        // The left column ends at the first run of two or more spaces.
        let left = line.trim_end().split("  ").next().unwrap_or_default();
        let value = if let Some((name, value)) = left.split_once(':') {
            field = match name {
                "Obsoletes" => Some(&mut relations.obsoletes),
                "Updates" => Some(&mut relations.updates),
                "Obsoleted by" => Some(&mut relations.obsoleted_by),
                "Updated by" => Some(&mut relations.updated_by),
                _ => None,
            };
            value
        } else if line.starts_with(' ') {
            // A continuation line: kept only after a field ending in ','.
            line.trim()
        } else {
            field = None;
            continue;
        };
        let Some(names) = field.as_deref_mut() else {
            continue;
        };
        names.extend(rfc_names(value));
        if !value.trim_end().ends_with(',') {
            field = None;
        }
    }
    relations
}

/// `rfc6749` for each run of digits in `6749, 6750 (if approved)`.
fn rfc_names(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty() && !n.starts_with('0'))
        .map(|n| format!("rfc{n}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn v3_identifiers_block() {
        let html = Html::parse_document(
            "<dl id=\"identifiers\"><dt class=\"label-rfc\">RFC:</dt><dd class=\"rfc\">9700</dd>\
             <dt class=\"label-obsoletes\">Obsoletes:</dt><dd class=\"obsoletes\">\
             <a href=\"https://www.rfc-editor.org/rfc/rfc6819\">6819</a></dd>\
             <dt class=\"label-updates\">Updates:</dt><dd class=\"updates\">\
             <a href=\"https://www.rfc-editor.org/rfc/rfc6749\">6749</a>, \
             <a href=\"https://www.rfc-editor.org/rfc/rfc6750\">6750</a> </dd></dl>\
             <p class=\"updates\">Updates: 1234</p>",
        );
        assert_eq!(
            relations(&html, SpecFormat::RfcV3),
            SpecRelations {
                obsoletes: names(&["rfc6819"]),
                updates: names(&["rfc6749", "rfc6750"]),
                obsoleted_by: Vec::new(),
                updated_by: Vec::new(),
            }
        );
    }

    #[test]
    fn text_header() {
        let html = Html::parse_document(
            "<pre>Internet Engineering Task Force (IETF)          D. Hardt, Ed.\n\
             Request for Comments: 6749                        Microsoft\n\
             Obsoletes: <a href=\"./rfc5849\">5849</a>                    October 2012\n\
             Updates: 1034, 1035, 1123,                        Someone\n\
             \x20        2181\n\
             Updated by: 8252\n\
             Category: Standards Track\n\
             ISSN: 2070-1721\n\
             \n   Obsoletes: 1111 is body text.</pre><pre>Updates: 2222</pre>",
        );
        assert_eq!(
            relations(&html, SpecFormat::RfcText),
            SpecRelations {
                obsoletes: names(&["rfc5849"]),
                updates: names(&["rfc1034", "rfc1035", "rfc1123", "rfc2181"]),
                obsoleted_by: Vec::new(),
                updated_by: names(&["rfc8252"]),
            }
        );
    }

    #[test]
    fn openid_pages_have_none() {
        let html = Html::parse_document("<pre>Updates: 6749</pre>");
        assert_eq!(
            relations(&html, SpecFormat::OpenId),
            SpecRelations::default()
        );
    }

    #[test]
    fn rfc_names_skip_text_and_leading_zeros() {
        assert_eq!(
            rfc_names(" 6749, 6750 (if approved) 0123"),
            names(&["rfc6749", "rfc6750"])
        );
        assert!(rfc_names("none").is_empty());
    }
}
