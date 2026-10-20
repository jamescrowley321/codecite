//! Code citations: `<repo>@<sha>:<path>#L<a>-L<b>`, or `#L<a>` for one line.

use std::fmt;
use std::str::FromStr;

use super::{CitationError, parse_number};

/// A git commit, 7 to 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitSha(String);

impl CommitSha {
    /// The commit as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CommitSha {
    type Err = CitationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Count chars, not bytes, so `é` fails the hex check instead of
        // passing a length check.
        let n = s.chars().count();
        if (7..=64).contains(&n) && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
            Ok(Self(s.into()))
        } else {
            Err(CitationError::BadSha { found: s.into() })
        }
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A span of lines, `L<a>-L<b>`, or `L<a>` for one line. Lines start at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineRange {
    start: u32,
    end: u32,
}

impl LineRange {
    /// The first line.
    pub fn start(&self) -> u32 {
        self.start
    }

    /// The last line, equal to [`start`](Self::start) for one line.
    pub fn end(&self) -> u32 {
        self.end
    }
}

impl FromStr for LineRange {
    type Err = CitationError;

    /// Parses the text after `#`, such as `L412-L455`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.strip_prefix('L').ok_or(CitationError::MissingLines)?;
        let Some((a, b)) = rest.split_once('-') else {
            let line = parse_number(rest, "line")?;
            return Ok(Self {
                start: line,
                end: line,
            });
        };
        let start = parse_number(a, "line")?;
        let b = b
            .strip_prefix('L')
            .ok_or_else(|| CitationError::BadNumber {
                what: "line",
                found: b.into(),
            })?;
        let end = parse_number(b, "line")?;
        if end < start {
            Err(CitationError::ReversedRange { start, end })
        } else if end == start {
            Err(CitationError::SingleLineRange { line: start })
        } else {
            Ok(Self { start, end })
        }
    }
}

impl fmt::Display for LineRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "L{}", self.start)
        } else {
            write!(f, "L{}-L{}", self.start, self.end)
        }
    }
}

/// A span of code at one commit: `<repo>@<sha>:<path>#L<a>-L<b>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodeCitation {
    repo: String,
    commit: CommitSha,
    path: String,
    lines: LineRange,
}

impl CodeCitation {
    /// The repo, such as `github.com/panva/openid-client`.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// The commit the span was read at.
    pub fn commit(&self) -> &CommitSha {
        &self.commit
    }

    /// The file's path inside the repo.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The lines cited.
    pub fn lines(&self) -> LineRange {
        self.lines
    }
}

impl FromStr for CodeCitation {
    type Err = CitationError;

    /// The repo runs to the first `@`, the commit to the next `:`, and the
    /// path to the last `#`, so a path may contain `@`, `:` and `#`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (repo, rest) = s.split_once('@').ok_or(CitationError::MissingAt)?;
        if repo.is_empty() {
            return Err(CitationError::EmptyName);
        }
        if repo.chars().any(char::is_whitespace) {
            return Err(CitationError::BadName { found: repo.into() });
        }
        let (commit, tail) = rest.split_once(':').ok_or(CitationError::MissingPath)?;
        let commit = commit.parse()?;
        let (path, lines) = tail.rsplit_once('#').ok_or(CitationError::MissingLines)?;
        if path.is_empty() {
            return Err(CitationError::EmptyPath);
        }
        Ok(Self {
            repo: repo.into(),
            commit,
            path: path.into(),
            lines: lines.parse()?,
        })
    }
}

