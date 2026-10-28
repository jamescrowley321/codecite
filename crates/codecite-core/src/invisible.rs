//! Characters that render as nothing, which could hide text from a person
//! reading what a model also reads.

/// Whether `c` is a Unicode format character (category Cf) that renders
/// as nothing: the soft hyphen, zero-width spaces and joiners, bidi marks,
/// embeddings, overrides and isolates, invisible operators, the byte order
/// mark, interlinear annotation marks and the tag characters.
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
        ] {
            assert!(is_invisible_format(c), "{c:?}");
        }
        for c in ['a', 'é', ' ', '\t', '\u{2014}'] {
            assert!(!is_invisible_format(c), "{c:?}");
        }
    }
}
