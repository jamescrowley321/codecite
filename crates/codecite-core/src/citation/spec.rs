//! Spec citations: `<doc>@sha256:<12 hex>#section-<n>[/step-<k>]`.

use std::fmt;
use std::str::FromStr;

use super::{CitationError, parse_number};

/// The first 12 lowercase hex digits of a spec document's SHA-256.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(String);

impl ContentHash {
    /// The hash as written, without the `sha256:` prefix.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ContentHash {
    type Err = CitationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Count chars, not bytes, as `CommitSha` does.
        if s.chars().count() == 12 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
            Ok(Self(s.into()))
        } else {
            Err(CitationError::BadHash { found: s.into() })
        }
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A section number such as `3.1.3.7`: one or more numbers joined by `.`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SectionNumber(Vec<u32>);

impl SectionNumber {
    /// The numbers, outermost first: `[3, 1, 3, 7]` for `3.1.3.7`.
    pub fn parts(&self) -> &[u32] {
        &self.0
    }
}

impl FromStr for SectionNumber {
    type Err = CitationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split('.')
            .map(|part| parse_number(part, "section"))
            .collect::<Result<Vec<u32>, _>>()
            .map(Self)
            .map_err(|_| CitationError::BadSection { found: s.into() })
    }
}

impl fmt::Display for SectionNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = self.0.iter();
        if let Some(first) = parts.next() {
            write!(f, "{first}")?;
        }
        for part in parts {
            write!(f, ".{part}")?;
        }
        Ok(())
    }
}

/// A section, or one step inside it, of a spec document at one content hash:
/// `<doc>@sha256:<hash>#section-<n>[/step-<k>]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpecCitation {
    doc: String,
    hash: ContentHash,
    section: SectionNumber,
    step: Option<u32>,
}

impl SpecCitation {
    /// The document, such as `rfc9449`.
    pub fn doc(&self) -> &str {
        &self.doc
    }

    /// The content hash of the document version cited.
    pub fn hash(&self) -> &ContentHash {
        &self.hash
    }

    /// The section cited.
    pub fn section(&self) -> &SectionNumber {
        &self.section
    }

    /// The step inside the section, if the citation names one.
    pub fn step(&self) -> Option<u32> {
        self.step
    }
}

impl FromStr for SpecCitation {
    type Err = CitationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (doc, rest) = s.split_once('@').ok_or(CitationError::MissingAt)?;
        if doc.is_empty() {
            return Err(CitationError::EmptyName);
        }
        if !doc
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(CitationError::BadName { found: doc.into() });
        }
        let rest = rest
            .strip_prefix("sha256:")
            .ok_or_else(|| CitationError::BadHash { found: rest.into() })?;
        let (hash, fragment) = rest.split_once('#').ok_or(CitationError::MissingSection)?;
        let hash = hash.parse()?;
        if fragment.starts_with("step-") {
            return Err(CitationError::StepWithoutSection);
        }
        let fragment =
            fragment
                .strip_prefix("section-")
                .ok_or_else(|| CitationError::BadSection {
                    found: fragment.into(),
                })?;
        // A leftover `/` (`4.3/`, `4.3/x`) stays in the section and fails its
        // digit check.
        let (section, step) = match fragment.split_once("/step-") {
            Some((section, step)) => (section, Some(parse_number(step, "step")?)),
            None => (fragment, None),
        };
        Ok(Self {
            doc: doc.into(),
            hash,
            section: section.parse()?,
            step,
        })
    }
}

impl fmt::Display for SpecCitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@sha256:{}#section-{}",
            self.doc, self.hash, self.section
        )?;
        if let Some(k) = self.step {
            write!(f, "/step-{k}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CitationError;

    #[test]
    fn spec_section_round_trips() {
        let s = "rfc9449@sha256:1a2b3c4d5e6f#section-4.3";
        let c: SpecCitation = s.parse().unwrap();
        assert_eq!(c.doc(), "rfc9449");
        assert_eq!(c.hash().as_str(), "1a2b3c4d5e6f");
        assert_eq!(c.section().parts(), &[4, 3]);
        assert_eq!(c.step(), None);
        assert_eq!(c.to_string(), s);
    }

    #[test]
    fn spec_step_round_trips() {
        let s = "openid-connect-core-1_0@sha256:1a2b3c4d5e6f#section-3.1.3.7/step-13";
        let c: SpecCitation = s.parse().unwrap();
        assert_eq!(c.doc(), "openid-connect-core-1_0");
        assert_eq!(c.section().parts(), &[3, 1, 3, 7]);
        assert_eq!(c.step(), Some(13));
        assert_eq!(c.to_string(), s);
    }

    #[test]
    fn bad_hash_length_is_an_error() {
        for h in ["1a2b3c4d5e6", "1a2b3c4d5e6f0", ""] {
            let s = format!("rfc9449@sha256:{h}#section-1");
            assert!(
                matches!(
                    s.parse::<SpecCitation>(),
                    Err(CitationError::BadHash { .. })
                ),
                "{s}"
            );
        }
    }

    #[test]
    fn multibyte_hash_is_an_error_not_a_panic() {
        assert!(matches!(
            "rfc9449@sha256:1a2b3c4d5eé#section-1".parse::<SpecCitation>(),
            Err(CitationError::BadHash { .. })
        ));
    }

    #[test]
    fn step_without_section_is_an_error() {
        assert_eq!(
            "rfc9449@sha256:1a2b3c4d5e6f#step-13".parse::<SpecCitation>(),
            Err(CitationError::StepWithoutSection)
        );
    }

    #[test]
    fn section_component_overflow_is_an_error() {
        assert!(matches!(
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.99999999999".parse::<SpecCitation>(),
            Err(CitationError::BadSection { .. })
        ));
    }

    #[test]
    fn non_canonical_spellings_are_rejected() {
        for s in [
            "rfc9449@sha256:1A2B3C4D5E6F#section-4.3",
            "rfc9449@sha256:1a2b3c4d5e6f#section-04.3",
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-013",
            "rfc9449@sha256:1a2b3c4d5e6f#Section-4.3",
        ] {
            assert!(s.parse::<SpecCitation>().is_err(), "{s}");
        }
    }

    #[test]
    fn empty_pieces_are_errors() {
        for (s, what) in [
            ("rfc9449@sha256:1a2b3c4d5e6f#section-", "section"),
            ("rfc9449@sha256:1a2b3c4d5e6f#section-4..3", "section"),
            ("rfc9449@sha256:1a2b3c4d5e6f#section-4.", "section"),
            ("rfc9449@sha256:1a2b3c4d5e6f#section-4/step-", "step"),
        ] {
            let err = s.parse::<SpecCitation>().unwrap_err();
            assert!(err.to_string().contains(what), "{s}: {err}");
        }
        assert_eq!(
            "@sha256:1a2b3c4d5e6f#section-1".parse::<SpecCitation>(),
            Err(CitationError::EmptyName)
        );
        assert_eq!(
            "rfc9449@sha256:1a2b3c4d5e6f".parse::<SpecCitation>(),
            Err(CitationError::MissingSection)
        );
    }

    #[test]
    fn trailing_junk_is_an_error() {
        for s in [
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-13/x",
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3x",
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/",
        ] {
            assert!(s.parse::<SpecCitation>().is_err(), "{s}");
        }
    }

    #[test]
    fn doc_name_with_a_slash_is_an_error() {
        assert!(matches!(
            "ietf/rfc9449@sha256:1a2b3c4d5e6f#section-1".parse::<SpecCitation>(),
            Err(CitationError::BadName { .. })
        ));
    }
}
