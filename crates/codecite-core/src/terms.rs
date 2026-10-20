//! Search terms for identifiers: split camelCase, snake_case and
//! kebab-case names into lowercase words, and keep the original so an
//! exact name still matches.

/// Splits an identifier into its lowercase parts.
///
/// Characters that are not letters or digits separate parts. An uppercase
/// letter starts a new part after a non-uppercase character
/// (`validateToken`) or at the end of an acronym (`HTTPServer`). Digits
/// join the part before them (`sha256`).
pub fn split_identifier(identifier: &str) -> Vec<String> {
    let chars: Vec<char> = identifier.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            flush(&mut current, &mut parts);
            continue;
        }
        // `current` is non-empty only when the previous char was pushed
        // into it, so `chars[i - 1]` is a letter or digit here.
        if c.is_uppercase() && !current.is_empty() {
            let after_lower = !chars[i - 1].is_uppercase();
            let ends_acronym = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if after_lower || ends_acronym {
                flush(&mut current, &mut parts);
            }
        }
        current.push(c);
    }
    flush(&mut current, &mut parts);
    parts
}

/// Returns the search terms for an identifier: its lowercase parts, then
/// the identifier itself, separated by spaces.
///
/// `validateToken` gives `validate token validateToken`, so a search for
/// "validate token" and a search for the exact name both match. An
/// identifier that is already a single lowercase part is returned alone.
pub fn identifier_terms(identifier: &str) -> String {
    let parts = split_identifier(identifier);
    if let [only] = parts.as_slice()
        && only == identifier
    {
        return identifier.to_owned();
    }
    let mut terms = parts.join(" ");
    if !terms.is_empty() && !identifier.is_empty() {
        terms.push(' ');
    }
    terms.push_str(identifier);
    terms
}

/// Moves a finished part, lowercased, into `parts`.
fn flush(current: &mut String, parts: &mut Vec<String>) {
    if !current.is_empty() {
        parts.push(current.to_lowercase());
        current.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(s: &str) -> Vec<String> {
        split_identifier(s)
    }

    #[test]
    fn camel_case_splits_at_each_capital() {
        assert_eq!(parts("validateToken"), ["validate", "token"]);
        assert_eq!(parts("ValidateIdToken"), ["validate", "id", "token"]);
    }

    #[test]
    fn snake_and_kebab_case_split_at_separators() {
        assert_eq!(parts("id_token_hint"), ["id", "token", "hint"]);
        assert_eq!(parts("back-channel"), ["back", "channel"]);
        assert_eq!(parts("__init__"), ["init"]);
        assert_eq!(parts("a--b"), ["a", "b"]);
    }

    #[test]
    fn acronym_then_word_splits_before_the_word() {
        assert_eq!(parts("HTTPServer"), ["http", "server"]);
        assert_eq!(parts("parseHTTPResponse"), ["parse", "http", "response"]);
        assert_eq!(parts("JWT"), ["jwt"]);
        assert_eq!(parts("MAX_AGE"), ["max", "age"]);
    }

    #[test]
    fn digits_stay_attached() {
        assert_eq!(parts("sha256"), ["sha256"]);
        assert_eq!(parts("rfc9449"), ["rfc9449"]);
        assert_eq!(parts("sha256Hash"), ["sha256", "hash"]);
        assert_eq!(parts("HTTP2Server"), ["http2", "server"]);
        assert_eq!(parts("2fa"), ["2fa"]);
    }

    #[test]
    fn non_ascii_identifiers_split_without_panicking() {
        assert_eq!(parts("ÜberMaß"), ["über", "maß"]);
        assert_eq!(parts("größeBerechnen"), ["größe", "berechnen"]);
        assert_eq!(parts("名前Value"), ["名前", "value"]);
        assert_eq!(parts("é"), ["é"]);
    }

    #[test]
    fn empty_input_has_no_parts() {
        assert!(parts("").is_empty());
        assert!(parts("__").is_empty());
        assert!(parts("-").is_empty());
    }

    #[test]
    fn lone_capital_before_lowercase_splits_off() {
        assert_eq!(parts("IPv4Address"), ["i", "pv4", "address"]);
    }

    #[test]
    fn terms_match_the_issue_examples() {
        assert_eq!(
            identifier_terms("validateToken"),
            "validate token validateToken"
        );
        assert_eq!(
            identifier_terms("id_token_hint"),
            "id token hint id_token_hint"
        );
        assert_eq!(
            identifier_terms("back-channel"),
            "back channel back-channel"
        );
        assert_eq!(identifier_terms("HTTPServer"), "http server HTTPServer");
    }

    #[test]
    fn one_part_identifier_is_not_repeated() {
        assert_eq!(identifier_terms("token"), "token");
        assert_eq!(identifier_terms("sha256"), "sha256");
        assert_eq!(identifier_terms("rfc9449"), "rfc9449");
        assert_eq!(identifier_terms("Token"), "token Token");
    }

    #[test]
    fn non_ascii_terms_keep_the_original() {
        assert_eq!(identifier_terms("ÜberMaß"), "über maß ÜberMaß");
    }

    #[test]
    fn empty_input_gives_no_terms() {
        assert_eq!(identifier_terms(""), "");
        assert_eq!(identifier_terms("__"), "__");
    }
}
