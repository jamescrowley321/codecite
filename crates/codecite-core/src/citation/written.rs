//! The spec citation forms people write: `RFC 9449 §4.3`,
//! `RFC9449 Section 4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)`, a
//! bare `§3.1.3.7`, and spec URLs with an anchor. They parse to an unresolved
//! [`SpecReference`]; resolving one against a corpus is `read`'s and `cite`'s
//! job.

use std::collections::HashMap;

use super::spec::is_doc_name;
use super::{CitationError, SectionNumber, parse_number};

/// The names people write for spec documents, such as `OIDC Core` for
/// `openid-connect-core-1_0`. The manifest supplies them. Lookups ignore
/// ASCII case and treat any run of whitespace as one space.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecAliases(HashMap<String, String>);

impl SpecAliases {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `alias` name the document `doc`, replacing any earlier entry.
    pub fn insert(&mut self, alias: &str, doc: &str) {
        self.0.insert(normalize(alias), doc.into());
    }

    /// The document `alias` names, if any.
    pub fn get(&self, alias: &str) -> Option<&str> {
        self.0.get(&normalize(alias)).map(String::as_str)
    }

    /// Each alias, as lookups normalize it, with the document it names, in
    /// no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(alias, doc)| (alias.as_str(), doc.as_str()))
    }
}

impl<'a> FromIterator<(&'a str, &'a str)> for SpecAliases {
    fn from_iter<I: IntoIterator<Item = (&'a str, &'a str)>>(iter: I) -> Self {
        let mut aliases = Self::new();
        for (alias, doc) in iter {
            aliases.insert(alias, doc);
        }
        aliases
    }
}

/// Lowercases ASCII and collapses whitespace, so `OIDC  Core` and
/// `oidc core` are the same alias.
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a written citation points at inside its document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SpecTarget {
    /// A numbered section, such as `3.1.3.7`.
    Section(SectionNumber),
    /// The document's own anchor, such as `IDTokenValidation` or
    /// `appendix-B`, from a URL.
    Anchor(String),
}

/// A spec citation as people write it, not yet resolved against a corpus:
/// an optional document, a target and an optional step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpecReference {
    doc: Option<String>,
    target: SpecTarget,
    step: Option<u32>,
}

impl SpecReference {
    /// The document, such as `rfc9449`, or `None` for a bare `§3.1` that
    /// names none.
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// The section or anchor cited.
    pub fn target(&self) -> &SpecTarget {
        &self.target
    }

    /// The step inside the section, if the citation names one.
    pub fn step(&self) -> Option<u32> {
        self.step
    }

    /// The same reference with its document set to `doc`, such as a bare
    /// `§3.1.3.6` given the document of the URL it links to.
    pub fn in_document(self, doc: &str) -> Self {
        Self {
            doc: Some(doc.into()),
            ..self
        }
    }

