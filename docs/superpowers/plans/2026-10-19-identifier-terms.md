# Identifier Terms Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #18: split camelCase, snake_case and kebab-case identifiers into lowercase search terms in `codecite-core`, keeping the original, as `docs/design.md#indexing-and-retrieval` step 4 describes.

**Architecture:** A new `terms` module in `codecite-core` with two public functions. `split_identifier` walks the identifier's `char`s once and returns its lowercase parts. `identifier_terms` joins those parts with spaces and appends the original identifier: that string is what the indexer writes into the `terms` column of `chunk_fts`, which FTS5's `unicode61` tokenizer then splits on whitespace and punctuation. The design keeps splitting out of the tokenizer on purpose (`docs/design.md`, "Identifier splitting" decision row: a custom FTS5 tokenizer needs `unsafe` FFI).

**Tech Stack:** Rust 1.99.1 (edition 2024). No new dependencies.

**Spec:** Issue #18 and `docs/design.md#indexing-and-retrieval` (step 4, Terms).

## Global Constraints

- The examples, verbatim from #18, are tests:
  - `validateToken` → `validate token validateToken`
  - `id_token_hint` → `id token hint id_token_hint`
  - `back-channel` → `back channel back-channel`
  - `HTTPServer` → `http server HTTPServer`
  - Digits stay attached: `sha256` and `rfc9449` are one part each.
- Splitting rules:
  - Every character that is not alphanumeric (`char::is_alphanumeric`) separates parts and is dropped from them: `_`, `-`, `.`, `$`, whitespace. Runs of separators and leading or trailing separators make no empty parts.
  - A new part starts at an uppercase letter (`char::is_uppercase`) when the previous character is not uppercase (`validate|Token`, `sha256|Hash`, `名前|Value`), or when the previous character is uppercase and the next one is lowercase, which ends an acronym (`HTTP|Server`).
  - Digits never start a part: they join the part before them (`sha256`, `HTTP2|Server` → `http2 server`).
  - Each part is lowercased with `str::to_lowercase`, which handles non-ASCII letters (`ÜberMaß` → `über maß`).
- Output rules for `identifier_terms`:
  - The parts, joined by one space, then a space and the original identifier, unchanged.
  - When the identifier is exactly one part already (`token`, `sha256`), the output is the identifier alone, not `token token`.
  - Empty input gives an empty string. Input with no alphanumeric characters (`__`) gives the input alone.
- No byte-index slicing (`&s[i..]`) anywhere. Walk `chars()` and push into `String`s, so multibyte input can never panic on a char boundary.
- The workspace forbids `unsafe_code` and warns on `missing_docs`. Every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- An acronym followed by a word must split before the word's capital, not after the acronym's last letter (`HTTPServer` → `http server`, not `https erver`). Test: `acronym_then_word_splits_before_the_word`, Task 1.
- Digits must stay attached to the part before them and must not start a part (`sha256`, `rfc9449`, `HTTP2Server` → `http2 server`). Test: `digits_stay_attached`, Task 1.
- Non-ASCII identifiers must split by case and lowercase correctly, and never panic (`ÜberMaß`, `größeBerechnen`, `名前Value`). Test: `non_ascii_identifiers_split_without_panicking`, Task 1.
- Empty input and separator-only input must return no parts and must not panic. Tests: `empty_input_has_no_parts` (Task 1), `empty_input_gives_no_terms` (Task 2).
- A one-part identifier must not be repeated in its terms (`token` → `token`). Test: `one_part_identifier_is_not_repeated`, Task 2.
- Known limit, not a bug: a lone capital before a lowercase run splits off (`IPv4Address` → `i pv4 address`). Every case-based splitter makes this trade, and the original stays in the terms for an exact match. Test: `lone_capital_before_lowercase_splits_off`, Task 1, pins the behaviour so a change to it is deliberate.

---

### Task 1: Split an identifier into lowercase parts

**Files:**
- Create: `crates/codecite-core/src/terms.rs` (`split_identifier` and its tests)
- Modify: `crates/codecite-core/src/lib.rs`

**Interfaces:**
- Produces, re-exported from the crate root: `pub fn split_identifier(identifier: &str) -> Vec<String>`.

