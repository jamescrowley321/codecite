# Canonical Citations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #15: a `Citation` type in `codecite-core` that parses and formats the canonical code and spec citation forms exactly, and returns a typed error for invalid input instead of panicking.

**Architecture:** A new `citation` module in `codecite-core`, with one submodule per form (`citation/code.rs`, `citation/spec.rs`). Each piece of a citation is a small validated newtype (`CommitSha`, `LineRange`, `ContentHash`, `SectionNumber`) with its own `FromStr` and `Display`. `CodeCitation` and `SpecCitation` compose those pieces, and `Citation` is an enum over the two forms that picks one by whether the text after the first `@` starts with `sha256:`. Parsing uses `split_once`, `rsplit_once` and `strip_prefix`; there is no regex.

**Tech Stack:** Rust 1.99.1 (edition 2024), `thiserror` 2. No new dependencies.

**Spec:** Issue #15, `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` (Spec structure), and `docs/design.md#interfaces` (the citation forms under the MCP tool list).

## Global Constraints

- The two canonical forms, verbatim from #15:
  - Code: `<repo>@<sha>:<path>#L<a>-L<b>`, and `#L<a>` for one line. Example: `github.com/panva/openid-client@3f2a9c1:lib/client.js#L412-L455`.
  - Spec: `<doc>@sha256:<12 hex>#section-<n>[/step-<k>]`. Examples: `rfc9449@sha256:1a2b3c4d5e6f#section-4.3`, `openid-connect-core-1_0@sha256:1a2b3c4d5e6f#section-3.1.3.7/step-13`.
- **Strict canonical grammar.** Every string the parser accepts formats back to exactly the same string. The parser never normalizes. It rejects anything that has a second spelling instead:
  - Hex (commit sha and content hash) is lowercase only.
  - Numbers (line numbers, section components, step numbers) are decimal `u32` values of at least 1, with no sign and no leading zeros.
  - A one-line range is written `#L10`. `#L10-L10` is rejected.
