//! Fetched-document citations: `<doc>@sha256:<12 hex>#L<a>-L<b>`, or `#L<a>`
//! for one line, naming lines of a fetched page's stored text.

use std::fmt;
use std::str::FromStr;

use super::spec::is_doc_name;
use super::{CitationError, ContentHash, LineRange};

/// A span of lines of a fetched document at one content hash:
/// `<doc>@sha256:<hash>#L<a>-L<b>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocumentCitation {
    doc: String,
    hash: ContentHash,
    lines: LineRange,
}

impl DocumentCitation {
    /// The document, such as `notes`.
    pub fn doc(&self) -> &str {
        &self.doc
    }

    /// The content hash of the document version cited.
    pub fn hash(&self) -> &ContentHash {
        &self.hash
    }

    /// The lines cited.
    pub fn lines(&self) -> LineRange {
        self.lines
    }
}

impl FromStr for DocumentCitation {
    type Err = CitationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (doc, rest) = s.split_once('@').ok_or(CitationError::MissingAt)?;
        if doc.is_empty() {
            return Err(CitationError::EmptyName);
        }
        if !is_doc_name(doc) {
            return Err(CitationError::BadName { found: doc.into() });
        }
        let rest = rest
            .strip_prefix("sha256:")
            .ok_or_else(|| CitationError::BadHash { found: rest.into() })?;
        let (hash, lines) = rest.split_once('#').ok_or(CitationError::MissingLines)?;
        Ok(Self {
            doc: doc.into(),
            hash: hash.parse()?,
            lines: lines.parse()?,
        })
    }
}

impl fmt::Display for DocumentCitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@sha256:{}#{}", self.doc, self.hash, self.lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_lines_round_trip() {
        let s = "rfc9449@sha256:1a2b3c4d5e6f#L120-L140";
        let c: DocumentCitation = s.parse().unwrap();
        assert_eq!(c.doc(), "rfc9449");
        assert_eq!(c.hash().as_str(), "1a2b3c4d5e6f");
        assert_eq!((c.lines().start(), c.lines().end()), (120, 140));
        assert_eq!(c.to_string(), s);
    }

    #[test]
    fn document_single_line_round_trips() {
        let s = "notes@sha256:1a2b3c4d5e6f#L7";
        assert_eq!(s.parse::<DocumentCitation>().unwrap().to_string(), s);
    }

    #[test]
    fn non_canonical_lines_are_errors() {
        assert_eq!(
            "notes@sha256:1a2b3c4d5e6f#L10-L10".parse::<DocumentCitation>(),
            Err(CitationError::SingleLineRange { line: 10 })
        );
        assert_eq!(
            "notes@sha256:1a2b3c4d5e6f#L10-L4".parse::<DocumentCitation>(),
            Err(CitationError::ReversedRange { start: 10, end: 4 })
        );
    }

    #[test]
    fn bad_pieces_are_errors() {
        assert_eq!(
            "@sha256:1a2b3c4d5e6f#L1".parse::<DocumentCitation>(),
            Err(CitationError::EmptyName)
        );
        assert!(matches!(
            "my notes@sha256:1a2b3c4d5e6f#L1".parse::<DocumentCitation>(),
            Err(CitationError::BadName { .. })
        ));
        assert!(matches!(
            "notes@sha256:1a2b#L1".parse::<DocumentCitation>(),
            Err(CitationError::BadHash { .. })
        ));
        assert!(matches!(
            "notes@3f2a9c1#L1".parse::<DocumentCitation>(),
            Err(CitationError::BadHash { .. })
        ));
        assert_eq!(
            "notes@sha256:1a2b3c4d5e6f".parse::<DocumentCitation>(),
            Err(CitationError::MissingLines)
        );
        assert_eq!(
            "notes@sha256:1a2b3c4d5e6f#install".parse::<DocumentCitation>(),
            Err(CitationError::MissingLines)
        );
    }
}
