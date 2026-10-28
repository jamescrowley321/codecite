//! Normative statements: the sentences and claim or parameter table rows
//! of a spec section that require something, found in its stored text.

use crate::{Keyword, requirement_keywords};

/// How strongly a statement requires something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// `MUST`, `MUST NOT`, `REQUIRED`, `SHALL`, `SHALL NOT`, or a
    /// `PROHIBITED` table row.
    Must,
    /// `SHOULD`, `SHOULD NOT`, `RECOMMENDED` or `NOT RECOMMENDED`, which
    /// BCP 14 defines as `SHOULD` and `SHOULD NOT`.
    Should,
}

/// The strongest level among `keywords`, or `None` when they only permit
/// (`MAY`, `OPTIONAL`) or there are none.
pub fn normative_level(keywords: &[Keyword]) -> Option<Level> {
    use Keyword::*;
    let must = |k: &Keyword| matches!(k, Must | MustNot | Required | Shall | ShallNot);
    let should = |k: &Keyword| matches!(k, Should | ShouldNot | Recommended | NotRecommended);
    if keywords.iter().any(must) {
        Some(Level::Must)
    } else if keywords.iter().any(should) {
        Some(Level::Should)
    } else {
        None
    }
}

/// What a statement is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementKind {
    /// A sentence of running text.
    Sentence,
    /// A row of a claim or parameter list: a term and the value that says
    /// whether it is `REQUIRED`, `OPTIONAL` or `PROHIBITED`.
    Row,
}

/// One normative statement of a spec section's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormativeStatement {
    /// A sentence or a table row.
    pub kind: StatementKind,
    /// The line of the text, counting from 0, where the statement's
    /// paragraph or row starts.
    pub line: usize,
    /// Its requirement keywords, in [`Keyword`] order.
    pub keywords: Vec<Keyword>,
    /// Whether it is a row whose value starts `PROHIBITED`, a word BCP 14
    /// doesn't define but claim lists use for `MUST NOT be present`.
    pub prohibited: bool,
    /// The statement, with each run of whitespace one space. A row is
    /// `term: value`, as in `exp: REQUIRED. Expiration time, …`.
    pub text: String,
}

impl NormativeStatement {
    /// How strongly it requires: always `Some` for a statement
    /// [`normative_statements`] returns.
    pub fn level(&self) -> Level {
        if self.prohibited {
            return Level::Must;
        }
        normative_level(&self.keywords).unwrap_or(Level::Should)
    }
}

/// The normative statements of a spec section's stored text, in order:
/// every sentence with a `MUST`-level or `SHOULD`-level keyword (see
/// [`Level`]), and every claim or parameter row whose value starts
/// `REQUIRED` or `PROHIBITED` or holds such a keyword.
///
/// The text is split into paragraphs at blank lines. Consecutive indented
/// lines are one wrapped paragraph, as in the older RFC layout; an
/// unindented line is a paragraph of its own, as in the OpenID and xml2rfc
/// v3 layouts, which render a paragraph per line. A row is a one-word term
/// line followed by a line starting `REQUIRED.`, `OPTIONAL.`,
/// `RECOMMENDED.`, `NOT RECOMMENDED.` or `PROHIBITED.`, indented at least
/// as deep; the value runs on over lines indented as deep as it and deeper
/// than the term. A sentence ends at `.`, `?` or `!` before whitespace and
/// an uppercase letter, digit, quote or bracket, but not after an initial
/// (`C.`) or `e.g.`, `i.e.`, `etc.`, `cf.`, `Ed.` or `vs.`.
///
/// It needs only the text, so a caller can blank lines it handles itself,
/// such as a section's steps, and keep the line numbers.
pub fn normative_statements(text: &str) -> Vec<NormativeStatement> {
    let lines: Vec<&str> = text.lines().collect();
    let blank = |i: usize| lines[i].trim().is_empty();
    let indent = |i: usize| lines[i].len() - lines[i].trim_start().len();
    let starts_row = |i: usize| {
        i + 1 < lines.len()
            && is_term(lines[i])
            && row_lead(lines[i + 1]).is_some()
            && indent(i + 1) >= indent(i)
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if blank(i) {
            i += 1;
            continue;
        }
        let start = i;
        if starts_row(i) {
            let term = lines[i].trim();
            let value_indent = indent(i + 1);
            let lead = row_lead(lines[i + 1]);
            let mut value = vec![lines[i + 1]];
            i += 2;
            while i < lines.len()
                && !blank(i)
                && indent(i) >= value_indent
                && indent(i) > indent(start)
            {
                value.push(lines[i]);
                i += 1;
            }
            let text = format!("{term}: {}", collapse(&value.join(" ")));
            let keywords = requirement_keywords(&text);
            let prohibited = lead == Some("PROHIBITED");
            if prohibited || lead == Some("REQUIRED") || normative_level(&keywords).is_some() {
                found.push(NormativeStatement {
                    kind: StatementKind::Row,
                    line: start,
                    keywords,
                    prohibited,
                    text,
                });
            }
            continue;
        }
        let mut paragraph = vec![lines[i]];
        i += 1;
        if indent(start) > 0 {
            while i < lines.len() && !blank(i) && indent(i) > 0 && !starts_row(i) {
                paragraph.push(lines[i]);
                i += 1;
            }
        }
        for sentence in sentences(&collapse(&paragraph.join(" "))) {
            let keywords = requirement_keywords(sentence);
            if normative_level(&keywords).is_some() {
                found.push(NormativeStatement {
                    kind: StatementKind::Sentence,
                    line: start,
                    keywords,
                    prohibited: false,
                    text: sentence.to_owned(),
                });
            }
        }
    }
    found
}