- The commit sha is 7 to 64 lowercase hex digits: 7 is git's default abbreviation and the design's example, and 64 is a full SHA-256 object name. The content hash is exactly 12 lowercase hex digits (#15).
- Splitting rules. The name is everything before the **first** `@`. For code, the sha runs to the **first** `:` after the `@`, and the path runs from there to the **last** `#`. So a path may contain `@`, `:` and `#` (`node_modules/@scope/x.js`, `docs/c#/notes.md`).
- Name rules. A repo is any non-empty text without whitespace. A spec document name is non-empty ASCII letters, digits, `-`, `_` and `.` (`rfc9449`, `openid-connect-core-1_0`). A path is any non-empty text.
- The lenient forms people write (`RFC 9449 §4.3`, `Core 3.1.3.7(13)`, spec URLs) are out of scope. They belong to `read` and `cite`, which resolve them against a corpus.
- No byte-index slicing (`&s[..n]`) anywhere in the parser. Use `split_once`, `rsplit_once`, `strip_prefix` and `chars()`, so multibyte input can never panic on a char boundary.
- The workspace forbids `unsafe_code` and warns on `missing_docs`. Every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- A non-canonical spelling of a valid citation (`#L10-L10`, `#L010`, `@3F2A9C1`, `#section-04.3`) must be rejected, not silently normalized. If it were accepted, two strings would name the same span and the round-trip promise would break. Test: `non_canonical_spellings_are_rejected`, Task 1 and Task 2.
- A path that contains `#`, `@` or `:` must parse with the whole path intact. Test: `path_may_contain_hash_at_and_colon`, Task 1.
- A number too big for `u32` (`#L99999999999`) must return an error, not panic or wrap. Tests: `line_number_overflow_is_an_error` (Task 1), `section_component_overflow_is_an_error` (Task 2).
- Multibyte input where a byte length and a char count differ (`rfc9449@sha256:1a2b3c4d5eé#section-1`) must return an error, not panic on a char boundary. Tests: `multibyte_hash_is_an_error_not_a_panic` (Task 2), `multibyte_sha_is_an_error_not_a_panic` (Task 1).
- Empty pieces (`@3f2a9c1:a#L1`, `r@3f2a9c1:#L1`, `#section-`, `#section-4..3`, `#section-4.`, `/step-`) and trailing junk (`/step-13/x`, `#L4-L9x`) must each return an error naming the bad piece. Tests: `empty_pieces_are_errors` and `trailing_junk_is_an_error`, Task 1 and Task 2.

---

### Task 1: The citation error type and code citations

**Files:**
- Create: `crates/codecite-core/src/citation.rs` (module root: `CitationError`, `parse_number`, re-exports)
- Create: `crates/codecite-core/src/citation/code.rs` (`CommitSha`, `LineRange`, `CodeCitation` and their tests)
- Modify: `crates/codecite-core/src/lib.rs`

**Interfaces:**
- Produces, re-exported from the crate root:
  - `pub enum CitationError` (`#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]`, `#[non_exhaustive]`)
  - `pub struct CommitSha(String)` with `fn as_str(&self) -> &str`
  - `pub struct LineRange { start: u32, end: u32 }` with `fn start(&self) -> u32` and `fn end(&self) -> u32`
  - `pub struct CodeCitation { repo: String, commit: CommitSha, path: String, lines: LineRange }` with `fn repo(&self) -> &str`, `fn commit(&self) -> &CommitSha`, `fn path(&self) -> &str`, `fn lines(&self) -> LineRange`
  - Each of the four types: `impl FromStr<Err = CitationError>` and `impl Display`, plus `#[derive(Debug, Clone, PartialEq, Eq, Hash)]` (`LineRange` is also `Copy`).
- Produces, crate-private in `citation.rs`, for Task 2: `fn parse_number(s: &str, what: &'static str) -> Result<u32, CitationError>`.

- [ ] **Step 1: Write the failing tests** in `citation/code.rs`'s `tests` module

```rust
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
    assert_eq!("r3f2a9c1:a.rs#L1".parse::<CodeCitation>(), Err(CitationError::MissingAt));
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
    for s in ["r@3F2A9C1:a.rs#L1", "r@3f2a9c1:a.rs#L010", "r@3f2a9c1:a.rs#L0", "r@3f2a9c1:a.rs#L+1"] {
        assert!(s.parse::<CodeCitation>().is_err(), "{s}");
    }
}

#[test]
fn empty_pieces_are_errors() {
    assert_eq!("@3f2a9c1:a.rs#L1".parse::<CodeCitation>(), Err(CitationError::EmptyName));
    assert_eq!("r@3f2a9c1:#L1".parse::<CodeCitation>(), Err(CitationError::EmptyPath));
    assert_eq!("r@3f2a9c1".parse::<CodeCitation>(), Err(CitationError::MissingPath));
    assert_eq!("r@3f2a9c1:a.rs".parse::<CodeCitation>(), Err(CitationError::MissingLines));
    assert!(matches!("r@3f2a9c1:a.rs#L".parse::<CodeCitation>(), Err(CitationError::BadNumber { .. })));
}

#[test]
fn trailing_junk_is_an_error() {
    for s in ["r@3f2a9c1:a.rs#L4-L9x", "r@3f2a9c1:a.rs#L4-9", "r@3f2a9c1:a.rs#4-L9", "r@3f2a9c1:a.rs#L4-L9-L12"] {
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p codecite-core citation`
Expected: FAIL to compile (`CodeCitation`, `CitationError` not defined).

- [ ] **Step 3: Write `CitationError` and `parse_number` in `citation.rs`**

Variants, each with a `#[error(...)]` message that names the problem and quotes the offending text where there is one. Every variant gets a doc comment.

| Variant | Message |
| --- | --- |
| `MissingAt` | `` no `@` between the name and the version `` |
| `EmptyName` | `` nothing before `@` `` |
| `BadName { found: String }` | `` name {found:?} has a character citations don't allow `` |
| `BadSha { found: String }` | `commit {found:?} is not 7 to 64 lowercase hex digits` |
| `MissingPath` | `` no `:` between the commit and the path `` |
| `EmptyPath` | `empty path` |
| `MissingLines` | `` no `#L` line range after the path `` |
| `BadNumber { what: &'static str, found: String }` | `{what} {found:?} is not a number from 1 without leading zeros` |
| `ReversedRange { start: u32, end: u32 }` | `line range L{start}-L{end} ends before it starts` |
| `SingleLineRange { line: u32 }` | `a single line is written #L{line}, not #L{line}-L{line}` |
| `BadHash { found: String }` | `content hash {found:?} is not 12 lowercase hex digits` |
| `MissingSection` | `` no `#section-` after the hash `` |
| `StepWithoutSection` | `a step needs a section: write #section-<n>/step-<k>` |
| `BadSection { found: String }` | `section {found:?} is not dot-separated numbers like 3.1.3.7` |

`parse_number(s, what)`: reject unless `s` is non-empty, every char is an ASCII digit, and it does not start with `'0'`; then `s.parse::<u32>()`, mapping any parse error (overflow) to `BadNumber { what, found: s.into() }`. The digit check comes first because `u32::from_str` accepts a leading `+`.

Learn: `#[derive(PartialEq)]` on the error is what lets the tests write `assert_eq!(…, Err(CitationError::MissingAt))`. It is possible here because no variant wraps a foreign error such as `io::Error`, which isn't `PartialEq`. That is also why this is its own enum instead of new variants on the crate's `Error`, whose `Io` and `Sqlite` variants rule out `PartialEq`.

- [ ] **Step 4: Write the module wiring**

In `citation.rs`: `mod code;` and `pub use code::{CodeCitation, CommitSha, LineRange};`. In `lib.rs`: `mod citation;` and `pub use citation::{CitationError, CodeCitation, CommitSha, LineRange};`. Add a sentence to the crate doc comment saying the crate also parses and formats canonical citations.

- [ ] **Step 5: Implement `CommitSha`, `LineRange` and `CodeCitation` in `citation/code.rs`**

- `CommitSha::from_str`: accept 7 to 64 chars, each in `'0'..='9' | 'a'..='f'`. Count with `chars()`, never `len()`, so `é` counts as one char and fails the hex check instead of passing a byte-length check. Otherwise `BadSha`. `Display` writes the string.
- `LineRange::from_str` takes the text after `#`, for example `L412-L455`. Strip `L` (otherwise `MissingLines`). Then `split_once("-")`. With no `-`, it is one line, `parse_number(rest, "line")`. With a `-`, the right side must strip another `L` (otherwise `BadNumber` on the right side), and both sides go through `parse_number`. Then `end < start` gives `ReversedRange`, and `end == start` gives `SingleLineRange`. `Display` writes `L{start}` when `start == end`, else `L{start}-L{end}`.
- `CodeCitation::from_str`: `split_once('@')` (otherwise `MissingAt`). An empty name gives `EmptyName`, and a name with any `char::is_whitespace` gives `BadName`. Then on the rest, `split_once(':')` (otherwise `MissingPath`) gives the sha and the tail. On the tail, `rsplit_once('#')` (otherwise `MissingLines`) gives the path and the fragment. An empty path gives `EmptyPath`. Parse the fragment as a `LineRange`. `Display`: `{repo}@{commit}:{path}#{lines}`.

Learn: `FromStr` is the trait behind `str::parse`. Implementing it means callers write `"…".parse::<CodeCitation>()`, or just `.parse()` when the type is known. `Display` is the inverse, and it gives `to_string()` and `format!("{c}")` for free. Making each piece a newtype with its own `FromStr` and `Display` means `CodeCitation`'s impls are just a split followed by `field.parse()?` calls, plus one `write!`.

Learn: a newtype (`struct CommitSha(String)`) costs nothing at run time. Its field stays private, so the only way to get a `CommitSha` is through `from_str`, and any `CommitSha` value is a valid one. Code that takes a `&CommitSha` never re-checks it, and a commit sha can't be passed where a path is expected.

Learn: a regex would match the whole form in one line (`^([^@]+)@([0-9a-f]{7,64}):(.+)#L(\d+)(?:-L(\d+))?$`), but a failed match only says "no match". Splitting by hand knows which piece failed, and #15 asks for an error that names it. It also needs no new dependency. A regex starts to pay off when the grammar has alternatives and repetition that hand-written splitting would get wrong, as the lenient human forms that `read` will accept probably do.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p codecite-core citation`
Expected: PASS.

- [ ] **Step 7: Run the gates and commit**

Run the full gates from Global Constraints. Expected: all green.

```bash
git add crates/codecite-core/src/citation.rs crates/codecite-core/src/citation/code.rs crates/codecite-core/src/lib.rs
git commit -m "feat(core): parse and format canonical code citations (#15)"
```

---

### Task 2: Spec citations and the `Citation` enum

**Files:**
- Create: `crates/codecite-core/src/citation/spec.rs` (`ContentHash`, `SectionNumber`, `SpecCitation` and their tests)
- Modify: `crates/codecite-core/src/citation.rs` (`Citation` enum and its tests)
- Modify: `crates/codecite-core/src/lib.rs`
- Modify: `docs/design.md` (Interfaces: the citation-forms bullet)

**Interfaces:**
- Consumes: `CitationError`, `parse_number`, `CodeCitation` from Task 1.
- Produces, re-exported from the crate root:
  - `pub struct ContentHash(String)` with `fn as_str(&self) -> &str`
  - `pub struct SectionNumber(Vec<u32>)` with `fn parts(&self) -> &[u32]`
  - `pub struct SpecCitation { doc: String, hash: ContentHash, section: SectionNumber, step: Option<u32> }` with `fn doc(&self) -> &str`, `fn hash(&self) -> &ContentHash`, `fn section(&self) -> &SectionNumber`, `fn step(&self) -> Option<u32>`
  - `pub enum Citation { Code(CodeCitation), Spec(SpecCitation) }`
  - Each: `impl FromStr<Err = CitationError>`, `impl Display`, `#[derive(Debug, Clone, PartialEq, Eq, Hash)]`.

- [ ] **Step 1: Write the failing tests** in `citation/spec.rs`'s `tests` module

```rust
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
        assert!(matches!(s.parse::<SpecCitation>(), Err(CitationError::BadHash { .. })), "{s}");
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
    assert_eq!("@sha256:1a2b3c4d5e6f#section-1".parse::<SpecCitation>(), Err(CitationError::EmptyName));
    assert_eq!("rfc9449@sha256:1a2b3c4d5e6f".parse::<SpecCitation>(), Err(CitationError::MissingSection));
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
```

And in `citation.rs`'s `tests` module:

```rust
use super::*;

#[test]
fn citation_picks_the_code_form() {
    let s = "github.com/panva/openid-client@3f2a9c1:lib/client.js#L412-L455";
    let c: Citation = s.parse().unwrap();
    assert!(matches!(c, Citation::Code(_)));
    assert_eq!(c.to_string(), s);
}

#[test]
fn citation_picks_the_spec_form() {
    let s = "openid-connect-core-1_0@sha256:1a2b3c4d5e6f#section-3.1.3.7/step-13";
    let c: Citation = s.parse().unwrap();
    assert!(matches!(c, Citation::Spec(_)));
    assert_eq!(c.to_string(), s);
}

#[test]
fn citation_without_at_is_an_error() {
    assert_eq!("rfc9449#section-4.3".parse::<Citation>(), Err(CitationError::MissingAt));
}

#[test]
fn citation_with_a_bad_hash_reports_the_hash() {
    assert!(matches!(
        "rfc9449@sha256:1a2b#section-4.3".parse::<Citation>(),
        Err(CitationError::BadHash { .. })
    ));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p codecite-core citation`
Expected: FAIL to compile (`SpecCitation`, `Citation` not defined).

- [ ] **Step 3: Implement `ContentHash`, `SectionNumber` and `SpecCitation` in `citation/spec.rs`**

- `ContentHash::from_str`: exactly 12 chars by `chars().count()`, each lowercase hex. Otherwise `BadHash`. `Display` writes the string.
- `SectionNumber::from_str`: `s.split('.')`, each part through `parse_number(part, "section")`. Any failure, including an empty `s`, becomes `BadSection { found: s.into() }`, so the error names the whole section. Collect into `Result<Vec<u32>, _>`. `Display` joins the parts with `.`.
- `SpecCitation::from_str`: `split_once('@')` (otherwise `MissingAt`). An empty name gives `EmptyName`, and a name with any char outside ASCII alphanumerics and `-_.` gives `BadName`. Strip the `sha256:` prefix from the rest (otherwise `BadHash { found: rest.into() }`). Then `split_once('#')` (otherwise `MissingSection`) gives the hash and the fragment, and the hash is parsed. A fragment starting with `step-` gives `StepWithoutSection`. Otherwise strip `section-` (otherwise `BadSection { found: fragment }`). Then `split_once("/step-")` on what is left: `None` means no step, and `Some((section, step))` parses the step with `parse_number(step, "step")`. A leftover `/` in the section part (`4.3/`, `4.3/x`) fails `SectionNumber`'s digit check and becomes `BadSection`. `Display`: `{doc}@sha256:{hash}#section-{section}`, then `/step-{k}` when there is a step.

Learn: `.collect::<Result<Vec<u32>, _>>()` turns an iterator of `Result`s into one `Result`. It stops at the first `Err`, so `SectionNumber` gets "all parts valid, here they are" or "the first bad part" without writing a loop.

Learn: `Option<u32>` for the step makes "section with no step" a value the type system tracks. `Display` handles it with `if let Some(k) = self.step { write!(f, "/step-{k}")?; }`.

- [ ] **Step 4: Implement `Citation` in `citation.rs`**

`mod spec;` and `pub use spec::{ContentHash, SectionNumber, SpecCitation};`. `Citation::from_str`: `let (_, rest) = s.split_once('@').ok_or(CitationError::MissingAt)?;` and then, if `rest.starts_with("sha256:")`, parse the whole `s` as `SpecCitation`, else as `CodeCitation`, mapping with `Citation::Spec` and `Citation::Code`. `Display` delegates to the inner value. Re-export `Citation`, `ContentHash`, `SectionNumber` and `SpecCitation` from `lib.rs`.

Learn: `.map(Citation::Spec)` works because an enum's tuple variant is also a function, `fn(SpecCitation) -> Citation`. There is no need for a closure such as `|c| Citation::Spec(c)`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p codecite-core citation`
Expected: PASS.

- [ ] **Step 6: Record the canonical grammar in `docs/design.md`**

In the Interfaces section, in the bullet that starts "Code citations look like" (line 288), add one sentence after the first: "The canonical forms parse strictly, so that each span has exactly one spelling: a commit is 7 to 64 lowercase hex digits, a spec hash is 12, numbers have no leading zeros, and one line is `#L10`, not `#L10-L10`."

- [ ] **Step 7: Run the gates and commit**

Run the full gates from Global Constraints. Expected: all green.

```bash
git add crates/codecite-core/src/citation.rs crates/codecite-core/src/citation/spec.rs crates/codecite-core/src/lib.rs docs/design.md
git commit -m "feat(core): parse and format canonical spec citations (#15)"
```
