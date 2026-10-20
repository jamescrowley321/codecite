//! Canonical citations: the exact strings codecite hands to agents to name a
//! span of code or a spec section.
//!
//! Parsing is strict. Every string the parser accepts formats back to exactly
//! the same string, so each span has one spelling.

mod code;
pub use code::{CodeCitation, CommitSha, LineRange};

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
