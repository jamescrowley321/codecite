//! Characters that render as nothing, which could hide text from a person
//! reading what a model also reads.

/// Whether `c` renders as nothing: a Unicode format character (category
/// Cf) such as the soft hyphen, zero-width spaces and joiners, bidi marks,
/// embeddings, overrides and isolates, invisible operators, the byte order
/// mark, interlinear annotation marks, the tag characters and the
/// format controls of Arabic, Kaithi, Egyptian hieroglyphs, shorthand and
/// musical notation; a variation selector, Mongolian ones included, whose
/// runs can carry bytes; the combining grapheme joiner; a Hangul filler; or
/// the line and paragraph separators.
pub fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{e0100}'..='\u{e01ef}'
            | '\u{115f}'
            | '\u{1160}'
            | '\u{3164}'
            | '\u{ffa0}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{34f}'
            | '\u{180b}'..='\u{180d}'
            | '\u{180f}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invisible_format_characters_are_found() {
        for c in [
            '\u{200b}',
            '\u{202e}',
            '\u{2066}',
            '\u{e0041}',
            '\u{feff}',
            '\u{ad}',
            '\u{fe0f}',
            '\u{e0100}',
            '\u{3164}',
            '\u{2028}',
            '\u{890}',
            '\u{8e2}',
            '\u{110bd}',
            '\u{110cd}',
            '\u{13435}',
            '\u{1bca1}',
            '\u{1d175}',
            '\u{34f}',
            '\u{180c}',
            '\u{180f}',
        ] {
            assert!(is_invisible_format(c), "{c:?}");
        }
        for c in ['a', 'é', ' ', '\t', '\u{2014}'] {
            assert!(!is_invisible_format(c), "{c:?}");
        }
    }
}