impl fmt::Display for CodeCitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{}:{}#{}",
            self.repo, self.commit, self.path, self.lines
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CitationError;

    const EXAMPLE: &str = "github.com/panva/openid-client@3f2a9c1:lib/client.js#L412-L455";

    #[test]
    fn code_range_round_trips() {
        let c: CodeCitation = EXAMPLE.parse().unwrap();
        assert_eq!(c.repo(), "github.com/panva/openid-client");
        assert_eq!(c.commit().as_str(), "3f2a9c1");
        assert_eq!(c.path(), "lib/client.js");
        assert_eq!((c.lines().start(), c.lines().end()), (412, 455));
        assert_eq!(c.to_string(), EXAMPLE);
    }

    #[test]
    fn code_single_line_round_trips() {
        let s = "github.com/panva/openid-client@3f2a9c1:lib/client.js#L412";
        let c: CodeCitation = s.parse().unwrap();
        assert_eq!((c.lines().start(), c.lines().end()), (412, 412));
        assert_eq!(c.to_string(), s);
    }

    #[test]
    fn full_sha256_commit_round_trips() {
        let s = format!("r@{}:a.rs#L1", "a".repeat(64));
        assert_eq!(s.parse::<CodeCitation>().unwrap().to_string(), s);
    }

    #[test]
    fn path_may_contain_hash_at_and_colon() {
        let s = "r@3f2a9c1:node_modules/@scope/c#/a:b.js#L3-L4";
        let c: CodeCitation = s.parse().unwrap();
        assert_eq!(c.path(), "node_modules/@scope/c#/a:b.js");
        assert_eq!(c.to_string(), s);
    }

    #[test]
    fn missing_at_is_an_error() {
        assert_eq!(
            "r3f2a9c1:a.rs#L1".parse::<CodeCitation>(),
            Err(CitationError::MissingAt)
        );
    }

    #[test]
    fn reversed_range_is_an_error() {
        assert_eq!(
            "r@3f2a9c1:a.rs#L10-L4".parse::<CodeCitation>(),
            Err(CitationError::ReversedRange { start: 10, end: 4 })
        );
    }

    #[test]
    fn bad_sha_is_an_error() {
        for sha in ["3f2a9c", "3f2a9cg", "a".repeat(65).as_str()] {
            let s = format!("r@{sha}:a.rs#L1");
            assert!(
                matches!(s.parse::<CodeCitation>(), Err(CitationError::BadSha { .. })),
                "{s}"
            );
        }
    }

    #[test]
    fn multibyte_sha_is_an_error_not_a_panic() {
        assert!(matches!(
            "r@3f2a9cé:a.rs#L1".parse::<CodeCitation>(),
            Err(CitationError::BadSha { .. })
        ));
    }

    #[test]
    fn line_number_overflow_is_an_error() {
        assert!(matches!(
            "r@3f2a9c1:a.rs#L99999999999".parse::<CodeCitation>(),
            Err(CitationError::BadNumber { what: "line", .. })
        ));
    }

    #[test]
    fn non_canonical_spellings_are_rejected() {
        assert_eq!(
            "r@3f2a9c1:a.rs#L10-L10".parse::<CodeCitation>(),
            Err(CitationError::SingleLineRange { line: 10 })
        );
        for s in [
            "r@3F2A9C1:a.rs#L1",
            "r@3f2a9c1:a.rs#L010",
            "r@3f2a9c1:a.rs#L0",
            "r@3f2a9c1:a.rs#L+1",
        ] {
            assert!(s.parse::<CodeCitation>().is_err(), "{s}");
        }
    }

    #[test]
    fn empty_pieces_are_errors() {
        assert_eq!(
            "@3f2a9c1:a.rs#L1".parse::<CodeCitation>(),
            Err(CitationError::EmptyName)
        );
        assert_eq!(
            "r@3f2a9c1:#L1".parse::<CodeCitation>(),
            Err(CitationError::EmptyPath)
        );
        assert_eq!(
            "r@3f2a9c1".parse::<CodeCitation>(),
            Err(CitationError::MissingPath)
        );
        assert_eq!(
            "r@3f2a9c1:a.rs".parse::<CodeCitation>(),
            Err(CitationError::MissingLines)
        );
        assert!(matches!(
            "r@3f2a9c1:a.rs#L".parse::<CodeCitation>(),
            Err(CitationError::BadNumber { .. })
        ));
    }

    #[test]
    fn trailing_junk_is_an_error() {
        for s in [
            "r@3f2a9c1:a.rs#L4-L9x",
            "r@3f2a9c1:a.rs#L4-9",
            "r@3f2a9c1:a.rs#4-L9",
            "r@3f2a9c1:a.rs#L4-L9-L12",
        ] {
            assert!(s.parse::<CodeCitation>().is_err(), "{s}");
        }
    }

    #[test]
    fn repo_with_whitespace_is_an_error() {
        assert!(matches!(
            "my repo@3f2a9c1:a.rs#L1".parse::<CodeCitation>(),
            Err(CitationError::BadName { .. })
        ));
    }

    #[test]
    fn error_display_names_the_problem() {
        assert_eq!(
            CitationError::ReversedRange { start: 10, end: 4 }.to_string(),
            "line range L10-L4 ends before it starts"
        );
    }
}
