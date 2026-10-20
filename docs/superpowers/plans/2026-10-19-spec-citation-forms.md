# Spec Citation Forms Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #16: parse the spec citation forms people write (`RFC 9449 §4.3`, `RFC9449 Section 4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)`, a bare `§3.1.3.7 step 12`, and spec URLs with an anchor) into an unresolved reference in `codecite-core`.

**Architecture:** A new submodule `citation/written.rs` next to `code.rs` and `spec.rs`. It holds `SpecAliases` (the alias table the manifest will supply), `SpecTarget` (a numbered section or the document's own anchor) and `SpecReference` (optional document, target, optional step). One entry point, `SpecReference::parse(s, &aliases) -> Result<Option<SpecReference>, CitationError>`, tries the URL forms, then the `§` form, then the `Section` form, then the bare `<doc> <n>(<k>)` form. Section numbers reuse `SectionNumber` and `parse_number` from #15, so the written forms share the canonical number rules. A hand-written parser over `strip_prefix`, `split_once`, `rsplit_once` and `split_whitespace`; no regex.

**Tech Stack:** Rust 1.99.1 (edition 2024), `thiserror` 2. No new dependencies.

**Spec:** Issue #16, `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` (Spec structure), and `docs/design.md#interfaces` (the citation-forms bullet).

## Global Constraints

- **One citation per string.** The input is one citation, trimmed. Extracting citations from prose is the M4 citation extractor's job. `§2 / §3.1.3.7`, `§3.1.3.7 and §3.3.2.11` and `RFC 9449 §8: honor …` are errors (text after the citation), not two references or a prefix match.
- **`Ok(None)` means "not shaped like a written spec citation"; `Err` means "shaped like one but malformed".** A caller such as `read` can try the canonical parser, then this one, then report "unrecognized". The rules:
  - Contains `§` → always `Ok(Some)` or `Err`.
  - Has a whitespace-separated word `Section` (any ASCII case) → always `Ok(Some)` or `Err`.
  - Starts with `http://` or `https://`: an `rfc-editor.org/rfc/…` or `openid.net/specs/….html` URL → `Ok(Some)` or `Err`; any other URL → `Ok(None)`.
  - Otherwise the bare form `<doc> <n>(<k>)`: recognized only when the last word starts with a digit and ends with `)` **and** the text before it names a known document. Anything else, including an unknown document in this form, is `Ok(None)`, because the document is the only sign that the string is a citation at all.
- **Documents.**
  - Empty document text (a bare `§` or bare `Section`) → `doc: None`. Resolving it is `read`'s and `cite`'s job (#16).
  - Aliases come from a `SpecAliases` table the caller builds (the manifest will). Lookups ignore ASCII case and collapse runs of whitespace, so `oidc  core` matches `OIDC Core`. The alias table is consulted first.
  - Otherwise `RFC <n>`, `RFC<n>`, `rfc <n>` and `rfc<n>` resolve to `rfc<n>` with no alias, since RFC numbers are the documents' corpus names. `<n>` goes through `parse_number(n, "RFC")`, so `RFC 09449` is an error.
  - Anything else in the `§` or `Section` form → `UnknownDocument`.
- **Locator** (after `§` or `Section`, and the last word of the bare form): a leading run of ASCII digits and `.` parsed as `SectionNumber` (so `04.3`, `4..3`, `3.1.3.7.` and the empty string are `BadSection`), then exactly one of: nothing; `(<k>)`; or whitespace, the word `step` (any ASCII case), whitespace and `<k>` with nothing after. `<k>` goes through `parse_number(k, "step")`. Anything else after the section is `TrailingText`. Whitespace between `§` and the number is allowed (`§ 4.3`).
- **Step ranges are not accepted.** `§3.1.3.7 steps 4-6` (in identity-model) is `TrailingText` and `§3.1.3.7 step 4-6` is `BadNumber`. The canonical form names one step, and a range is a design question for `read`/`cite`. Section ranges (`§5.1–5.2`) are `TrailingText` too.
- **URLs.**
  - `http(s)://[www.]rfc-editor.org/rfc/rfc<n>[.html]#<anchor>` → `doc: rfc<n>`. An anchor `section-<n>` becomes `SpecTarget::Section` (parsed as `SectionNumber`, so `section-04.3` is `BadSection`); any other anchor (`appendix-B`) becomes `SpecTarget::Anchor`. A path segment that is not `rfc<digits…>` (for example `rfc/`) is `Ok(None)`; `rfc09449` is `BadNumber`.
  - `http(s)://[www.]openid.net/specs/<name>.html#<anchor>` → `doc: <name>` (`openid-connect-core-1_0`), always `SpecTarget::Anchor`. `<name>` must be a valid spec document name (ASCII letters, digits, `-`, `_`, `.`); otherwise `Ok(None)`.
  - A recognized URL with no `#` or an empty anchor is `UrlWithoutAnchor`. An anchor with whitespace is `BadAnchor`.
  - URLs never carry a step.
- The written forms have no `Display` and no round-trip promise; only the canonical forms do (#15).
- No byte-index slicing with computed literals (`&s[..n]`). The one `split_at` uses an index from `str::find`, which is always a char boundary. Multibyte input must return `Ok(None)` or `Err`, never panic.
- The workspace forbids `unsafe_code` and warns on `missing_docs`. Every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- A string that is not a spec citation (prose, a canonical citation, a code citation, an unrelated URL, `len(2)`) must be `Ok(None)`, not an error, or `read` cannot fall through to "unrecognized". Test: `non_citations_are_none`, Task 1, and `other_urls_are_none`, Task 2.
- A malformed citation must be `Err` naming the bad piece, not `Ok(None)`, or a typo reads as "not a citation". Tests: `malformed_citations_are_errors` (Task 1), `malformed_urls_are_errors` (Task 2).
- A bare `§` must give `doc: None`, never a guessed document (identity-model#861). Test: `forms_from_the_issue`, Task 1.
- Text after the citation (`§2 / §3.1.3.7`, `steps 4-6`) must be an error, not silently dropped. Test: `malformed_citations_are_errors`, Task 1.
- Multibyte input must not panic. Test: `multibyte_input_is_not_a_panic`, Task 1.

---

### Task 1: Written prose forms and the alias table

**Files:**
- Create: `crates/codecite-core/src/citation/written.rs` (`SpecAliases`, `SpecTarget`, `SpecReference`, the prose parser and its tests)
- Modify: `crates/codecite-core/src/citation.rs` (new `CitationError` variants, `mod written`, the `BadNumber` doc)
- Modify: `crates/codecite-core/src/citation/spec.rs` (pull the document-name check into `pub(super) fn is_doc_name`, used again in Task 2)
- Modify: `crates/codecite-core/src/lib.rs` (re-exports, crate doc)

**Interfaces:**
- Consumes: `CitationError`, `parse_number` (`citation.rs`), `SectionNumber` (`citation/spec.rs`) from #15.
- Produces, re-exported from the crate root:
  - `pub struct SpecAliases(HashMap<String, String>)`, `#[derive(Debug, Clone, Default, PartialEq, Eq)]`, with `fn new() -> Self`, `fn insert(&mut self, alias: &str, doc: &str)`, `fn get(&self, alias: &str) -> Option<&str>`, and `impl<'a> FromIterator<(&'a str, &'a str)>`.
  - `pub enum SpecTarget { Section(SectionNumber), Anchor(String) }`, `#[derive(Debug, Clone, PartialEq, Eq, Hash)]`.
  - `pub struct SpecReference { doc: Option<String>, target: SpecTarget, step: Option<u32> }`, same derives, with `fn doc(&self) -> Option<&str>`, `fn target(&self) -> &SpecTarget`, `fn step(&self) -> Option<u32>`, and `fn parse(s: &str, aliases: &SpecAliases) -> Result<Option<Self>, CitationError>`.
- New `CitationError` variants (the enum is already `#[non_exhaustive]`): `UnknownDocument { found: String }`, `TrailingText { found: String }`, `UrlWithoutAnchor { url: String }`, `BadAnchor { found: String }`. The last two are used in Task 2 but added now so the error enum changes once.

- [ ] **Step 1: Write the failing tests** in `citation/written.rs`'s `tests` module

```rust
use super::*;
use crate::CitationError;

fn aliases() -> SpecAliases {
    [
        ("OIDC Core", "openid-connect-core-1_0"),
        ("OIDC Core 1.0", "openid-connect-core-1_0"),
        ("OpenID Connect Core 1.0", "openid-connect-core-1_0"),
        ("Core", "openid-connect-core-1_0"),
        ("OIDC Discovery 1.0", "openid-connect-discovery-1_0"),
        ("Back-Channel Logout 1.0", "openid-connect-backchannel-1_0"),
    ]
    .into_iter()
    .collect()
}

/// Parses `s` and checks the document, section parts and step.
fn check(s: &str, doc: Option<&str>, section: &[u32], step: Option<u32>) {
    let r = SpecReference::parse(s, &aliases())
        .unwrap_or_else(|e| panic!("{s}: {e}"))
        .unwrap_or_else(|| panic!("{s}: no match"));
    assert_eq!(r.doc(), doc, "{s}");
    match r.target() {
        SpecTarget::Section(n) => assert_eq!(n.parts(), section, "{s}"),
        SpecTarget::Anchor(a) => panic!("{s}: anchor {a}"),
    }
    assert_eq!(r.step(), step, "{s}");
}

#[test]
fn forms_from_the_issue() {
    check("RFC 9449 §4.3", Some("rfc9449"), &[4, 3], None);
    check("RFC9449 Section 4.3", Some("rfc9449"), &[4, 3], None);
    check(
        "OIDC Core §3.1.3.7 step 13",
        Some("openid-connect-core-1_0"),
        &[3, 1, 3, 7],
        Some(13),
    );
    check("Core 3.1.3.7(13)", Some("openid-connect-core-1_0"), &[3, 1, 3, 7], Some(13));
    check("§3.1.3.7 step 12", None, &[3, 1, 3, 7], Some(12));
}

/// Citation shapes found in ~/repos/auth/identity-model at 9458d60
/// (grep for `§` and `Section`), as written there.
#[test]
fn shapes_from_identity_model() {
    let core = Some("openid-connect-core-1_0");
    check("RFC 9449 §8", Some("rfc9449"), &[8], None);
    check("RFC 6749 §2.3.1", Some("rfc6749"), &[2, 3, 1], None);
    check("OIDC Core §3.1.3.7", core, &[3, 1, 3, 7], None);
    check("OIDC Core 1.0 §3.1.3.7", core, &[3, 1, 3, 7], None);
    check("OIDC Discovery 1.0 §3", Some("openid-connect-discovery-1_0"), &[3], None);
    check(
        "Back-Channel Logout 1.0 §2.4 step 2",
        Some("openid-connect-backchannel-1_0"),
        &[2, 4],
        Some(2),
    );
    check("§3.3.2.11", None, &[3, 3, 2, 11], None);
    check("§3.1.3.7 step 11", None, &[3, 1, 3, 7], Some(11));
    check("RFC 7592 Section 2.3", Some("rfc7592"), &[2, 3], None);
    check("OpenID Connect Core 1.0 Section 9", core, &[9], None);
    check("OIDC Core Section 10.1", core, &[10, 1], None);
    check("Section 2.2", None, &[2, 2], None);
}

#[test]
fn spacing_and_case_are_forgiven() {
    let core = Some("openid-connect-core-1_0");
    check("  oidc  core   §3.1  ", core, &[3, 1], None);
    check("RFC 9449 § 4.3", Some("rfc9449"), &[4, 3], None);
    check("rfc9449 section 4.3", Some("rfc9449"), &[4, 3], None);
    check("Core §3.1.3.7 Step 13", core, &[3, 1, 3, 7], Some(13));
    check("Core §3.1.3.7(13)", core, &[3, 1, 3, 7], Some(13));
}

#[test]
fn an_alias_wins_over_the_rfc_rule() {
    let a: SpecAliases = [("RFC 6749bis", "draft-ietf-oauth-v2-1")].into_iter().collect();
    let r = SpecReference::parse("RFC 6749bis §4.1", &a).unwrap().unwrap();
    assert_eq!(r.doc(), Some("draft-ietf-oauth-v2-1"));
}

#[test]
fn non_citations_are_none() {
    for s in [
        "",
        "hello world",
        "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
        "github.com/panva/openid-client@3f2a9c1:lib/client.js#L412",
        "len(2)",
        "Unknown 3.1(2)",
        "Core 3.1.3.7",
    ] {
        assert_eq!(SpecReference::parse(s, &aliases()), Ok(None), "{s:?}");
    }
}

#[test]
fn malformed_citations_are_errors() {
    use CitationError::*;
    let cases: &[(&str, fn(&CitationError) -> bool)] = &[
        ("Foo §3.1", |e| matches!(e, UnknownDocument { .. })),
        ("Foo Section 3.1", |e| matches!(e, UnknownDocument { .. })),
        ("§", |e| matches!(e, BadSection { .. })),
        ("RFC 9449 §04.3", |e| matches!(e, BadSection { .. })),
        ("§3.1.3.7.", |e| matches!(e, BadSection { .. })),
        ("RFC 09449 §4.3", |e| matches!(e, BadNumber { what: "RFC", .. })),
        ("RFC §4.3", |e| matches!(e, BadNumber { what: "RFC", .. })),
        ("§3.1.3.7 step 0", |e| matches!(e, BadNumber { what: "step", .. })),
        ("§3.1.3.7 step 4-6", |e| matches!(e, BadNumber { what: "step", .. })),
        ("Core 3.1.3.7(013)", |e| matches!(e, BadNumber { what: "step", .. })),
        ("§3.1.3.7 steps 4-6", |e| matches!(e, TrailingText { .. })),
        ("§2 / §3.1.3.7", |e| matches!(e, TrailingText { .. })),
        ("§5.1–5.2", |e| matches!(e, TrailingText { .. })),
        ("RFC 9449 §8: honor", |e| matches!(e, TrailingText { .. })),
        ("§3.1.3.7 step 12 extra", |e| matches!(e, TrailingText { .. })),
    ];
    for (s, want) in cases {
        let got = SpecReference::parse(s, &aliases());
        assert!(matches!(&got, Err(e) if want(e)), "{s}: {got:?}");
    }
}

#[test]
fn multibyte_input_is_not_a_panic() {
    for s in ["§3.1é", "§é", "Coreé 3.1(2)", "Core 3.1(2é)", "é Section 1"] {
        let _ = SpecReference::parse(s, &aliases());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p codecite-core written`
Expected: FAIL to compile (`SpecReference`, `SpecAliases`, `SpecTarget` not defined).

- [ ] **Step 3: Add the error variants in `citation.rs`**

After `BadSection`:

```rust
    /// A written citation names a document that is neither an RFC nor a
    /// known alias.
    #[error("no document is called {found:?}")]
    UnknownDocument {
        /// The document as written.
        found: String,
    },

    /// A written citation has text after it that is not a step.
    #[error("unexpected {found:?} after the citation")]
    TrailingText {
        /// The text after the citation, trimmed.
        found: String,
    },

    /// A spec URL has no `#` anchor naming a section.
    #[error("spec URL {url:?} has no #anchor naming a section")]
    UrlWithoutAnchor {
        /// The URL as written.
        url: String,
    },

    /// A spec URL's anchor has whitespace in it.
    #[error("anchor {found:?} has whitespace in it")]
    BadAnchor {
        /// The anchor as written, without `#`.
        found: String,
    },
```

Change the `BadNumber` `what` doc to "Which number: `line`, `section`, `step` or `RFC`." Add `mod written; pub use written::{SpecAliases, SpecReference, SpecTarget};`, and say in the module doc that `written` parses the lenient forms, which have no round trip.

In `spec.rs`, replace the inline name check in `SpecCitation::from_str` with a call to:

```rust
/// Whether `s` is a spec document name: non-empty ASCII letters, digits,
/// `-`, `_` and `.`.
pub(super) fn is_doc_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
```

(The empty check stays first in `from_str` so it still returns `EmptyName`.)

- [ ] **Step 4: Implement `written.rs`**

```rust
//! The spec citation forms people write: `RFC 9449 §4.3`,
//! `RFC9449 Section 4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)`, a
//! bare `§3.1.3.7`, and spec URLs with an anchor. They parse to an unresolved
//! [`SpecReference`]; resolving one against a corpus is `read`'s and `cite`'s
//! job.

use std::collections::HashMap;

use super::{CitationError, SectionNumber, parse_number};
```

- `SpecAliases`: a `HashMap` from the normalized alias to the document name. `normalize(s)` is `s.split_whitespace().map(str::to_ascii_lowercase).collect::<Vec<_>>().join(" ")`. `insert` and `get` both normalize. `FromIterator<(&str, &str)>` builds with `new()` and `insert` in a `for` loop.
- `SpecTarget`, `SpecReference` and the accessors as in Interfaces. `doc()` is `self.doc.as_deref()`.
- `SpecReference::parse`:

```rust
    pub fn parse(s: &str, aliases: &SpecAliases) -> Result<Option<Self>, CitationError> {
        let s = s.trim();
        if let Some((doc, locator)) = s.split_once('§') {
            return Self::written(resolve_doc(doc, aliases)?, locator.trim_start()).map(Some);
        }
        let words: Vec<&str> = s.split_whitespace().collect();
        if let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case("section")) {
            let doc = resolve_doc(&words[..i].join(" "), aliases)?;
            return Self::written(doc, &words[i + 1..].join(" ")).map(Some);
        }
        Self::bare(s, aliases)
    }
```

  Task 2 adds the URL branch at the top.
- `fn written(doc: Option<String>, locator: &str) -> Result<Self, CitationError>`: `parse_locator(locator)` and build `SpecReference { doc, target: SpecTarget::Section(section), step }`.
- `fn bare(s, aliases) -> Result<Option<Self>, CitationError>`: `let Some((doc, last)) = s.rsplit_once(char::is_whitespace) else { return Ok(None) };`. If `last` does not start with an ASCII digit or does not end with `)`, `Ok(None)`. `match resolve_doc(doc, aliases)`: `Ok(Some(doc))` continues; `Ok(None) | Err(CitationError::UnknownDocument { .. })` is `Ok(None)`; any other `Err` returns. Then `Self::written(Some(doc), last).map(Some)`.
- `fn resolve_doc(text: &str, aliases: &SpecAliases) -> Result<Option<String>, CitationError>`: trim; empty is `Ok(None)`; `aliases.get(text)` wins; then `text.strip_prefix("RFC").or_else(|| text.strip_prefix("rfc"))`, trim the rest, and `parse_number(rest, "RFC")` gives `format!("rfc{n}")`; otherwise `UnknownDocument`.
- `fn parse_locator(locator: &str) -> Result<(SectionNumber, Option<u32>), CitationError>`:

```rust
    // `find` returns a char boundary, so `split_at` cannot panic on
    // multibyte input.
    let end = locator
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(locator.len());
    let (section, rest) = locator.split_at(end);
    let section = section.parse()?;
    let step = if rest.is_empty() {
        None
    } else if let Some(k) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        Some(parse_number(k, "step")?)
    } else if let Some(k) = step_word(rest) {
        Some(parse_number(k, "step")?)
    } else {
        return Err(CitationError::TrailingText { found: rest.trim().into() });
    };
    Ok((section, step))
```

- `fn step_word(rest: &str) -> Option<&str>`: `None` unless `rest` starts with whitespace; then take three words from `split_whitespace()` and match `(Some(w), Some(k), None) if w.eq_ignore_ascii_case("step") => Some(k)`, else `None`.

Learn: a small hand-written parser is a few functions that each peel one piece off the front of a `&str` and hand the rest on: `split_once('§')` finds the document, `find` plus `split_at` takes the number, and `strip_prefix('(')` and `strip_suffix(')')` take a step. Each returns borrowed slices of the input, so nothing is copied until a value is built.

Learn: `Result<Option<T>, E>` gives three outcomes: `Ok(Some(r))` parsed, `Ok(None)` "this is not my form, try another parser", and `Err(e)` "this is my form and it is wrong". Returning `Option` alone would turn a typo into "not a citation"; returning `Result` alone would make prose an error. `?` still works on the outer `Result`, and `Option::transpose` converts to `Option<Result<T, E>>` when a caller wants to chain with `Option` combinators.

Learn: `let … else` (`let Some((doc, last)) = s.rsplit_once(…) else { return Ok(None) };`) names the success case and exits early on the other one, without nesting the rest of the function in a `match`.

Learn: `match` on `Result` with an or-pattern (`Ok(None) | Err(CitationError::UnknownDocument { .. })`) folds several outcomes into one arm; `{ .. }` ignores a variant's fields.

- [ ] **Step 5: Re-export and run the tests**

In `lib.rs`, add `SpecAliases`, `SpecReference`, `SpecTarget` to the `pub use citation::{…}` list and mention the written forms in the crate doc.

Run: `cargo test -p codecite-core`
Expected: PASS (the #15 tests too, including the `is_doc_name` refactor).

- [ ] **Step 6: Run the gates and commit**

Run the full gates from Global Constraints. Expected: all green.

```bash
git add crates/codecite-core/src/citation.rs crates/codecite-core/src/citation/spec.rs crates/codecite-core/src/citation/written.rs crates/codecite-core/src/lib.rs
git commit -m "feat(core): parse the written spec citation forms (#16)"
```

### Task 2: Spec URLs and the design doc

**Files:**
- Modify: `crates/codecite-core/src/citation/written.rs` (URL branch and tests)
- Modify: `docs/design.md` (Interfaces: the citation-forms bullet)

**Interfaces:**
- Consumes: everything from Task 1, `is_doc_name` from `spec.rs`.
- Produces: no new public items. `SpecReference::parse` also accepts the two URL forms.

- [ ] **Step 1: Write the failing tests** in `written.rs`'s `tests` module

```rust
/// Parses a URL and checks the document and target.
fn check_url(s: &str, doc: &str, target: SpecTarget) {
    let r = SpecReference::parse(s, &aliases())
        .unwrap_or_else(|e| panic!("{s}: {e}"))
        .unwrap_or_else(|| panic!("{s}: no match"));
    assert_eq!(r.doc(), Some(doc), "{s}");
    assert_eq!(r.target(), &target, "{s}");
    assert_eq!(r.step(), None, "{s}");
}

fn section(s: &str) -> SpecTarget {
    SpecTarget::Section(s.parse().unwrap())
}

fn anchor(s: &str) -> SpecTarget {
    SpecTarget::Anchor(s.into())
}

#[test]
fn urls_from_the_issue() {
    check_url(
        "https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation",
        "openid-connect-core-1_0",
        anchor("IDTokenValidation"),
    );
    check_url(
        "https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3",
        "rfc9449",
        section("4.3"),
    );
}

/// URL shapes found in ~/repos/auth/identity-model at 9458d60
/// (grep for `openid.net/specs` and `rfc-editor.org`).
#[test]
fn urls_from_identity_model() {
    check_url("https://www.rfc-editor.org/rfc/rfc6749#section-5.2", "rfc6749", section("5.2"));
    check_url("https://www.rfc-editor.org/rfc/rfc8693#section-2.2.1", "rfc8693", section("2.2.1"));
    check_url("https://www.rfc-editor.org/rfc/rfc7636#appendix-B", "rfc7636", anchor("appendix-B"));
    check_url(
        "https://openid.net/specs/openid-connect-discovery-1_0.html#ProviderMetadata",
        "openid-connect-discovery-1_0",
        anchor("ProviderMetadata"),
    );
    check_url(
        "http://openid.net/specs/openid-connect-basic-1_0-32.html#ClaimStability",
        "openid-connect-basic-1_0-32",
        anchor("ClaimStability"),
    );
    check_url("https://rfc-editor.org/rfc/rfc9449#section-8", "rfc9449", section("8"));
}

#[test]
fn other_urls_are_none() {
    for s in [
        "https://example.com/rfc9449#section-4.3",
        "https://www.rfc-editor.org/info/rfc9449",
        "https://www.rfc-editor.org/rfc/",
        "https://openid.net/specs/",
        "https://openid.net/specs/a/b.html#x",
        "https://openid.net/foundation/",
    ] {
        assert_eq!(SpecReference::parse(s, &aliases()), Ok(None), "{s}");
    }
}

#[test]
fn malformed_urls_are_errors() {
    use CitationError::*;
    let cases: &[(&str, fn(&CitationError) -> bool)] = &[
        ("https://www.rfc-editor.org/rfc/rfc9449", |e| matches!(e, UrlWithoutAnchor { .. })),
        ("https://www.rfc-editor.org/rfc/rfc9449#", |e| matches!(e, UrlWithoutAnchor { .. })),
        ("https://openid.net/specs/openid-connect-discovery-1_0.html", |e| {
            matches!(e, UrlWithoutAnchor { .. })
        }),
        ("https://www.rfc-editor.org/rfc/rfc9449#section-04.3", |e| matches!(e, BadSection { .. })),
        ("https://www.rfc-editor.org/rfc/rfc09449#section-1", |e| {
            matches!(e, BadNumber { what: "RFC", .. })
        }),
        ("https://openid.net/specs/openid-connect-core-1_0.html#ID Token", |e| {
            matches!(e, BadAnchor { .. })
        }),
    ];
    for (s, want) in cases {
        let got = SpecReference::parse(s, &aliases());
        assert!(matches!(&got, Err(e) if want(e)), "{s}: {got:?}");
    }
}
```

Note: `https://openid.net/specs/openid-connect-core-1_0.html#ID Token` has a space, and so would otherwise reach the `Section`/bare branches; the URL branch must run first and own every string that starts with a scheme. A URL with `§` in its anchor is likewise the URL branch's.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p codecite-core written`
Expected: FAIL (URLs return `Ok(None)` or the wrong error).

- [ ] **Step 3: Implement the URL branch**

At the top of `SpecReference::parse`, after the trim:

```rust
        if let Some(rest) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) {
            return Self::url(s, rest);
        }
```

`fn url(url: &str, rest: &str) -> Result<Option<Self>, CitationError>`:

- `let rest = rest.strip_prefix("www.").unwrap_or(rest);` then `let (path, anchor) = match rest.split_once('#') { Some((p, a)) => (p, Some(a)), None => (rest, None) };`.
- RFC Editor: `if let Some(name) = path.strip_prefix("rfc-editor.org/rfc/")`: `let name = name.strip_suffix(".html").unwrap_or(name);`, then `let Some(n) = name.strip_prefix("rfc").filter(|n| n.starts_with(|c: char| c.is_ascii_digit())) else { return Ok(None) };`, `let doc = format!("rfc{}", parse_number(n, "RFC")?);`, `let anchor = need_anchor(url, anchor)?;`, and the target is `SpecTarget::Section(s.parse()?)` when `anchor.strip_prefix("section-")` is `Some(s)`, else `SpecTarget::Anchor(anchor.into())`.
- OpenID: `if let Some(file) = path.strip_prefix("openid.net/specs/")`: `let Some(name) = file.strip_suffix(".html").filter(|n| is_doc_name(n)) else { return Ok(None) };` and the target is `SpecTarget::Anchor(need_anchor(url, anchor)?.into())`.
- Any other URL: `Ok(None)`.
- `fn need_anchor<'a>(url: &str, anchor: Option<&'a str>) -> Result<&'a str, CitationError>`: `None` or `Some("")` is `UrlWithoutAnchor { url: url.into() }`; an anchor with any `char::is_whitespace` is `BadAnchor`; else `Ok(anchor)`.

Learn: `Option::filter` keeps a `Some` only when a predicate holds, so `strip_suffix(".html").filter(|n| is_doc_name(n))` is "ends in `.html` and the rest is a valid name" in one expression, ready for `let … else`.

Learn: the lifetime in `need_anchor<'a>` says the returned `&str` borrows from `anchor`, not from `url`. Without it the compiler cannot tell which input the output borrows from, because there are two reference inputs.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p codecite-core`
Expected: PASS.

- [ ] **Step 5: Record the written-form rules in `docs/design.md`**

In the Interfaces bullet that starts "Code citations look like", after the sentence that ends "a spec URL with its anchor).", add: "These parse to an unresolved reference (a document or none, a section number or the document's own anchor, and an optional step), with document aliases taken from the manifest. A string that matches no form is no match rather than an error, and one string holds one citation: step ranges such as `steps 4-6` and lists such as `§2 / §3.1.3.7` are errors."

- [ ] **Step 6: Run the gates and commit**

Run the full gates from Global Constraints. Expected: all green.

```bash
git add crates/codecite-core/src/citation/written.rs docs/design.md
git commit -m "feat(core): parse spec URLs with an anchor (#16)"
```