/// The words a row's value starts with, longest first.
const ROW_LEADS: [&str; 5] = [
    "NOT RECOMMENDED",
    "RECOMMENDED",
    "REQUIRED",
    "OPTIONAL",
    "PROHIBITED",
];

/// Which of [`ROW_LEADS`] `line` starts with, followed by `.`, `,`, `;`
/// or nothing.
fn row_lead(line: &str) -> Option<&'static str> {
    let line = line.trim();
    ROW_LEADS.into_iter().find(|lead| {
        line.strip_prefix(lead)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', ',', ';']))
    })
}

/// Whether `line` can be a row's term: one word of at most 80 bytes.
fn is_term(line: &str) -> bool {
    let term = line.trim();
    !term.is_empty() && term.len() <= 80 && !term.contains(char::is_whitespace)
}

/// `text` with each run of whitespace one space, and none at the ends.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Words that end in `.` without ending a sentence.
const ABBREVIATIONS: [&str; 6] = ["e.g", "i.e", "etc", "cf", "Ed", "vs"];

/// The sentences of one paragraph, whitespace already collapsed.
fn sentences(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        if !matches!(c, '.' | '?' | '!') || bytes.get(i + 1) != Some(&b' ') {
            continue;
        }
        let next = text[i + 2..].chars().next();
        let opens = next.is_some_and(|n| {
            n.is_uppercase()
                || n.is_ascii_digit()
                || matches!(n, '"' | '\'' | '“' | '‘' | '[' | '(')
        });
        if !opens || (c == '.' && !ends_sentence(&text[start..i])) {
            continue;
        }
        found.push(&text[start..=i]);
        start = i + 2;
    }
    if start < text.len() {
        found.push(&text[start..]);
    }
    found
}

