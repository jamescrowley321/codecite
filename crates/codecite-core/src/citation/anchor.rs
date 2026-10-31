//! Anchor references: a heading named by its anchor rather than its lines,
//! `<repo>@<sha>:<path>#<anchor>` or `<doc>@sha256:<12 hex>#<anchor>`.
//!
//! These are input only. `read` and `cite` resolve one to the lines of the
//! heading's section and answer with the canonical line citation.

use std::fmt;
use std::str::FromStr;

use super::spec::is_doc_name;
use super::{CitationError, CommitSha, ContentHash, SectionNumber, is_line_fragment};

/// What an anchor reference is pinned to: a repository commit or a fetched
/// document's content hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pin {
    /// A file in a repository at one commit.
    Commit(CommitSha),
    /// A fetched document at one content hash.
    Hash(ContentHash),
}

/// A heading named by its anchor: `<repo>@<sha>:<path>#<anchor>` or
/// `<doc>@sha256:<hash>#<anchor>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnchorCitation {
    source: String,
    pin: Pin,
    path: Option<String>,
    anchor: String,
}

impl AnchorCitation {
    /// The repository or document name.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The commit or content hash.
    pub fn pin(&self) -> &Pin {
        &self.pin
    }

    /// The file's path inside the repository; `None` for a fetched document.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The anchor, without `#`.
    pub fn anchor(&self) -> &str {
        &self.anchor
    }
}

impl FromStr for AnchorCitation {
    type Err = CitationError;

    /// The source runs to the first `@`. Under `sha256:` the anchor follows
    /// the first `#`; under a commit the path runs to the last `#`, as in a
    /// code citation.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (source, rest) = s.split_once('@').ok_or(CitationError::MissingAt)?;
        if source.is_empty() {
            return Err(CitationError::EmptyName);
        }
        if let Some(rest) = rest.strip_prefix("sha256:") {
            if !is_doc_name(source) {
                return Err(CitationError::BadName {
                    found: source.into(),
                });
            }
            let (hash, anchor) = rest.split_once('#').ok_or(CitationError::MissingAnchor)?;
            let hash = hash.parse()?;
            if is_spec_section(anchor) {
                return Err(CitationError::NotAnAnchor {
                    found: anchor.into(),
                });
            }
            return Ok(Self {
                source: source.into(),
                pin: Pin::Hash(hash),
                path: None,
                anchor: check_anchor(anchor)?,
            });
        }
        if source.chars().any(char::is_whitespace) {
            return Err(CitationError::BadName {
                found: source.into(),
            });
        }
        let (commit, tail) = rest.split_once(':').ok_or(CitationError::MissingPath)?;
        let commit = commit.parse()?;
        let (path, anchor) = tail.rsplit_once('#').ok_or(CitationError::MissingAnchor)?;
        if path.is_empty() {
            return Err(CitationError::EmptyPath);
        }
        Ok(Self {
            source: source.into(),
            pin: Pin::Commit(commit),
            path: Some(path.into()),
            anchor: check_anchor(anchor)?,
        })
    }
}

/// Whether a fragment under `sha256:` is a spec section the canonical form
/// spells, `section-<n>` with an optional `/step-<k>`.
fn is_spec_section(fragment: &str) -> bool {
    fragment.strip_prefix("section-").is_some_and(|rest| {
        let section = rest
            .split_once("/step-")
            .map_or(rest, |(section, _)| section);
        section.parse::<SectionNumber>().is_ok()
    })
}

/// Checks an anchor: non-empty, not a line span, and only the characters
/// heading slugs and HTML ids use (letters, digits, `-`, `_`, `.`, `:`).
fn check_anchor(anchor: &str) -> Result<String, CitationError> {
    if anchor.is_empty() {
        return Err(CitationError::EmptyAnchor);
    }
    if is_line_fragment(anchor) {
        return Err(CitationError::NotAnAnchor {
            found: anchor.into(),
        });
    }
    if !anchor
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(CitationError::BadAnchor {
            found: anchor.into(),
        });
    }
    Ok(anchor.into())
}