    /// Parses one written spec citation.
    ///
    /// Returns `Ok(None)` when `s` is not shaped like a written spec citation
    /// at all, so a caller can try another parser, and `Err` when it is
    /// shaped like one but malformed.
    pub fn parse(s: &str, aliases: &SpecAliases) -> Result<Option<Self>, CitationError> {
        let s = s.trim();
        // The URL branch owns every string with a scheme, even one with a
        // space or `§` in its anchor.
        if let Some(rest) = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))
        {
            return Self::url(s, rest);
        }
        if let Some((doc, locator)) = s.split_once('§') {
            return Self::written(resolve_doc(doc, aliases)?, locator.trim_start()).map(Some);
        }
        let words: Vec<&str> = s.split_whitespace().collect();
        if let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case("section")) {
            let doc = resolve_doc(&words[..i].join(" "), aliases)?;
            return Self::written(doc, &words[i + 1..].join(" ")).map(Some);
        }
        Self::bare(s, aliases)
    }

    /// An RFC Editor or OpenID spec URL with an anchor; `rest` is `url`
    /// without its scheme. Any other URL is no match.
    fn url(url: &str, rest: &str) -> Result<Option<Self>, CitationError> {
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let (path, anchor) = match rest.split_once('#') {
            Some((path, anchor)) => (path, Some(anchor)),
            None => (rest, None),
        };
        if let Some(name) = path.strip_prefix("rfc-editor.org/rfc/") {
            let name = name.strip_suffix(".html").unwrap_or(name);
            let Some(n) = name
                .strip_prefix("rfc")
                .filter(|n| n.starts_with(|c: char| c.is_ascii_digit()))
            else {
                return Ok(None);
            };
            let doc = format!("rfc{}", parse_number(n, "RFC")?);
            let anchor = need_anchor(url, anchor)?;
            let target = match anchor.strip_prefix("section-") {
                Some(section) => SpecTarget::Section(section.parse()?),
                None => SpecTarget::Anchor(anchor.into()),
            };
            return Ok(Some(Self {
                doc: Some(doc),
                target,
                step: None,
            }));
        }
        if let Some(file) = path.strip_prefix("openid.net/specs/") {
            let Some(name) = file.strip_suffix(".html").filter(|n| is_doc_name(n)) else {
                return Ok(None);
            };
            return Ok(Some(Self {
                doc: Some(name.into()),
                target: SpecTarget::Anchor(need_anchor(url, anchor)?.into()),
                step: None,
            }));
        }
        Ok(None)
    }

    /// Builds a reference from a document and the text after `§` or
    /// `Section`.
    fn written(doc: Option<String>, locator: &str) -> Result<Self, CitationError> {
        let (section, step) = parse_locator(locator)?;
        Ok(Self {
            doc,
            target: SpecTarget::Section(section),
            step,
        })
    }

    /// The bare form `<doc> <n>(<k>)`. Only a known document marks it as a
    /// citation, so an unknown one is no match rather than an error.
    fn bare(s: &str, aliases: &SpecAliases) -> Result<Option<Self>, CitationError> {
        let Some((doc, last)) = s.rsplit_once(char::is_whitespace) else {
            return Ok(None);
        };
        if !last.starts_with(|c: char| c.is_ascii_digit()) || !last.ends_with(')') {
            return Ok(None);
        }
        let doc = match resolve_doc(doc, aliases) {
            Ok(Some(doc)) => doc,
            Ok(None) | Err(CitationError::UnknownDocument { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        Self::written(Some(doc), last).map(Some)
    }
}

/// Resolves the document text before `§` or `Section`: empty is `None`, an
/// alias wins, then `RFC <n>` or `RFC<n>` is `rfc<n>`.
fn resolve_doc(text: &str, aliases: &SpecAliases) -> Result<Option<String>, CitationError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if let Some(doc) = aliases.get(text) {
        return Ok(Some(doc.into()));
    }
    if let Some(n) = text
        .strip_prefix("RFC")
        .or_else(|| text.strip_prefix("rfc"))
    {
        return Ok(Some(format!("rfc{}", parse_number(n.trim(), "RFC")?)));
    }
    Err(CitationError::UnknownDocument { found: text.into() })
}

/// The anchor of a recognized spec URL, which must be present, non-empty
/// and free of whitespace.
fn need_anchor<'a>(url: &str, anchor: Option<&'a str>) -> Result<&'a str, CitationError> {
    match anchor {
        None | Some("") => Err(CitationError::UrlWithoutAnchor { url: url.into() }),
        Some(a) if a.contains(char::is_whitespace) => {
            Err(CitationError::BadAnchor { found: a.into() })
        }
        Some(a) => Ok(a),
    }
}

/// Parses `<section>`, `<section>(<step>)` or `<section> step <step>`.
fn parse_locator(locator: &str) -> Result<(SectionNumber, Option<u32>), CitationError> {
    // `find` returns a char boundary, so `split_at` cannot panic on
    // multibyte input.
    let end = locator
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(locator.len());
    let (section, rest) = locator.split_at(end);
    let section = section.parse()?;
    let step = if rest.is_empty() {
        None
    } else if let Some(k) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        Some(parse_number(k, "step")?)
    } else if let Some(k) = step_word(rest) {
        Some(parse_number(k, "step")?)
    } else {
        return Err(CitationError::TrailingText {
            found: rest.trim().into(),
        });
    };
    Ok((section, step))
}