/// Whether a `.` after `before` ends a sentence: not after an initial or
/// one of [`ABBREVIATIONS`].
fn ends_sentence(before: &str) -> bool {
    let word = before
        .rsplit(' ')
        .next()
        .unwrap_or_default()
        .trim_start_matches(|c: char| !c.is_alphanumeric());
    let initial = word.chars().count() == 1 && word.chars().all(char::is_uppercase);
    !initial && !ABBREVIATIONS.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Keyword;

    /// Back-Channel Logout 1.0 §2.4 as the OpenID layout renders it,
    /// abridged: one line per paragraph and per definition term and value.
    const LOGOUT_TOKEN: &str = "2.4. Logout Token

OPs send a JWT similar to an ID Token to RPs called a Logout Token to request that they log out. ID Tokens are defined in Section 2 of [OpenID.Core] (Sakimura, N., Bradley, J., Jones, M., de Medeiros, B., and C. Mortimore, “OpenID Connect Core 1.0,” December 2023.).

The following Claims are used within the Logout Token:

sub
OPTIONAL. Subject Identifier, as specified in Section 2 of [OpenID.Core].
exp
REQUIRED. Expiration time, as specified in Section 2 of [OpenID.Core] (Sakimura, N., Bradley, J., Jones, M., de Medeiros, B., and C. Mortimore, “OpenID Connect Core 1.0,” December 2023.).
events
REQUIRED. Claim whose value is a JSON object containing the member name http://schemas.openid.net/event/backchannel-logout. This declares that the JWT is a Logout Token. The corresponding member value MUST be a JSON object and SHOULD be the empty JSON object {}.

A Logout Token MUST contain either a sub or a sid Claim, and MAY contain both. If a sid Claim is not present, the intent is that all sessions at the RP for the End-User identified by the iss and sub Claims be logged out.

The following Claim MUST NOT be used within the Logout Token:

nonce
PROHIBITED. A nonce Claim MUST NOT be present. Its use is prohibited to make a Logout Token syntactically invalid.

Logout Tokens MAY contain other Claims.";

    /// RFC 6749 §2.3 and §2.3.1 as the older RFC layout renders them:
    /// wrapped, indented text.
    const CLIENT_AUTH: &str = "2.3.1.  Client Password

   The authorization server MAY establish a client authentication method
   with public clients.  However, the authorization server MUST NOT rely
   on public client authentication for the purpose of identifying the
   client.

   client_id
         REQUIRED.  The client identifier issued to the client during
         the registration process described by Section 2.2.

   client_secret
         REQUIRED.  The client secret.  The client MAY omit the
         parameter if the client secret is an empty string.

   The client MUST NOT use more than one authentication method in each
   request.  The parameters can only be transmitted in the request-body
   and MUST NOT be included in the request URI (e.g., in a query string).";

    fn texts(statements: &[NormativeStatement]) -> Vec<&str> {
        statements.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn the_rows_of_a_claim_list_that_require_or_prohibit() {
        let found = normative_statements(LOGOUT_TOKEN);
        let rows: Vec<&NormativeStatement> = found
            .iter()
            .filter(|s| s.kind == StatementKind::Row)
            .collect();
        assert_eq!(rows.len(), 3, "{:#?}", texts(&found));
        assert!(rows[0].text.starts_with("exp: REQUIRED. Expiration time"));
        assert!(
            rows[0].text.ends_with("December 2023.)."),
            "{}",
            rows[0].text
        );
        assert_eq!(rows[0].line, 8);
        assert_eq!(rows[0].keywords, [Keyword::Required]);
        assert_eq!(rows[0].level(), Level::Must);
        assert!(
            rows[1]
                .text
                .starts_with("events: REQUIRED. Claim whose value")
        );
        assert!(rows[1].text.contains("MUST be a JSON object"));
        assert_eq!(
            rows[1].keywords,
            [Keyword::Must, Keyword::Required, Keyword::Should]
        );
        assert!(
            rows[2]
                .text
                .starts_with("nonce: PROHIBITED. A nonce Claim MUST NOT")
        );
        assert!(rows[2].prohibited);
        assert!(!texts(&found).iter().any(|t| t.starts_with("sub:")));
    }

    #[test]
    fn the_sentences_that_require_but_not_those_that_permit() {
        let found = normative_statements(LOGOUT_TOKEN);
        let sentences: Vec<&str> = found
            .iter()
            .filter(|s| s.kind == StatementKind::Sentence)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(
            sentences,
            [
                "A Logout Token MUST contain either a sub or a sid Claim, and MAY contain both.",
                "The following Claim MUST NOT be used within the Logout Token:",
            ]
        );
    }

    #[test]
    fn wrapped_text_is_one_sentence_and_indented_rows_continue() {
        let found = normative_statements(CLIENT_AUTH);
        assert_eq!(
            texts(&found),
            [
                "However, the authorization server MUST NOT rely on public client authentication for the purpose of identifying the client.",
                "client_id: REQUIRED. The client identifier issued to the client during the registration process described by Section 2.2.",
                "client_secret: REQUIRED. The client secret. The client MAY omit the parameter if the client secret is an empty string.",
                "The client MUST NOT use more than one authentication method in each request.",
                "The parameters can only be transmitted in the request-body and MUST NOT be included in the request URI (e.g., in a query string).",
            ]
        );
        assert_eq!(found[0].line, 2);
        assert_eq!(found[1].line, 7);
        assert_eq!(found[2].line, 11);
        assert_eq!(found[3].line, 15);
        assert_eq!(found[4].line, 15);
    }

    #[test]
    fn initials_and_abbreviations_do_not_end_a_sentence() {
        let text = "See Jones, M., and C. Mortimore, e.g. Section 2. The RP MUST check it.";
        assert_eq!(
            texts(&normative_statements(text)),
            ["The RP MUST check it."]
        );
        let one = "A client, i.e. Ed. Smith, MUST send etc. More text.";
        assert_eq!(
            texts(&normative_statements(one)),
            ["A client, i.e. Ed. Smith, MUST send etc. More text."]
        );
    }

    #[test]
    fn levels() {
        assert_eq!(normative_level(&[Keyword::May]), None);
        assert_eq!(normative_level(&[Keyword::Optional]), None);
        assert_eq!(
            normative_level(&[Keyword::Recommended]),
            Some(Level::Should)
        );
        assert_eq!(
            normative_level(&[Keyword::ShallNot, Keyword::Should]),
            Some(Level::Must)
        );
        assert!(normative_statements("").is_empty());
        assert!(normative_statements("\n\n").is_empty());
    }
}