impl fmt::Display for AnchorCitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.pin, &self.path) {
            (Pin::Commit(sha), Some(path)) => {
                write!(f, "{}@{sha}:{path}#{}", self.source, self.anchor)
            }
            (Pin::Commit(sha), None) => write!(f, "{}@{sha}#{}", self.source, self.anchor),
            (Pin::Hash(hash), _) => write!(f, "{}@sha256:{hash}#{}", self.source, self.anchor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(s: &str) -> Result<AnchorCitation, CitationError> {
        s.parse()
    }

    #[test]
    fn repository_anchor_round_trips() {
        let s = "repo@abc1234:README.md#getting-started";
        let a = anchor(s).unwrap();
        assert_eq!(a.source(), "repo");
        assert_eq!(a.pin(), &Pin::Commit("abc1234".parse().unwrap()));
        assert_eq!(a.path(), Some("README.md"));
        assert_eq!(a.anchor(), "getting-started");
        assert_eq!(a.to_string(), s);
    }

    #[test]
    fn document_anchor_round_trips() {
        let s = "notes@sha256:1a2b3c4d5e6f#install-linux-1";
        let a = anchor(s).unwrap();
        assert_eq!(a.source(), "notes");
        assert_eq!(a.pin(), &Pin::Hash("1a2b3c4d5e6f".parse().unwrap()));
        assert_eq!(a.path(), None);
        assert_eq!(a.anchor(), "install-linux-1");
        assert_eq!(a.to_string(), s);
    }

    #[test]
    fn lowercase_l_and_wordy_sections_are_anchors() {
        for s in [
            "repo@abc1234:README.md#l10",
            "notes@sha256:1a2b3c4d5e6f#l10",
            "notes@sha256:1a2b3c4d5e6f#section-four",
            "repo@abc1234:docs/a.html#section-4.3",
            "notes@sha256:1a2b3c4d5e6f#IDToken",
            "notes@sha256:1a2b3c4d5e6f#über-uns",
        ] {
            assert_eq!(anchor(s).unwrap().to_string(), s, "{s}");
        }
    }

    #[test]
    fn line_spans_are_not_anchors() {
        for s in [
            "repo@abc1234:README.md#L10",
            "repo@abc1234:README.md#L10-L12",
            "notes@sha256:1a2b3c4d5e6f#L10",
        ] {
            assert!(
                matches!(anchor(s), Err(CitationError::NotAnAnchor { .. })),
                "{s}"
            );
        }
    }

    #[test]
    fn a_spec_section_is_not_an_anchor() {
        for s in [
            "notes@sha256:1a2b3c4d5e6f#section-4",
            "notes@sha256:1a2b3c4d5e6f#section-4.3/step-2",
        ] {
            assert!(
                matches!(anchor(s), Err(CitationError::NotAnAnchor { .. })),
                "{s}"
            );
        }
    }

    #[test]
    fn empty_and_odd_anchors_are_errors() {
        assert_eq!(
            anchor("repo@abc1234:README.md#"),
            Err(CitationError::EmptyAnchor)
        );
        assert_eq!(
            anchor("notes@sha256:1a2b3c4d5e6f#"),
            Err(CitationError::EmptyAnchor)
        );
        for s in [
            "notes@sha256:1a2b3c4d5e6f#install linux",
            "notes@sha256:1a2b3c4d5e6f#a/b",
            "notes@sha256:1a2b3c4d5e6f#a#b",
            "repo@abc1234:README.md#what?",
        ] {
            assert!(
                matches!(anchor(s), Err(CitationError::BadAnchor { .. })),
                "{s}"
            );
        }
    }

    #[test]
    fn missing_pieces_are_errors() {
        assert_eq!(anchor("repo#intro"), Err(CitationError::MissingAt));
        assert_eq!(
            anchor("@abc1234:README.md#intro"),
            Err(CitationError::EmptyName)
        );
        assert_eq!(
            anchor("repo@abc1234#intro"),
            Err(CitationError::MissingPath)
        );
        assert_eq!(anchor("repo@abc1234:#intro"), Err(CitationError::EmptyPath));
        assert_eq!(
            anchor("repo@abc1234:README.md"),
            Err(CitationError::MissingAnchor)
        );
        assert_eq!(
            anchor("notes@sha256:1a2b3c4d5e6f"),
            Err(CitationError::MissingAnchor)
        );
        assert!(matches!(
            anchor("my notes@sha256:1a2b3c4d5e6f#intro"),
            Err(CitationError::BadName { .. })
        ));
        assert!(matches!(
            anchor("notes@sha256:1a2b#intro"),
            Err(CitationError::BadHash { .. })
        ));
    }
}
