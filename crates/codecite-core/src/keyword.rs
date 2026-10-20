//! Requirement keywords: the uppercase BCP 14 words (RFC 2119, RFC 8174)
//! that say how strongly a spec requires something.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// One BCP 14 requirement keyword. Two-word keywords such as `MUST NOT`
/// are one keyword, not `MUST` plus `NOT`.
///
/// The variants are in the order RFC 2119 lists them, and sort that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Keyword {
    /// `MUST`
    Must,
    /// `MUST NOT`
    MustNot,
    /// `REQUIRED`
    Required,
    /// `SHALL`
    Shall,
    /// `SHALL NOT`
    ShallNot,
    /// `SHOULD`
    Should,
    /// `SHOULD NOT`
    ShouldNot,
    /// `RECOMMENDED`
    Recommended,
    /// `NOT RECOMMENDED`
    NotRecommended,
    /// `MAY`
    May,
    /// `OPTIONAL`
    Optional,
}

impl Keyword {
    /// Every keyword, in order.
    pub const ALL: [Keyword; 11] = [
        Self::Must,
        Self::MustNot,
        Self::Required,
        Self::Shall,
        Self::ShallNot,
        Self::Should,
        Self::ShouldNot,
        Self::Recommended,
        Self::NotRecommended,
        Self::May,
        Self::Optional,
    ];

    /// The keyword as a spec writes it: `MUST NOT`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Must => "MUST",
            Self::MustNot => "MUST NOT",
            Self::Required => "REQUIRED",
            Self::Shall => "SHALL",
            Self::ShallNot => "SHALL NOT",
            Self::Should => "SHOULD",
            Self::ShouldNot => "SHOULD NOT",
            Self::Recommended => "RECOMMENDED",
            Self::NotRecommended => "NOT RECOMMENDED",
            Self::May => "MAY",
            Self::Optional => "OPTIONAL",
        }
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A string that is not one of the eleven keywords.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a BCP 14 requirement keyword: {found:?}")]
pub struct UnknownKeyword {
    /// The string that was parsed.
    pub found: String,
}

impl FromStr for Keyword {
    type Err = UnknownKeyword;

    /// Parses a keyword exactly as [`Keyword::as_str`] writes it: uppercase,
    /// with one space in the two-word keywords.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| UnknownKeyword { found: s.into() })
    }
}

/// The requirement keywords in `text`, each once, in [`Keyword`] order.
///
/// Only whole uppercase words count: `must` and `Must` are ordinary
/// English (RFC 8174), and so is the `MUST` inside `MUSTARD`. `MUST`
/// followed by `NOT` across nothing but whitespace, line breaks included,
/// is `MUST NOT`; so are `SHALL NOT`, `SHOULD NOT` and `NOT RECOMMENDED`.
/// A `NOT` on its own is not a keyword.
pub fn requirement_keywords(text: &str) -> Vec<Keyword> {
    let words = word_spans(text);
    let mut found = BTreeSet::new();
    let mut i = 0;
    while i < words.len() {
        let (start, end) = words[i];
        let word = &text[start..end];
        let next = words
            .get(i + 1)
            .filter(|&&(next_start, _)| text[end..next_start].chars().all(char::is_whitespace))
            .map(|&(s, e)| &text[s..e]);
        let (keyword, used) = match (word, next) {
            ("MUST", Some("NOT")) => (Some(Keyword::MustNot), 2),
            ("SHALL", Some("NOT")) => (Some(Keyword::ShallNot), 2),
            ("SHOULD", Some("NOT")) => (Some(Keyword::ShouldNot), 2),
            ("NOT", Some("RECOMMENDED")) => (Some(Keyword::NotRecommended), 2),
            // A single word has no space, so it never parses as a
            // two-word keyword.
            (word, _) => (word.parse().ok(), 1),
        };
        found.extend(keyword);
        i += used;
    }
    found.into_iter().collect()
}

