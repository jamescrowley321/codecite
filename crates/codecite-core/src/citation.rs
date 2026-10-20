//! Canonical citations: the exact strings codecite hands to agents to name a
//! span of code or a spec section.
//!
//! Parsing is strict. Every string the parser accepts formats back to exactly
//! the same string, so each span has one spelling.

use std::fmt;
use std::str::FromStr;

mod code;
pub use code::{CodeCitation, CommitSha, LineRange};

mod spec;
pub use spec::{ContentHash, SectionNumber, SpecCitation};

/// A canonical citation of either form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Citation {
    /// A span of code: `<repo>@<sha>:<path>#L<a>-L<b>`.
    Code(CodeCitation),
    /// A spec section or step: `<doc>@sha256:<hash>#section-<n>[/step-<k>]`.
    Spec(SpecCitation),
}

impl FromStr for Citation {
    type Err = CitationError;

    /// Picks the spec form when the text after the first `@` starts with
    /// `sha256:`, and the code form otherwise.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (_, rest) = s.split_once('@').ok_or(CitationError::MissingAt)?;
        if rest.starts_with("sha256:") {
            s.parse().map(Citation::Spec)
        } else {
            s.parse().map(Citation::Code)
        }
    }
}

impl fmt::Display for Citation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Citation::Code(c) => c.fmt(f),
            Citation::Spec(c) => c.fmt(f),
        }
    }
}

/// Why a string is not a canonical citation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CitationError {
    /// The string has no `@` after the repo or document name.
    #[error("no `@` between the name and the version")]
    MissingAt,

    /// The string starts with `@`.
    #[error("nothing before `@`")]
    EmptyName,

    /// The repo or document name has a character citations don't allow.
    #[error("name {found:?} has a character citations don't allow")]
    BadName {
        /// The name as written.
        found: String,
    },

    /// The commit is not 7 to 64 lowercase hex digits.
    #[error("commit {found:?} is not 7 to 64 lowercase hex digits")]
    BadSha {
        /// The commit as written.
        found: String,
    },

    /// A code citation has no `:` after the commit.
    #[error("no `:` between the commit and the path")]
    MissingPath,

    /// A code citation's path is empty.
    #[error("empty path")]
    EmptyPath,

    /// A code citation has no `#L` line range after the path.
    #[error("no `#L` line range after the path")]
    MissingLines,

    /// A number is empty, not decimal, zero, too big or has a leading zero.
    #[error("{what} {found:?} is not a number from 1 without leading zeros")]
    BadNumber {
        /// Which number: `line`, `section` or `step`.
        what: &'static str,
        /// The number as written.
        found: String,
    },

    /// A line range ends before it starts.
    #[error("line range L{start}-L{end} ends before it starts")]
    ReversedRange {
        /// The first line.
        start: u32,
        /// The last line.
        end: u32,
    },

    /// A one-line range is written as a range; the canonical form is `#L<n>`.
    #[error("a single line is written #L{line}, not #L{line}-L{line}")]
    SingleLineRange {
        /// The line.
        line: u32,
    },

    /// The spec content hash is not 12 lowercase hex digits.
    #[error("content hash {found:?} is not 12 lowercase hex digits")]
    BadHash {
        /// The hash as written.
        found: String,
    },

    /// A spec citation has no `#section-` after the hash.
    #[error("no `#section-` after the hash")]
    MissingSection,

    /// A spec citation names a step but no section.
    #[error("a step needs a section: write #section-<n>/step-<k>")]
    StepWithoutSection,

    /// The section is not dot-separated numbers.
    #[error("section {found:?} is not dot-separated numbers like 3.1.3.7")]
    BadSection {
        /// The section as written.
        found: String,
    },
}

/// Parses a canonical number: decimal digits only, no leading zero, at least
/// 1, and small enough for a `u32`.
fn parse_number(s: &str, what: &'static str) -> Result<u32, CitationError> {
    let bad = || CitationError::BadNumber {
        what,
        found: s.into(),
    };
    // The digit check comes first: `u32::from_str` accepts a leading `+`.
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) || s.starts_with('0') {
        return Err(bad());
    }
    s.parse().map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citation_picks_the_code_form() {
        let s = "github.com/panva/openid-client@3f2a9c1:lib/client.js#L412-L455";
        let c: Citation = s.parse().unwrap();
        assert!(matches!(c, Citation::Code(_)));
        assert_eq!(c.to_string(), s);
    }

    #[test]
    fn citation_picks_the_spec_form() {
        let s = "openid-connect-core-1_0@sha256:1a2b3c4d5e6f#section-3.1.3.7/step-13";
        let c: Citation = s.parse().unwrap();
        assert!(matches!(c, Citation::Spec(_)));
        assert_eq!(c.to_string(), s);
    }

    #[test]
    fn citation_without_at_is_an_error() {
        assert_eq!(
            "rfc9449#section-4.3".parse::<Citation>(),
            Err(CitationError::MissingAt)
        );
    }

    #[test]
    fn citation_with_a_bad_hash_reports_the_hash() {
        assert!(matches!(
            "rfc9449@sha256:1a2b#section-4.3".parse::<Citation>(),
            Err(CitationError::BadHash { .. })
        ));
    }
}
