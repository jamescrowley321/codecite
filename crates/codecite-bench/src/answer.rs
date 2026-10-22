//! Labelled answers and whether a search result's citation is one of them.

use anyhow::{Context as _, bail};

/// A span that answers a query, as written in the queries file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// A spec section by number, or one step of it: `rfc9449#section-4.3` or
    /// `rfc9449#section-4.3/step-8`.
    Spec {
        /// The document's name.
        document: String,
        /// The section anchor, with `/step-<n>` for one step.
        anchor: String,
    },
    /// A line of a code file, normally a definition's first line:
    /// `py/src/client.py#L42`.
    Code {
        /// The file's path in its source.
        path: String,
        /// The line, counted from 1.
        line: u32,
    },
}

impl Answer {
    /// Parses an answer label.
    pub fn parse(label: &str) -> anyhow::Result<Answer> {
        let Some((target, fragment)) = label.split_once('#') else {
            bail!("answer `{label}` has no `#`");
        };
        if target.is_empty() || fragment.is_empty() {
            bail!("answer `{label}` is missing its target or fragment");
        }
        if let Some(line) = fragment
            .strip_prefix('L')
            .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            let line = line
                .parse()
                .with_context(|| format!("answer `{label}` has a bad line number"))?;
            return Ok(Answer::Code {
                path: target.to_owned(),
                line,
            });
        }
        // Search cites a section by number, never by the page's own anchor
        // (`#IDTokenValidation`), so a label written that way could never
        // match.
        if !fragment.starts_with("section-") {
            bail!(
                "answer `{label}` is neither `#L<line>` nor `#section-<number>`; \
                 write a spec section by the number search cites"
            );
        }
        Ok(Answer::Spec {
            document: target.to_owned(),
            anchor: fragment.to_owned(),
        })
    }

    /// Whether the result cited by `citation` is this answer.
    ///
    /// A spec result matches its section and every step in it, from any
    /// version of the document. A code result matches when it is in the
    /// file and its lines hold the answer's line.
    pub fn matches(&self, citation: &Citation) -> bool {
        match (self, citation) {
            (
                Answer::Spec { document, anchor },
                Citation::Spec {
                    document: cited,
                    fragment,
                },
            ) => {
                document == cited
                    && fragment
                        .strip_prefix(anchor.as_str())
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            }
            (Answer::Code { path, line }, Citation::Code { path: cited, lines }) => {
                path == cited && lines.0 <= *line && *line <= lines.1
            }
            _ => false,
        }
    }
}

/// A search result's citation, as `codecite search --json` prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Citation {
    /// `rfc9449@sha256:2d8657f07291#section-4.3/step-8`.
    Spec {
        /// The document's name.
        document: String,
        /// Everything after the `#`.
        fragment: String,
    },
    /// `identity-model@9458d60f:py/src/client.py#L12-L30`.
    Code {
        /// The file's path in its source.
        path: String,
        /// The first and last lines.
        lines: (u32, u32),
    },
    /// Anything else, which matches no answer.
    Other,
}

impl Citation {
    /// Parses a citation. One this runner doesn't know is [`Citation::Other`].
    pub fn parse(citation: &str) -> Citation {
        let Some((target, fragment)) = citation.split_once('#') else {
            return Citation::Other;
        };
        let Some((name, revision)) = target.split_once('@') else {
            return Citation::Other;
        };
        if revision.starts_with("sha256:") {
            return Citation::Spec {
                document: name.to_owned(),
                fragment: fragment.to_owned(),
            };
        }
        let Some((_, path)) = revision.split_once(':') else {
            return Citation::Other;
        };
        let Some(lines) = fragment.strip_prefix('L') else {
            return Citation::Other;
        };
        let (start, end) = lines.split_once("-L").unwrap_or((lines, lines));
        match (start.parse(), end.parse()) {
            (Ok(start), Ok(end)) => Citation::Code {
                path: path.to_owned(),
                lines: (start, end),
            },
            _ => Citation::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(answer: &str, citation: &str) -> bool {
        Answer::parse(answer)
            .unwrap()
            .matches(&Citation::parse(citation))
    }

    #[test]
    fn a_section_matches_itself_and_its_steps() {
        let answer = "rfc9449#section-4.3";
        assert!(hit(answer, "rfc9449@sha256:2d8657f07291#section-4.3"));
        assert!(hit(
            answer,
            "rfc9449@sha256:2d8657f07291#section-4.3/step-8"
        ));
        assert!(!hit(answer, "rfc9449@sha256:2d8657f07291#section-4.31"));
        assert!(!hit(answer, "rfc9449@sha256:2d8657f07291#section-4"));
        assert!(!hit(answer, "rfc6749@sha256:0123456789ab#section-4.3"));
    }

    #[test]
    fn a_step_matches_only_itself() {
        let answer = "rfc9449#section-4.3/step-8";
        assert!(hit(
            answer,
            "rfc9449@sha256:2d8657f07291#section-4.3/step-8"
        ));
        assert!(!hit(answer, "rfc9449@sha256:2d8657f07291#section-4.3"));
        assert!(!hit(
            answer,
            "rfc9449@sha256:2d8657f07291#section-4.3/step-80"
        ));
    }

    #[test]
    fn a_code_line_matches_the_spans_that_hold_it() {
        let answer = "py/src/client.py#L42";
        assert!(hit(
            answer,
            "identity-model@9458d60f:py/src/client.py#L40-L60"
        ));
        assert!(hit(
            answer,
            "identity-model@9458d60f:py/src/client.py#L42-L42"
        ));
        assert!(hit(answer, "identity-model@9458d60f:py/src/client.py#L42"));
        assert!(!hit(
            answer,
            "identity-model@9458d60f:py/src/client.py#L43-L60"
        ));
        assert!(!hit(
            answer,
            "identity-model@9458d60f:py/src/other.py#L40-L60"
        ));
        assert!(!hit(answer, "rfc9449@sha256:2d8657f07291#L42"));
    }

    #[test]
    fn bad_labels_are_errors() {
        assert!(Answer::parse("rfc9449").is_err());
        assert!(Answer::parse("#section-1").is_err());
        assert!(Answer::parse("a.py#Lx").is_err());
        assert!(Answer::parse("openid-connect-core-1_0#IDTokenValidation").is_err());
        assert!(Answer::parse("openid-connect-rpinitiated-1_0#Logout").is_err());
    }

    #[test]
    fn unknown_citations_match_nothing() {
        assert_eq!(Citation::parse("nonsense"), Citation::Other);
        assert_eq!(Citation::parse("a@b:c.py#section-1"), Citation::Other);
    }
}