/// The byte ranges of the words in `text`: maximal runs of letters,
/// digits and underscores.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let in_word = c.is_alphanumeric() || c == '_';
        match (in_word, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use Keyword::*;
    use proptest::prelude::*;

    #[test]
    fn every_keyword_parses_back_from_its_string() {
        for k in Keyword::ALL {
            assert_eq!(k.as_str().parse(), Ok(k));
            assert_eq!(k.to_string(), k.as_str());
            assert_eq!(requirement_keywords(k.as_str()), [k], "{k}");
        }
        assert_eq!(
            "must".parse::<Keyword>(),
            Err(UnknownKeyword {
                found: "must".into()
            })
        );
        assert!("MUST  NOT".parse::<Keyword>().is_err());
        assert!("NOT".parse::<Keyword>().is_err());
    }

    #[test]
    fn finds_the_keywords_as_written() {
        assert_eq!(
            requirement_keywords(
                "The client MUST validate it, SHOULD log it and MAY retry. \
                 It MUST NOT be reused."
            ),
            [Must, MustNot, Should, May]
        );
        assert_eq!(
            requirement_keywords("This is NOT RECOMMENDED; it is OPTIONAL."),
            [NotRecommended, Optional]
        );
        assert_eq!(
            requirement_keywords("It SHALL NOT, it SHALL."),
            [Shall, ShallNot]
        );
        assert_eq!(
            requirement_keywords("REQUIRED. RECOMMENDED. SHOULD NOT."),
            [Required, ShouldNot, Recommended]
        );
    }

    #[test]
    fn two_word_keywords_span_line_breaks_but_not_punctuation() {
        assert_eq!(requirement_keywords("MUST\n   NOT"), [MustNot]);
        assert_eq!(requirement_keywords("MUST. NOT"), [Must]);
        assert_eq!(requirement_keywords("MUST-NOT"), [Must]);
        assert_eq!(requirement_keywords("NOT"), []);
        assert_eq!(requirement_keywords("NOT REQUIRED"), [Required]);
        // `MUST NOT` takes the `NOT`, leaving `RECOMMENDED` alone.
        assert_eq!(
            requirement_keywords("MUST NOT RECOMMENDED"),
            [MustNot, Recommended]
        );
    }

    #[test]
    fn lowercase_and_partial_words_are_not_keywords() {
        assert_eq!(requirement_keywords("the client must not"), []);
        assert_eq!(requirement_keywords("Must May Should"), []);
        assert_eq!(
            requirement_keywords("MUSTARD MAYBE _MUST MUST1 SHOULDN'T"),
            []
        );
        assert_eq!(requirement_keywords(""), []);
    }

    proptest! {
        #[test]
        fn lowercase_text_has_no_keywords(text in "\\PC*") {
            prop_assert!(requirement_keywords(&text.to_lowercase()).is_empty());
        }

        #[test]
        fn any_text_scans_without_panicking(text in any::<String>()) {
            let found = requirement_keywords(&text);
            // Each keyword at most once, in order.
            prop_assert!(found.windows(2).all(|w| w[0] < w[1]));
        }

        #[test]
        fn a_keyword_among_lowercase_words_is_found(
            before in "[a-z ,.]{0,30}",
            k in prop::sample::select(Keyword::ALL.to_vec()),
            after in "[a-z ,.]{0,30}",
        ) {
            let text = format!("{before} {k} {after}");
            prop_assert_eq!(requirement_keywords(&text), [k]);
        }

        #[test]
        fn keywords_joined_by_punctuation_are_all_found(
            ks in prop::collection::btree_set(prop::sample::select(Keyword::ALL.to_vec()), 0..11),
        ) {
            let text = ks.iter().map(|k| k.as_str()).collect::<Vec<_>>().join("; ");
            prop_assert_eq!(requirement_keywords(&text), ks.into_iter().collect::<Vec<_>>());
        }
    }
}
