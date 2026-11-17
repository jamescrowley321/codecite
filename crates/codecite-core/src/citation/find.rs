//! Finding the citations written in a file: the canonical forms codecite
//! prints, and the Markdown links to a spec section that `cite` takes.
//! What is found is only a candidate; whether it resolves is `cite`'s job.

use std::sync::OnceLock;

use regex::Regex;

/// A citation as written in some text, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenCitation<'a> {
    /// The line it starts on, from 1.
    pub line: usize,
    /// The character it starts at on that line, from 1.
    pub column: usize,
    /// The citation as written.
    pub text: &'a str,
}

/// A canonical citation: a name (a repo may hold `/`), `@`, a commit or
/// `sha256:` and a 12-digit hash, an optional `:path`, and a `#` fragment
/// (`L10-L42`, `section-4.3/step-2` or a heading's anchor). A path stops at
/// whitespace, quotes, brackets and the other characters that close a
/// citation in prose, Markdown or code.
const CANONICAL: &str = r#"[A-Za-z0-9][A-Za-z0-9._/-]*@(?:sha256:[0-9a-f]{12}|[0-9a-f]{7,64})(?::[^\s#`'"<>()\[\]{}|,;*]+)?#[A-Za-z0-9][A-Za-z0-9._/-]*"#;

/// A Markdown link whose text cites a section with `§` and whose target is
/// a web URL with an anchor, as `cite` takes it.
const SPEC_LINK: &str = r"\[[^\[\]\n]*§[^\[\]\n]*\]\(https?://[^\s()]+#[^\s()]+\)";

fn pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(&format!("{SPEC_LINK}|{CANONICAL}")).expect("the citation pattern compiles")
    })
}

/// Every citation written in `text`, in order: each canonical citation
/// (`repo@sha:path#L10-L42`, `doc@sha256:<12 hex>#L3-L9`, an anchor such
/// as `repo@sha:README.md#install`, `doc@sha256:<12 hex>#section-4.3`), and
/// each Markdown link whose text holds `§` and whose target is a URL with an
/// anchor. Trailing dots, which end a sentence rather than a citation, are
/// left out. Written spec forms in running text, such as `RFC 9449 §4.3`,
/// are not found.
pub fn find_citations(text: &str) -> Vec<WrittenCitation<'_>> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for m in pattern().find_iter(line) {
            let written = m.as_str().trim_end_matches('.');
            found.push(WrittenCitation {
                line: index + 1,
                column: line[..m.start()].chars().count() + 1,
                text: written,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn texts(text: &str) -> Vec<&str> {
        find_citations(text).into_iter().map(|c| c.text).collect()
    }

    #[test]
    fn finds_every_canonical_form() {
        let code = format!("github.com/panva/openid-client@{SHA}:lib/client.js#L412-L455");
        let lines = "notes@sha256:1a2b3c4d5e6f#L3-L9";
        let section = "rfc9449@sha256:2d8657f07291#section-4.3";
        let step = "openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13";
        let anchor = "app@0123456:docs/setup.md#install";
        let doc_anchor = "notes@sha256:1a2b3c4d5e6f#getting-started";
        let text = format!("{code}\n{lines} and {section}\n{step}\n{anchor} {doc_anchor}\n");
        assert_eq!(
            texts(&text),
            [&*code, lines, section, step, anchor, doc_anchor]
        );
    }

    #[test]
    fn finds_spec_links_whose_text_cites_a_section() {
        let link = "[OIDC Core §3.1.3.7](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)";
        let text = format!(
            "See {link}, and [the docs](https://example.com/page#top) or [§2](https://x.org/a)."
        );
        assert_eq!(texts(&text), [link]);
    }

    #[test]
    fn delimiters_and_sentence_ends_are_left_out() {
        let c = "app@0123456:src/lib.rs#L10-L20";
        for text in [
            format!("`{c}`"),
            format!("({c})"),
            format!("[{c}]"),
            format!("\"{c}\","),
            format!("'{c}';"),
            format!("<{c}>"),
            format!("**{c}**"),
            format!("see {c}."),
            format!("// {c}"),
            format!("# {c}"),
        ] {
            assert_eq!(texts(&text), [c], "{text}");
        }
        assert_eq!(
            texts("rfc9449@sha256:2d8657f07291#section-4.3."),
            ["rfc9449@sha256:2d8657f07291#section-4.3"]
        );
    }

    #[test]
    fn lines_and_columns_count_from_one_in_characters() {
        let text = "first\n§ é app@0123456:a.rs#L1 and b@abcdef0:b.rs#L2\n";
        let found = find_citations(text);
        assert_eq!(
            found,
            [
                WrittenCitation {
                    line: 2,
                    column: 5,
                    text: "app@0123456:a.rs#L1",
                },
                WrittenCitation {
                    line: 2,
                    column: 29,
                    text: "b@abcdef0:b.rs#L2",
                },
            ]
        );
    }

    #[test]
    fn text_that_only_looks_close_is_not_found() {
        for text in [
            "mail me@example.com#top",
            "git@github.com:owner/repo.git",
            "app@0123456:src/lib.rs",
            "app@012345:src/lib.rs#L1",
            "app@0123456789ABCDEF:a.rs#L1",
            "rfc9449@sha256:2d86#section-4",
            "RFC 9449 §4.3",
            "@0123456:a.rs#L1",
        ] {
            assert_eq!(texts(text), Vec::<&str>::new(), "{text}");
        }
    }
}