/// The `<k>` in ` step <k>`, when that is all of `rest`.
fn step_word(rest: &str) -> Option<&str> {
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut words = rest.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some(w), Some(k), None) if w.eq_ignore_ascii_case("step") => Some(k),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CitationError;

    /// Whether an error is the kind a malformed-input case expects.
    type ErrorCheck = fn(&CitationError) -> bool;

    fn aliases() -> SpecAliases {
        [
            ("OIDC Core", "openid-connect-core-1_0"),
            ("OIDC Core 1.0", "openid-connect-core-1_0"),
            ("OpenID Connect Core 1.0", "openid-connect-core-1_0"),
            ("Core", "openid-connect-core-1_0"),
            ("OIDC Discovery 1.0", "openid-connect-discovery-1_0"),
            ("Back-Channel Logout 1.0", "openid-connect-backchannel-1_0"),
        ]
        .into_iter()
        .collect()
    }

    /// Parses `s` and checks the document, section parts and step.
    fn check(s: &str, doc: Option<&str>, section: &[u32], step: Option<u32>) {
        let r = SpecReference::parse(s, &aliases())
            .unwrap_or_else(|e| panic!("{s}: {e}"))
            .unwrap_or_else(|| panic!("{s}: no match"));
        assert_eq!(r.doc(), doc, "{s}");
        match r.target() {
            SpecTarget::Section(n) => assert_eq!(n.parts(), section, "{s}"),
            SpecTarget::Anchor(a) => panic!("{s}: anchor {a}"),
        }
        assert_eq!(r.step(), step, "{s}");
    }

    #[test]
    fn forms_from_the_issue() {
        check("RFC 9449 §4.3", Some("rfc9449"), &[4, 3], None);
        check("RFC9449 Section 4.3", Some("rfc9449"), &[4, 3], None);
        check(
            "OIDC Core §3.1.3.7 step 13",
            Some("openid-connect-core-1_0"),
            &[3, 1, 3, 7],
            Some(13),
        );
        check(
            "Core 3.1.3.7(13)",
            Some("openid-connect-core-1_0"),
            &[3, 1, 3, 7],
            Some(13),
        );
        check("§3.1.3.7 step 12", None, &[3, 1, 3, 7], Some(12));
    }

    /// Citation shapes found in ~/repos/auth/identity-model at 9458d60
    /// (grep for `§` and `Section`), as written there.
    #[test]
    fn shapes_from_identity_model() {
        let core = Some("openid-connect-core-1_0");
        check("RFC 9449 §8", Some("rfc9449"), &[8], None);
        check("RFC 6749 §2.3.1", Some("rfc6749"), &[2, 3, 1], None);
        check("OIDC Core §3.1.3.7", core, &[3, 1, 3, 7], None);
        check("OIDC Core 1.0 §3.1.3.7", core, &[3, 1, 3, 7], None);
        check(
            "OIDC Discovery 1.0 §3",
            Some("openid-connect-discovery-1_0"),
            &[3],
            None,
        );
        check(
            "Back-Channel Logout 1.0 §2.4 step 2",
            Some("openid-connect-backchannel-1_0"),
            &[2, 4],
            Some(2),
        );
        check("§3.3.2.11", None, &[3, 3, 2, 11], None);
        check("§3.1.3.7 step 11", None, &[3, 1, 3, 7], Some(11));
        check("RFC 7592 Section 2.3", Some("rfc7592"), &[2, 3], None);
        check("OpenID Connect Core 1.0 Section 9", core, &[9], None);
        check("OIDC Core Section 10.1", core, &[10, 1], None);
        check("Section 2.2", None, &[2, 2], None);
    }

    #[test]
    fn spacing_and_case_are_forgiven() {
        let core = Some("openid-connect-core-1_0");
        check("  oidc  core   §3.1  ", core, &[3, 1], None);
        check("RFC 9449 § 4.3", Some("rfc9449"), &[4, 3], None);
        check("rfc9449 section 4.3", Some("rfc9449"), &[4, 3], None);
        check("Core §3.1.3.7 Step 13", core, &[3, 1, 3, 7], Some(13));
        check("Core §3.1.3.7(13)", core, &[3, 1, 3, 7], Some(13));
    }

    #[test]
    fn in_document_names_the_document() {
        let r = SpecReference::parse("§3.1.3.6 step 2", &aliases())
            .unwrap()
            .unwrap()
            .in_document("openid-connect-core-1_0");
        assert_eq!(r.doc(), Some("openid-connect-core-1_0"));
        assert_eq!(r.target(), &SpecTarget::Section("3.1.3.6".parse().unwrap()));
        assert_eq!(r.step(), Some(2));
    }

    #[test]
    fn iter_yields_normalized_aliases() {
        let a: SpecAliases = [("OIDC  Core", "openid-connect-core-1_0")]
            .into_iter()
            .collect();
        let pairs: Vec<(&str, &str)> = a.iter().collect();
        assert_eq!(pairs, [("oidc core", "openid-connect-core-1_0")]);
        let again: SpecAliases = a.iter().collect();
        assert_eq!(again, a);
    }

    #[test]
    fn an_alias_wins_over_the_rfc_rule() {
        let a: SpecAliases = [("RFC 6749bis", "draft-ietf-oauth-v2-1")]
            .into_iter()
            .collect();
        let r = SpecReference::parse("RFC 6749bis §4.1", &a)
            .unwrap()
            .unwrap();
        assert_eq!(r.doc(), Some("draft-ietf-oauth-v2-1"));
    }

    #[test]
    fn non_citations_are_none() {
        for s in [
            "",
            "hello world",
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
            "github.com/panva/openid-client@3f2a9c1:lib/client.js#L412",
            "len(2)",
            "Unknown 3.1(2)",
            "Core 3.1.3.7",
        ] {
            assert_eq!(SpecReference::parse(s, &aliases()), Ok(None), "{s:?}");
        }
    }

    #[test]
    fn malformed_citations_are_errors() {
        use CitationError::*;
        let cases: &[(&str, ErrorCheck)] = &[
            ("Foo §3.1", |e| matches!(e, UnknownDocument { .. })),
            ("Foo Section 3.1", |e| matches!(e, UnknownDocument { .. })),
            ("§", |e| matches!(e, BadSection { .. })),
            ("RFC 9449 §04.3", |e| matches!(e, BadSection { .. })),
            ("§3.1.3.7.", |e| matches!(e, BadSection { .. })),
            ("RFC 09449 §4.3", |e| {
                matches!(e, BadNumber { what: "RFC", .. })
            }),
            ("RFC §4.3", |e| matches!(e, BadNumber { what: "RFC", .. })),
            ("§3.1.3.7 step 0", |e| {
                matches!(e, BadNumber { what: "step", .. })
            }),
            ("§3.1.3.7 step 4-6", |e| {
                matches!(e, BadNumber { what: "step", .. })
            }),
            ("Core 3.1.3.7(013)", |e| {
                matches!(e, BadNumber { what: "step", .. })
            }),
            ("§3.1.3.7 steps 4-6", |e| matches!(e, TrailingText { .. })),
            ("§2 / §3.1.3.7", |e| matches!(e, TrailingText { .. })),
            ("§5.1–5.2", |e| matches!(e, TrailingText { .. })),
            ("RFC 9449 §8: honor", |e| matches!(e, TrailingText { .. })),
            ("§3.1.3.7 step 12 extra", |e| {
                matches!(e, TrailingText { .. })
            }),
        ];
        for (s, want) in cases {
            let got = SpecReference::parse(s, &aliases());
            assert!(matches!(&got, Err(e) if want(e)), "{s}: {got:?}");
        }
    }

    #[test]
    fn multibyte_input_is_not_a_panic() {
        for s in ["§3.1é", "§é", "Coreé 3.1(2)", "Core 3.1(2é)", "é Section 1"] {
            let _ = SpecReference::parse(s, &aliases());
        }
    }

    /// Parses a URL and checks the document and target.
    fn check_url(s: &str, doc: &str, target: SpecTarget) {
        let r = SpecReference::parse(s, &aliases())
            .unwrap_or_else(|e| panic!("{s}: {e}"))
            .unwrap_or_else(|| panic!("{s}: no match"));
        assert_eq!(r.doc(), Some(doc), "{s}");
        assert_eq!(r.target(), &target, "{s}");
        assert_eq!(r.step(), None, "{s}");
    }

    fn section(s: &str) -> SpecTarget {
        SpecTarget::Section(s.parse().unwrap())
    }

    fn anchor(s: &str) -> SpecTarget {
        SpecTarget::Anchor(s.into())
    }

    #[test]
    fn urls_from_the_issue() {
        check_url(
            "https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation",
            "openid-connect-core-1_0",
            anchor("IDTokenValidation"),
        );
        check_url(
            "https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3",
            "rfc9449",
            section("4.3"),
        );
    }

    /// URL shapes found in ~/repos/auth/identity-model at 9458d60
    /// (grep for `openid.net/specs` and `rfc-editor.org`).
    #[test]
    fn urls_from_identity_model() {
        check_url(
            "https://www.rfc-editor.org/rfc/rfc6749#section-5.2",
            "rfc6749",
            section("5.2"),
        );
        check_url(
            "https://www.rfc-editor.org/rfc/rfc8693#section-2.2.1",
            "rfc8693",
            section("2.2.1"),
        );
        check_url(
            "https://www.rfc-editor.org/rfc/rfc7636#appendix-B",
            "rfc7636",
            anchor("appendix-B"),
        );
        check_url(
            "https://openid.net/specs/openid-connect-discovery-1_0.html#ProviderMetadata",
            "openid-connect-discovery-1_0",
            anchor("ProviderMetadata"),
        );
        check_url(
            "http://openid.net/specs/openid-connect-basic-1_0-32.html#ClaimStability",
            "openid-connect-basic-1_0-32",
            anchor("ClaimStability"),
        );
        check_url(
            "https://rfc-editor.org/rfc/rfc9449#section-8",
            "rfc9449",
            section("8"),
        );
    }

    #[test]
    fn other_urls_are_none() {
        for s in [
            "https://example.com/rfc9449#section-4.3",
            "https://www.rfc-editor.org/info/rfc9449",
            "https://www.rfc-editor.org/rfc/",
            "https://openid.net/specs/",
            "https://openid.net/specs/a/b.html#x",
            "https://openid.net/foundation/",
        ] {
            assert_eq!(SpecReference::parse(s, &aliases()), Ok(None), "{s}");
        }
    }

    #[test]
    fn malformed_urls_are_errors() {
        use CitationError::*;
        let cases: &[(&str, ErrorCheck)] = &[
            ("https://www.rfc-editor.org/rfc/rfc9449", |e| {
                matches!(e, UrlWithoutAnchor { .. })
            }),
            ("https://www.rfc-editor.org/rfc/rfc9449#", |e| {
                matches!(e, UrlWithoutAnchor { .. })
            }),
            (
                "https://openid.net/specs/openid-connect-discovery-1_0.html",
                |e| matches!(e, UrlWithoutAnchor { .. }),
            ),
            ("https://www.rfc-editor.org/rfc/rfc9449#section-04.3", |e| {
                matches!(e, BadSection { .. })
            }),
            ("https://www.rfc-editor.org/rfc/rfc09449#section-1", |e| {
                matches!(e, BadNumber { what: "RFC", .. })
            }),
            (
                "https://openid.net/specs/openid-connect-core-1_0.html#ID Token",
                |e| matches!(e, BadAnchor { .. }),
            ),
        ];
        for (s, want) in cases {
            let got = SpecReference::parse(s, &aliases());
            assert!(matches!(&got, Err(e) if want(e)), "{s}: {got:?}");
        }
    }
}