- [ ] **Step 1: Write the failing tests** in `terms.rs`'s `tests` module

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p codecite-core terms`
Expected: FAIL to compile (`split_identifier` not defined).

- [ ] **Step 3: Write `split_identifier`**

```rust
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

/// Moves a finished part, lowercased, into `parts`.
fn flush(current: &mut String, parts: &mut Vec<String>) {
    if !current.is_empty() {
        parts.push(current.to_lowercase());
        current.clear();
    }
}
```

> **Learn: iterating `char`s.** A `&str` is UTF-8 bytes, and one `char` (a Unicode scalar value) takes 1 to 4 of them. `chars()` decodes them one at a time. Collecting into `Vec<char>` gives an indexable sequence where `chars[i - 1]` and `chars.get(i + 1)` look one character back and ahead; `get` returns `None` past the end instead of panicking.
>
> **Learn: why byte indexing into a `&str` panics.** `&s[a..b]` takes byte offsets, and Rust panics when an offset lands inside a multibyte character (`&"é"[..1]`). `s[i]` doesn't compile at all, because "the i-th character" isn't an O(1) operation on UTF-8. Building parts by pushing `char`s into a `String` avoids offsets altogether, which is why `ÜberMaß` can't panic here.
>
> **Learn: `char::is_uppercase`.** It reads the Unicode `Uppercase` property, so `Ü` is uppercase and `ß` is lowercase. CJK characters like `名` are neither, so they behave like lowercase for splitting. `is_alphanumeric` and `is_lowercase` are the same kind of Unicode check, and `str::to_lowercase` handles letters whose lowercase form is more than one `char`, which `char::to_lowercase` returns as an iterator.

In `lib.rs`, add after the `store` lines:

```rust
mod terms;
pub use terms::split_identifier;
```

and extend the crate doc comment with: "It also splits identifiers into the search terms the index stores."

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p codecite-core terms`
Expected: PASS, 7 tests.

- [ ] **Step 5: Run the gates and commit**

Run the gates. Commit `feat(core): split identifiers into lowercase parts (#18)`.

---

### Task 2: Build the terms string and record the rules in the design

**Files:**
- Modify: `crates/codecite-core/src/terms.rs` (`identifier_terms` and its tests)
- Modify: `crates/codecite-core/src/lib.rs`
- Modify: `docs/design.md` (Indexing pipeline, step 4)

**Interfaces:**
- Consumes: `split_identifier` from Task 1.
- Produces, re-exported from the crate root: `pub fn identifier_terms(identifier: &str) -> String`.

- [ ] **Step 1: Write the failing tests**, added to the `tests` module

```rust
#[test]
fn terms_match_the_issue_examples() {
    assert_eq!(identifier_terms("validateToken"), "validate token validateToken");
    assert_eq!(identifier_terms("id_token_hint"), "id token hint id_token_hint");
    assert_eq!(identifier_terms("back-channel"), "back channel back-channel");
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p codecite-core terms`
Expected: FAIL to compile (`identifier_terms` not defined).

- [ ] **Step 3: Write `identifier_terms`**

```rust
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
```

> **Learn: slice patterns and let chains.** `if let [only] = parts.as_slice()` matches a slice of exactly one element and binds it. Edition 2024 lets `&&` chain a plain condition onto an `if let`, so the length check and the comparison read as one test.

In `lib.rs`, change the re-export to `pub use terms::{identifier_terms, split_identifier};`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p codecite-core terms`
Expected: PASS, 11 tests.

- [ ] **Step 5: Record the rules in `docs/design.md`**

Replace step 4 of the indexing pipeline with:

```markdown
4. **Terms:** split camelCase, snake_case and kebab-case identifiers into lowercase parts and keep the original, so `validateToken` matches "validate token" and the exact name. Any character that is not a letter or digit separates parts, an acronym ends before the capital that starts the next word (`HTTPServer` → `http server`), and digits stay with the part before them (`sha256`, `rfc9449`).
```

- [ ] **Step 6: Run the gates and commit**

Run the gates. Commit `feat(core): build identifier search terms (#18)`.
