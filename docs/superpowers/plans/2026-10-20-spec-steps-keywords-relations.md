# Spec Steps, Requirement Keywords and RFC Relations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #24 on top of #23's `split_spec`: each spec section gets its numbered **steps** (addressable as `section-<n>/step-<k>`) and its **strength** (the uppercase BCP 14 keywords it contains, as does each step), and each RFC gets its **relations** ("Obsoletes", "Updates", "Obsoleted by", "Updated by") from its header, with the manifest's hand-written relations winning. Nothing is stored yet: #25 indexes these.

**Architecture:** The keyword scanner goes in `codecite-core` (`Keyword`, `requirement_keywords`), because `cite --claim` (#35) must scan a claim the same way the index scans a section; core depends on no other codecite crate, so the index can use it. In `codecite-index`, the renderer's single tree walk also records the items of each top-level numbered `<ol>` with their line ranges; `split` assigns them to the section they start in (a binary search over the items, which are in document order). An older RFC has no `<ol>`: its steps are found in the section's rendered plain text, as paragraphs numbered `1.`, `2.`, … at one indent. A new `spec/relations.rs` reads the relations from the parsed HTML: the v3 `dl#identifiers` block, or the left column of an older RFC's plain-text header. `SpecRelations::with_manifest` overlays the manifest's `obsoleted_by`/`updated_by`.

**Tech Stack:** Rust 1.99.1 (edition 2024). One new dev-dependency, `proptest` 1 (MIT OR Apache-2.0) in `codecite-core`, with `default-features = false, features = ["std"]` so it does not pull the process-forking test runner; `cargo deny check` passes with no new entries. Prototyped: all gates pass, and so do the six `--ignored` tests against the real documents.

**Spec:** Issue #24; `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` (Spec structure); `docs/design.md#indexing-and-retrieval` (step 3) and the manifest section's sentence on document relations, which this plan updates.

## Global Constraints

- Keywords, uppercase only (RFC 8174): MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD NOT, RECOMMENDED, NOT RECOMMENDED, MAY, OPTIONAL. A keyword is a whole word (letters, digits and `_` make a word, so `MUSTARD` and `MUST1` are not keywords). `MUST NOT`, `SHALL NOT`, `SHOULD NOT` and `NOT RECOMMENDED` are one keyword when only whitespace (line breaks included) separates the two words; `NOT` alone is nothing. A strength lists each keyword once, in the order above.
- A step is an item of a numbered (not lettered) `<ol>` that sits inside no other list, or, in an older RFC, a numbered paragraph: a list starts at `1.`, goes on with each next number at the same indent, and ends at a line not indented past the number. Nested lists stay inside their step's text. A step keeps the number the document writes (`<ol start>` counts); within a section a number is used once, so a later list that reuses it gives no step. A step's text is its own line range of `SpecDocument::text`, with blank lines trimmed from both ends, inside its section's range.
- Relations name documents `rfc<N>`, the corpus's document names for RFCs. OpenID pages have none. The manifest's `obsoleted_by`/`updated_by` replace the header's when set; it has no `obsoletes`/`updates`, so those always come from the header.
- Parsing stays separate from I/O, and spec documents are never committed: fixtures are small hand-written HTML. Only the `#[ignore]` tests fetch, through the pinned cache in `std::env::temp_dir()`.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`. Before the PR, also `cargo test -p codecite-index --lib spec:: -- --ignored --nocapture`, with its output in the PR body.

## Review Focus

- Acceptance, against the real pages (`--ignored`): Core §3.1.3.7 has 13 steps and `auth_time` is in step 13 only (`real_openid_connect_core`); Back-Channel Logout §2.6 has 11 (`real_openid_connect_backchannel_logout`), and RFC 9449 §4.3 has 12, the 12th starting with a paragraph (`real_rfc9449`), both counted by reading the documents. RFC 8252's Appendix A has 5 numbered paragraphs and the header says it updates RFC 6749 (`real_rfc8252`); RFC 9700's v3 identifiers say it updates 6749, 6750 and 6819 (`real_rfc9700`); RFC 6749 obsoletes 5849 (`real_rfc6749`).
- The keyword scanner is the one place a false positive would mislead `cite`: `lowercase_and_partial_words_are_not_keywords` and the four property tests (`lowercase_text_has_no_keywords`, `any_text_scans_without_panicking`, `a_keyword_among_lowercase_words_is_found`, `keywords_joined_by_punctuation_are_all_found`).
- Which lists are steps: `nested_lettered_and_renumbered_lists` (a nested `<ul>` and `<ol>` stay in their step, a lettered list and a bulleted list give no steps, a list restarting at 1 gives none, `start="3"` gives step 3, an empty item and a number past `u32::MAX` give none) and `a_step_belongs_to_the_section_it_is_in`.
- Plain-text steps: `numbered_paragraphs_are_steps_in_older_rfcs` (continuation lines and paragraphs inside a step, body text ending the list, a list not starting at 1, a number out of order ending the list) and `numbered_paragraph_lines`.
- Relations: `text_header` reads only the first `<pre>`'s left column, follows a list wrapped after a trailing comma, and ignores an indented `Obsoletes:` in body text; `v3_identifiers_block` ignores the same classes outside `dl#identifiers`.

---

### Task 1: Find requirement keywords (`codecite-core`)

**Files:**
- Modify: `Cargo.toml`, `crates/codecite-core/Cargo.toml`, `Cargo.lock`
- Create: `crates/codecite-core/src/keyword.rs`
- Modify: `crates/codecite-core/src/lib.rs`

**Interfaces:**
- Produces (for Task 2, #25 and #35): `pub enum Keyword { Must, MustNot, Required, Shall, ShallNot, Should, ShouldNot, Recommended, NotRecommended, May, Optional }` (`Copy`, `Ord`, `Hash`) with `Keyword::ALL`, `fn as_str(self) -> &'static str`, `Display` and `FromStr<Err = UnknownKeyword>`; `pub fn requirement_keywords(text: &str) -> Vec<Keyword>`. Re-exported from the crate root.

- [ ] **Step 1: Add `proptest` as a dev-dependency**

```diff
diff --git a/Cargo.toml b/Cargo.toml
index d18d717..f3de494 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -46,3 +46,4 @@ streaming-iterator = "0.1"
 tempfile = "3"
 scraper = { version = "0.27", default-features = false }
 ego-tree = "0.11"
+proptest = { version = "1", default-features = false, features = ["std"] }
```

```diff
diff --git a/crates/codecite-core/Cargo.toml b/crates/codecite-core/Cargo.toml
index d0e1497..1553f3a 100644
--- a/crates/codecite-core/Cargo.toml
+++ b/crates/codecite-core/Cargo.toml
@@ -13,3 +13,6 @@ workspace = true
 [dependencies]
 rusqlite.workspace = true
 thiserror.workspace = true
+
+[dev-dependencies]
+proptest.workspace = true
```

> **Learn: property tests.** A unit test checks the inputs you thought of; `proptest` generates hundreds of inputs from a *strategy* (a regex like `"[a-z ,.]{0,30}"`, `any::<String>()`, `prop::sample::select(...)`) and checks a property that must hold for all of them. When one fails, it *shrinks* the input to the smallest failing case and prints it. Inside `proptest!`, use `prop_assert!`/`prop_assert_eq!` so a failure shrinks instead of panicking straight away.

- [ ] **Step 2: Write the failing tests**

Create `crates/codecite-core/src/keyword.rs` holding the module doc comment and the `use` lines from Step 3 and this test module, and add the module to `lib.rs`:

```rust
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
```

```diff
diff --git a/crates/codecite-core/src/lib.rs b/crates/codecite-core/src/lib.rs
index 78c4925..cab028b 100644
--- a/crates/codecite-core/src/lib.rs
+++ b/crates/codecite-core/src/lib.rs
@@ -2,8 +2,8 @@
 //! and answers searches against it. It also parses and formats the
 //! canonical code and spec citations, and parses the spec citation forms
 //! people write into unresolved references. It also splits identifiers
-//! into the search terms the index stores. Depends on no other codecite
-//! crate.
+//! into the search terms the index stores, and finds the BCP 14
+//! requirement keywords in spec text. Depends on no other codecite crate.
 
 mod citation;
 pub use citation::{
@@ -14,6 +14,9 @@ pub use citation::{
 mod error;
 pub use error::{Error, Result};
 
+mod keyword;
+pub use keyword::{Keyword, UnknownKeyword, requirement_keywords};
+
 mod store;
 pub use store::{ChunkRecord, DocumentRecord, SCHEMA_VERSION, SourceRecord, Store, SymbolRecord};
```

Run: `cargo test -p codecite-core keyword`
Expected: compile errors, `Keyword` and `requirement_keywords` not found.

- [ ] **Step 3: Write the keyword type and scanner**

Put this above the test module in `keyword.rs`:

```rust
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
```

> **Learn: an enum for a closed vocabulary.** The eleven keywords are a fixed set, so they are an enum, not strings: a `match` on `Keyword` must handle every variant, and the compiler says so when one is missed. Deriving `Ord` sorts variants in declaration order, which is why a `BTreeSet<Keyword>` gives the strength in RFC 2119's order for free.
>
> **Learn: `strum` or a hand-written `FromStr`.** `strum` derives `FromStr`, `Display` and an iterator over variants from attributes. With eleven variants, one `as_str` match plus a `const ALL` array gives all three in a few lines with no proc-macro dependency, and `from_str` is just `ALL.into_iter().find(...)`. `strum` pays off when many enums need this, or variants change often.

Run: `cargo test -p codecite-core keyword`
Expected: 8 passed.

- [ ] **Step 4: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all pass; `advisories ok, bans ok, licenses ok, sources ok`.

```bash
git add Cargo.toml Cargo.lock crates/codecite-core/Cargo.toml crates/codecite-core/src/keyword.rs crates/codecite-core/src/lib.rs
git commit -m "feat(core): find BCP 14 requirement keywords in text (#24)"
```

### Task 2: Steps, strength and relations in `split_spec` (`codecite-index`)

**Files:**
- Modify: `crates/codecite-index/src/spec/render.rs`, `crates/codecite-index/src/spec.rs`, `crates/codecite-index/src/lib.rs`
- Create: `crates/codecite-index/src/spec/relations.rs`

**Interfaces:**
- Consumes: `Keyword` and `requirement_keywords` from Task 1; `DocumentSource` (`obsoleted_by`, `updated_by`) from #17.
- Produces (for #25 indexing and #34 `cite`): `SpecSection` gains `strength: Vec<Keyword>`, `steps: Vec<SpecStep>` and `fn step(&self, number: u32) -> Option<&SpecStep>`; `SpecStep { number: u32, start_line: u32, end_line: u32, text: String, strength: Vec<Keyword> }`; `SpecDocument` gains `relations: SpecRelations`; `SpecRelations { obsoletes, updates, obsoleted_by, updated_by: Vec<String> }` (`Default`) with `fn with_manifest(self, &DocumentSource) -> Self`. `SpecStep` and `SpecRelations` are re-exported from the crate root.

- [ ] **Step 1: Write the failing tests**

Apply the test-module parts of this diff to `crates/codecite-index/src/spec.rs` (the `use Keyword::*;` line, `assert_ranges_match` and `steps`, the new tests, `test_source`, the shorter `fetch_real`, the new assertions in the three real-document tests, the three new real-document tests and `report`), and create `relations.rs` with only its test module. Step 2 applies the rest.

Run: `cargo test -p codecite-index spec::`
Expected: compile errors: no `steps`, `strength`, `relations`, `SpecRelations`, `numbered_paragraph`.

- [ ] **Step 2: Record list items in the renderer**

```diff
diff --git a/crates/codecite-index/src/spec/render.rs b/crates/codecite-index/src/spec/render.rs
index 113397f..89eaf81 100644
--- a/crates/codecite-index/src/spec/render.rs
+++ b/crates/codecite-index/src/spec/render.rs
@@ -7,12 +7,28 @@ use scraper::{CaseSensitivity, ElementRef, Html, Node, Selector};
 
 use super::SpecFormat;
 
-/// The page as lines, and every heading in it in document order.
+/// The page as lines, and every heading and numbered list item in it in
+/// document order.
 pub(super) struct Rendered {
     /// Lines without line endings. Blank lines are empty strings, and two
     /// never follow each other.
     pub(super) lines: Vec<String>,
     pub(super) headings: Vec<Heading>,
+    /// The items of each numbered `<ol>` that is not inside another list.
+    /// An older RFC's steps are plain text, so it has none.
+    pub(super) items: Vec<Item>,
+}
+
+/// An item of a numbered list: a step, if it falls inside a section.
+pub(super) struct Item {
+    /// The number written before the item: its list's `start` plus its
+    /// position.
+    pub(super) number: u64,
+    /// The item's first line in [`Rendered::lines`], counting from 0. It
+    /// may be blank when the item starts with a paragraph.
+    pub(super) start: usize,
+    /// The line after the item's last.
+    pub(super) end: usize,
 }
 
 /// A heading: the start of a section, or of an unnumbered part such as
@@ -57,6 +73,7 @@ pub(super) fn render(html: &Html, format: SpecFormat) -> Rendered {
     let mut out = Writer::default();
     let mut headings = Vec::new();
     let mut lists: Vec<List> = Vec::new();
+    let mut items = Items::default();
     let mut pre_depth = 0usize;
     let mut skipping: Option<NodeId> = None;
 
@@ -75,7 +92,7 @@ pub(super) fn render(html: &Html, format: SpecFormat) -> Rendered {
                             skipping = Some(node.id());
                             continue;
                         }
-                        open(el, &mut out, &mut lists, &mut pre_depth);
+                        open(el, &mut out, &mut lists, &mut items, &mut pre_depth);
                         if let Some(numbered) = heading(format, el) {
                             headings.push(Heading {
                                 line: out.next_line(),
@@ -92,7 +109,7 @@ pub(super) fn render(html: &Html, format: SpecFormat) -> Rendered {
                 } else if skipping.is_none()
                     && let Some(el) = ElementRef::wrap(node)
                 {
-                    close(el, &mut out, &mut lists, &mut pre_depth);
+                    close(el, &mut out, &mut lists, &mut items, &mut pre_depth);
                 }
             }
         }
@@ -105,7 +122,11 @@ pub(super) fn render(html: &Html, format: SpecFormat) -> Rendered {
     while lines.last().is_some_and(String::is_empty) {
         lines.pop();
     }
-    Rendered { lines, headings }
+    Rendered {
+        lines,
+        headings,
+        items: items.done,
+    }
 }
 
 /// Elements whose text is not part of the spec: page furniture, scripts,
@@ -135,7 +156,27 @@ enum List {
     Unordered,
 }
 
-fn open(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut usize) {
+/// The numbered list items written so far, and the one being written.
+#[derive(Default)]
+struct Items {
+    /// The open item's number and first line.
+    open: Option<(u64, usize)>,
+    done: Vec<Item>,
+}
+
+/// Whether an `<li>` opening or closing now is an item of a numbered list
+/// that sits inside no other list: `lists` holds only its `<ol>`.
+fn is_step_item(lists: &[List]) -> bool {
+    matches!(lists, [List::Ordered { letters: None, .. }])
+}
+
+fn open(
+    el: ElementRef,
+    out: &mut Writer,
+    lists: &mut Vec<List>,
+    items: &mut Items,
+    pre_depth: &mut usize,
+) {
     let e = el.value();
     match e.name() {
         "pre" => {
@@ -161,6 +202,11 @@ fn open(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut
         }
         "li" => {
             out.end_line();
+            if is_step_item(lists)
+                && let Some(List::Ordered { next, .. }) = lists.last()
+            {
+                items.open = Some((*next, out.next_line()));
+            }
             match lists.last_mut() {
                 Some(List::Ordered { next, letters }) => {
                     let label = match *letters {
@@ -183,7 +229,13 @@ fn open(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut
     }
 }
 
-fn close(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mut usize) {
+fn close(
+    el: ElementRef,
+    out: &mut Writer,
+    lists: &mut Vec<List>,
+    items: &mut Items,
+    pre_depth: &mut usize,
+) {
     match el.value().name() {
         "pre" => {
             *pre_depth = pre_depth.saturating_sub(1);
@@ -198,6 +250,15 @@ fn close(el: ElementRef, out: &mut Writer, lists: &mut Vec<List>, pre_depth: &mu
             // it can't land on the text after the list.
             out.label = None;
             out.end_line();
+            if is_step_item(lists)
+                && let Some((number, start)) = items.open.take()
+            {
+                items.done.push(Item {
+                    number,
+                    start,
+                    end: out.next_line(),
+                });
+            }
         }
         "dt" | "dd" | "tr" => out.end_line(),
         name if is_block(name) => out.end_paragraph(),
```

`is_step_item` checks the open-list stack with a slice pattern: `[List::Ordered { letters: None, .. }]` matches a stack of exactly one numbered list, so an `<li>` in a nested list, or in a lettered or bulleted one, is not a step. The stack at an `<li>`'s close is the one at its open, because nested lists pop themselves first.

- [ ] **Step 3: Read relations from the header**

Create `crates/codecite-index/src/spec/relations.rs`:

```rust
//! Reads an RFC's relations to other RFCs from its header.

use scraper::{Html, Selector};

use super::{SpecFormat, SpecRelations};

/// The relations an RFC's header lists. OpenID specs have none.
pub(super) fn relations(html: &Html, format: SpecFormat) -> SpecRelations {
    match format {
        SpecFormat::RfcV3 => v3_relations(html),
        SpecFormat::RfcText => text_relations(html),
        SpecFormat::OpenId => SpecRelations::default(),
    }
}

/// An xml2rfc v3 RFC lists them in its identifiers block:
/// `<dl id="identifiers">…<dt class="label-updates">Updates:</dt>
/// <dd class="updates"><a href="…/rfc6749">6749</a>, …</dd>`.
fn v3_relations(html: &Html) -> SpecRelations {
    let field = |class: &str| {
        let selector =
            Selector::parse(&format!("dl#identifiers > dd.{class}")).expect("a valid selector");
        html.select(&selector)
            .flat_map(|dd| rfc_names(&dd.text().collect::<String>()))
            .collect()
    };
    SpecRelations {
        obsoletes: field("obsoletes"),
        updates: field("updates"),
        obsoleted_by: field("obsoleted-by"),
        updated_by: field("updated-by"),
    }
}

/// An older RFC writes them in the left column of the plain-text header,
/// the first `<pre>` on the page:
///
/// ```text
/// Request for Comments: 8252                                        Google
/// Updates: 6749                                              Ping Identity
/// ```
///
/// A list too long for one line goes on in the next lines, indented.
fn text_relations(html: &Html) -> SpecRelations {
    let mut relations = SpecRelations::default();
    let selector = Selector::parse("pre").expect("a valid selector");
    let Some(pre) = html.select(&selector).next() else {
        return relations;
    };
    let header = pre.text().collect::<String>();
    let mut field: Option<&mut Vec<String>> = None;
    for line in header.lines() {
        // The left column ends at the first run of two or more spaces.
        let left = line.trim_end().split("  ").next().unwrap_or_default();
        let value = if let Some((name, value)) = left.split_once(':') {
            field = match name {
                "Obsoletes" => Some(&mut relations.obsoletes),
                "Updates" => Some(&mut relations.updates),
                "Obsoleted by" => Some(&mut relations.obsoleted_by),
                "Updated by" => Some(&mut relations.updated_by),
                _ => None,
            };
            value
        } else if line.starts_with(' ') {
            // A continuation line: kept only after a field ending in ','.
            line.trim()
        } else {
            field = None;
            continue;
        };
        let Some(names) = field.as_deref_mut() else {
            continue;
        };
        names.extend(rfc_names(value));
        if !value.trim_end().ends_with(',') {
            field = None;
        }
    }
    relations
}

/// `rfc6749` for each run of digits in `6749, 6750 (if approved)`.
fn rfc_names(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty() && !n.starts_with('0'))
        .map(|n| format!("rfc{n}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn v3_identifiers_block() {
        let html = Html::parse_document(
            "<dl id=\"identifiers\"><dt class=\"label-rfc\">RFC:</dt><dd class=\"rfc\">9700</dd>\
             <dt class=\"label-obsoletes\">Obsoletes:</dt><dd class=\"obsoletes\">\
             <a href=\"https://www.rfc-editor.org/rfc/rfc6819\">6819</a></dd>\
             <dt class=\"label-updates\">Updates:</dt><dd class=\"updates\">\
             <a href=\"https://www.rfc-editor.org/rfc/rfc6749\">6749</a>, \
             <a href=\"https://www.rfc-editor.org/rfc/rfc6750\">6750</a> </dd></dl>\
             <p class=\"updates\">Updates: 1234</p>",
        );
        assert_eq!(
            relations(&html, SpecFormat::RfcV3),
            SpecRelations {
                obsoletes: names(&["rfc6819"]),
                updates: names(&["rfc6749", "rfc6750"]),
                obsoleted_by: Vec::new(),
                updated_by: Vec::new(),
            }
        );
    }

    #[test]
    fn text_header() {
        let html = Html::parse_document(
            "<pre>Internet Engineering Task Force (IETF)          D. Hardt, Ed.\n\
             Request for Comments: 6749                        Microsoft\n\
             Obsoletes: <a href=\"./rfc5849\">5849</a>                    October 2012\n\
             Updates: 1034, 1035, 1123,                        Someone\n\
             \x20        2181\n\
             Updated by: 8252\n\
             Category: Standards Track\n\
             ISSN: 2070-1721\n\
             \n   Obsoletes: 1111 is body text.</pre><pre>Updates: 2222</pre>",
        );
        assert_eq!(
            relations(&html, SpecFormat::RfcText),
            SpecRelations {
                obsoletes: names(&["rfc5849"]),
                updates: names(&["rfc1034", "rfc1035", "rfc1123", "rfc2181"]),
                obsoleted_by: Vec::new(),
                updated_by: names(&["rfc8252"]),
            }
        );
    }

    #[test]
    fn openid_pages_have_none() {
        let html = Html::parse_document("<pre>Updates: 6749</pre>");
        assert_eq!(
            relations(&html, SpecFormat::OpenId),
            SpecRelations::default()
        );
    }

    #[test]
    fn rfc_names_skip_text_and_leading_zeros() {
        assert_eq!(
            rfc_names(" 6749, 6750 (if approved) 0123"),
            names(&["rfc6749", "rfc6750"])
        );
        assert!(rfc_names("none").is_empty());
    }
}
```

`field` is an `Option<&mut Vec<String>>` pointing at whichever relation the current header line fills, so a continuation line can keep filling it; `as_deref_mut` borrows the `Vec` out of it without moving the `Option`.

- [ ] **Step 4: Build steps and strength in `split`**

```diff
diff --git a/crates/codecite-index/src/spec.rs b/crates/codecite-index/src/spec.rs
index 58a3de1..ce71e42 100644
--- a/crates/codecite-index/src/spec.rs
+++ b/crates/codecite-index/src/spec.rs
@@ -1,11 +1,19 @@
-//! Spec documents: splits RFC and OpenID HTML into numbered sections over a
-//! plain-text rendering of the page. This module only parses: the caller
-//! fetches the bytes (see [`Fetcher`](crate::Fetcher)) and passes the HTML in.
+//! Spec documents: splits RFC and OpenID HTML into numbered sections and
+//! the numbered steps inside them, over a plain-text rendering of the page,
+//! and reads an RFC's relations to other RFCs from its header. This module
+//! only parses: the caller fetches the bytes (see [`Fetcher`](crate::Fetcher))
+//! and passes the HTML in.
 
+mod relations;
 mod render;
 
+use std::collections::HashSet;
+
+use codecite_core::{Keyword, requirement_keywords};
 use scraper::Html;
 
+use crate::DocumentSource;
+
 /// The HTML layouts [`split_spec`] reads.
 #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
 pub enum SpecFormat {
@@ -40,6 +48,67 @@ pub struct SpecSection {
     pub end_line: u32,
     /// Lines `start_line..=end_line` of [`SpecDocument::text`].
     pub text: String,
+    /// The requirement keywords in [`SpecSection::text`], steps included.
+    pub strength: Vec<Keyword>,
+    /// The items of the section's numbered lists, in order.
+    pub steps: Vec<SpecStep>,
+}
+
+impl SpecSection {
+    /// The step numbered `number`, as `section-<n>/step-<k>` cites it.
+    pub fn step(&self, number: u32) -> Option<&SpecStep> {
+        self.steps.iter().find(|s| s.number == number)
+    }
+}
+
+/// One item of a numbered list inside a section: step 13 of Core
+/// §3.1.3.7.
+///
+/// The items of an `<ol>` are steps, and so, in an older RFC's plain text,
+/// are paragraphs numbered `1.`, `2.`, … at one indent. Lists nested in a
+/// step stay part of its text. Step numbers are unique within a section: a
+/// later list that numbers an item again gives no step for it.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct SpecStep {
+    /// The number the document writes before the step.
+    pub number: u32,
+    /// The step's first line in [`SpecDocument::text`], counting from 1.
+    pub start_line: u32,
+    /// The step's last line, inclusive.
+    pub end_line: u32,
+    /// Lines `start_line..=end_line` of [`SpecDocument::text`], starting
+    /// with the number: `13. If the auth_time Claim was requested, …`.
+    pub text: String,
+    /// The requirement keywords in [`SpecStep::text`].
+    pub strength: Vec<Keyword>,
+}
+
+/// How a document relates to other documents, by name: `rfc6749`.
+#[derive(Debug, Clone, Default, PartialEq, Eq)]
+pub struct SpecRelations {
+    /// The documents this one replaces.
+    pub obsoletes: Vec<String>,
+    /// The documents this one amends.
+    pub updates: Vec<String>,
+    /// The documents that replace this one.
+    pub obsoleted_by: Vec<String>,
+    /// The documents that amend this one.
+    pub updated_by: Vec<String>,
+}
+
+impl SpecRelations {
+    /// These relations with the manifest's hand-written ones in place of
+    /// what the document says: a document published before its successor
+    /// can't name it, and the manifest names documents as the corpus does.
+    pub fn with_manifest(mut self, source: &DocumentSource) -> Self {
+        if let Some(name) = &source.obsoleted_by {
+            self.obsoleted_by = vec![name.clone()];
+        }
+        if let Some(name) = &source.updated_by {
+            self.updated_by = vec![name.clone()];
+        }
+        self
+    }
 }
 
 /// A spec page split into sections.
@@ -53,6 +122,10 @@ pub struct SpecDocument {
     /// Every numbered section in document order. Front matter, the table
     /// of contents, unnumbered sections and the references are not here.
     pub sections: Vec<SpecSection>,
+    /// The relations the page's header lists: the "Obsoletes", "Updates",
+    /// "Obsoleted by" and "Updated by" fields of an RFC. Empty for an
+    /// OpenID spec. See [`SpecRelations::with_manifest`].
+    pub relations: SpecRelations,
 }
 
 impl SpecDocument {
@@ -71,7 +144,8 @@ pub enum SpecError {
     UnknownFormat,
 }
 
-/// Splits a spec page's HTML into its numbered sections.
+/// Splits a spec page's HTML into its numbered sections and steps, and
+/// reads its relations.
 ///
 /// Parsing never fails on malformed HTML: `html5ever` repairs it the way a
 /// browser does. The only error is a page in none of the [`SpecFormat`]s.
@@ -79,7 +153,9 @@ pub fn split_spec(html: &str) -> Result<SpecDocument, SpecError> {
     let html = Html::parse_document(html);
     let format = render::detect(&html).ok_or(SpecError::UnknownFormat)?;
     let rendered = render::render(&html, format);
-    Ok(split(format, rendered))
+    let mut doc = split(format, rendered);
+    doc.relations = relations::relations(&html, format);
+    Ok(doc)
 }
 
 /// Section titles that start the references, which are not sections.
@@ -93,7 +169,11 @@ const REFERENCES: [&str; 3] = [
 /// the section before it, numbered or not, so an unnumbered heading such as
 /// "Authors' Addresses" keeps its text out of the last section.
 fn split(format: SpecFormat, rendered: render::Rendered) -> SpecDocument {
-    let render::Rendered { lines, headings } = rendered;
+    let render::Rendered {
+        lines,
+        headings,
+        items,
+    } = rendered;
     let mut sections: Vec<SpecSection> = Vec::new();
     let mut excluded: Vec<String> = Vec::new();
     for (i, heading) in headings.iter().enumerate() {
@@ -129,6 +209,11 @@ fn split(format: SpecFormat, rendered: render::Rendered) -> SpecDocument {
         let parent = number
             .rsplit_once('.')
             .and_then(|(up, _)| sections.iter().rposition(|s| s.number == up));
+        let spans = match format {
+            SpecFormat::RfcText => text_steps(&lines[..end], start + 1),
+            SpecFormat::RfcV3 | SpecFormat::OpenId => item_steps(&items, start, end),
+        };
+        let text = lines[start..end].join("\n");
         sections.push(SpecSection {
             number: number.clone(),
             title: numbered.title.clone(),
@@ -136,7 +221,9 @@ fn split(format: SpecFormat, rendered: render::Rendered) -> SpecDocument {
             parent,
             start_line: line_number(start + 1),
             end_line: line_number(end),
-            text: lines[start..end].join("\n"),
+            strength: requirement_keywords(&text),
+            text,
+            steps: steps(&lines, spans),
         });
     }
     let mut text = lines.join("\n");
@@ -145,7 +232,115 @@ fn split(format: SpecFormat, rendered: render::Rendered) -> SpecDocument {
         format,
         text,
         sections,
+        relations: SpecRelations::default(),
+    }
+}
+
+/// A step's number and line range, `start..end` in the rendered lines,
+/// before blank lines are trimmed from either end.
+type StepSpan = (u64, usize, usize);
+
+/// The list items that start between lines `start` and `end`, cut off at
+/// `end`. `items` is in document order, so a binary search finds them.
+fn item_steps(items: &[render::Item], start: usize, end: usize) -> Vec<StepSpan> {
+    let first = items.partition_point(|item| item.start < start);
+    items[first..]
+        .iter()
+        .take_while(|item| item.start < end)
+        .map(|item| (item.number, item.start, item.end.min(end)))
+        .collect()
+}
+
+/// The numbered paragraphs in `lines[from..]`, a section of an older RFC
+/// after its heading:
+///
+/// ```text
+///    1.  Support private-use URI scheme redirect URIs.  This is required
+///        to support mobile operating systems.  See Section 7.1.
+///
+///    2.  Support "https" scheme redirect URIs …
+/// ```
+///
+/// A list starts at a `1.` and goes on with each next number at the same
+/// indent. A step runs to the next step, or to the first line that is not
+/// indented past the step's number, which ends the list.
+fn text_steps(lines: &[String], from: usize) -> Vec<StepSpan> {
+    let mut steps = Vec::new();
+    // The open list's indent and next number, and the open step.
+    let mut list: Option<(usize, u64)> = None;
+    let mut open: Option<(u64, usize)> = None;
+    for (n, line) in lines.iter().enumerate().skip(from) {
+        if line.is_empty() {
+            continue;
+        }
+        let numbered = numbered_paragraph(line);
+        let next_step = match (list, numbered) {
+            (None, Some((_, 1))) => numbered,
+            (Some((indent, next)), Some(found)) if found == (indent, next) => numbered,
+            _ => None,
+        };
+        if let Some((indent, number)) = next_step {
+            steps.extend(open.map(|(k, start)| (k, start, n)));
+            open = Some((number, n));
+            list = Some((indent, number + 1));
+        } else if let Some((indent, _)) = list
+            && indent_of(line) <= indent
+        {
+            steps.extend(open.take().map(|(k, start)| (k, start, n)));
+            list = None;
+        }
+    }
+    steps.extend(open.map(|(k, start)| (k, start, lines.len())));
+    steps
+}
+
+/// `Some((3, 2))` for `   2.  Support …`: the indent and the number of a
+/// line that starts with a number, a dot and a space, then text.
+fn numbered_paragraph(line: &str) -> Option<(usize, u64)> {
+    let indent = indent_of(line);
+    let rest = &line[indent..];
+    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
+    let after = rest[digits..].strip_prefix(". ")?;
+    if digits == 0 || after.trim_start().is_empty() || rest.starts_with('0') {
+        return None;
+    }
+    Some((indent, rest[..digits].parse().ok()?))
+}
+
+/// The number of spaces a line starts with.
+fn indent_of(line: &str) -> usize {
+    line.len() - line.trim_start_matches(' ').len()
+}
+
+/// Builds the steps from their spans: trims blank lines from each end, and
+/// drops a span left empty, numbered past `u32::MAX`, or numbered like an
+/// earlier step.
+fn steps(lines: &[String], spans: Vec<StepSpan>) -> Vec<SpecStep> {
+    let mut steps = Vec::new();
+    let mut taken = HashSet::new();
+    for (number, mut start, mut end) in spans {
+        while start < end && lines[start].is_empty() {
+            start += 1;
+        }
+        while end > start && lines[end - 1].is_empty() {
+            end -= 1;
+        }
+        let Ok(number) = u32::try_from(number) else {
+            continue;
+        };
+        if start == end || !taken.insert(number) {
+            continue;
+        }
+        let text = lines[start..end].join("\n");
+        steps.push(SpecStep {
+            number,
+            start_line: line_number(start + 1),
+            end_line: line_number(end),
+            strength: requirement_keywords(&text),
+            text,
+        });
     }
+    steps
 }
 
 /// A line count as the `u32` that chunks store. A page under the 16 MiB
@@ -157,6 +352,7 @@ fn line_number(n: usize) -> u32 {
 #[cfg(test)]
 mod tests {
     use super::*;
+    use Keyword::*;
 
     /// Each section as (number, title, anchor, parent number).
     fn outline(doc: &SpecDocument) -> Vec<(&str, &str, &str, Option<&str>)> {
@@ -174,15 +370,37 @@ mod tests {
             .collect()
     }
 
-    /// Every section's text is its own line range of the document text.
+    /// Every section's and step's text is its own line range of the
+    /// document text, and a step's lines are inside its section's.
     fn assert_ranges_match(doc: &SpecDocument) {
         let lines: Vec<&str> = doc.text.lines().collect();
         for s in &doc.sections {
             let range = &lines[s.start_line as usize - 1..s.end_line as usize];
             assert_eq!(range.join("\n"), s.text, "§{}", s.number);
+            for step in &s.steps {
+                let range = &lines[step.start_line as usize - 1..step.end_line as usize];
+                assert_eq!(
+                    range.join("\n"),
+                    step.text,
+                    "§{} step {}",
+                    s.number,
+                    step.number
+                );
+                assert!(s.start_line < step.start_line && step.end_line <= s.end_line);
+            }
         }
     }
 
+    /// Each step of `section` as (number, text).
+    fn steps<'a>(doc: &'a SpecDocument, section: &str) -> Vec<(u32, &'a str)> {
+        doc.section(section)
+            .unwrap()
+            .steps
+            .iter()
+            .map(|s| (s.number, s.text.as_str()))
+            .collect()
+    }
+
     #[test]
     fn rfc_v3_fixture() {
         let doc = split_spec(include_str!("../tests/fixtures/spec/rfc-v3.html")).unwrap();
@@ -364,22 +582,191 @@ mod tests {
         assert_ranges_match(&doc);
     }
 
-    /// A real spec page, fetched through the pinned cache in the temp
-    /// directory, so only the first run downloads it.
-    fn fetch_real(name: &str, url: &str, sha256: &str) -> String {
-        use crate::{DocumentSource, Fetcher, LicenseClass};
-        let doc = DocumentSource {
+    #[test]
+    fn list_items_are_steps() {
+        let v3 = split_spec(include_str!("../tests/fixtures/spec/rfc-v3.html")).unwrap();
+        assert_eq!(
+            steps(&v3, "4.3"),
+            [
+                (
+                    1,
+                    "1. There is not more than one DPoP HTTP request header field."
+                ),
+                (
+                    2,
+                    "2. The typ JOSE Header Parameter has the value dpop+jwt."
+                ),
+            ]
+        );
+        assert_eq!(v3.section("4.3").unwrap().strength, [Must]);
+        assert!(v3.section("4.3.1").unwrap().steps.is_empty());
+
+        let openid = split_spec(include_str!("../tests/fixtures/spec/openid.html")).unwrap();
+        let s = openid.section("3.1.3.7").unwrap();
+        assert_eq!(s.steps.len(), 2);
+        assert_eq!(s.strength, [Must]);
+        assert_eq!(s.step(1).unwrap().strength, []);
+        assert_eq!(s.step(2).unwrap().strength, [Must]);
+        assert!(
+            s.step(2)
+                .unwrap()
+                .text
+                .starts_with("2. The Issuer Identifier")
+        );
+        assert_eq!(s.step(3), None);
+    }
+
+    #[test]
+    fn nested_lettered_and_renumbered_lists() {
+        let html = "<section id=\"section-1\"><h2><a class=\"section-number\">1. </a>Steps</h2>\
+                    <ol><li>One MUST.<ul><li>inner</li></ul></li>\
+                    <li><p>Two starts with a paragraph.</p><p>And SHOULD go on.</p>\
+                    <ol><li>nested</li></ol></li></ol>\
+                    <p>Between the lists.</p>\
+                    <ol type=\"a\"><li>lettered</li></ol>\
+                    <ul><li>bulleted</li></ul>\
+                    <ol><li>restarted at one</li></ol>\
+                    <ol start=\"3\"><li>three</li><li></li></ol>\
+                    <ol start=\"4294967296\"><li>too big</li></ol>\
+                    </section>";
+        let doc = split_spec(html).unwrap();
+        assert_ranges_match(&doc);
+        assert_eq!(
+            steps(&doc, "1"),
+            [
+                (1, "1. One MUST.\n\n- inner"),
+                (
+                    2,
+                    "2. Two starts with a paragraph.\n\nAnd SHOULD go on.\n\n1. nested"
+                ),
+                (3, "3. three"),
+            ]
+        );
+        let s = doc.section("1").unwrap();
+        assert_eq!(s.step(1).unwrap().strength, [Must]);
+        assert_eq!(s.step(2).unwrap().strength, [Should]);
+        assert_eq!(s.strength, [Must, Should]);
+    }
+
+    #[test]
+    fn a_step_belongs_to_the_section_it_is_in() {
+        let html = "<section id=\"section-1\"><h2><a class=\"section-number\">1. </a>One</h2>\
+                    <ol><li>first</li></ol>\
+                    <section id=\"section-1.1\"><h3><a class=\"section-number\">1.1. </a>Sub</h3>\
+                    <ol><li>sub first</li><li>sub second</li></ol></section></section>";
+        let doc = split_spec(html).unwrap();
+        assert_ranges_match(&doc);
+        assert_eq!(steps(&doc, "1"), [(1, "1. first")]);
+        assert_eq!(
+            steps(&doc, "1.1"),
+            [(1, "1. sub first"), (2, "2. sub second")]
+        );
+    }
+
+    #[test]
+    fn numbered_paragraphs_are_steps_in_older_rfcs() {
+        let html = "<pre><span class=\"h2\"><a class=\"selflink\" id=\"section-1\">1</a>.  Checks</span>\n\
+                    \n   Servers MUST:\n\
+                    \n   1.  Support private-use URI scheme redirect URIs.  This is\n\
+                    \x20      required.\n\
+                    \n       Still step one.\n\
+                    \n   2.  Support PKCE.  It SHOULD be used.\n\
+                    \n   Body text ends the list.\n\
+                    \n   3.  Not a step: its list does not start at 1.\n\
+                    \n<span class=\"h2\"><a class=\"selflink\" id=\"section-2\">2</a>.  Next</span>\n\
+                    \n   1.  Last step.\n\
+                    \x20  2. One space after the dot is enough.\n\
+                    \x20  10.  Not next in order.\n</pre>";
+        let doc = split_spec(html).unwrap();
+        assert_eq!(doc.format, SpecFormat::RfcText);
+        assert_ranges_match(&doc);
+        assert_eq!(
+            steps(&doc, "1"),
+            [
+                (
+                    1,
+                    "   1.  Support private-use URI scheme redirect URIs.  This is\n       required.\n\n       Still step one."
+                ),
+                (2, "   2.  Support PKCE.  It SHOULD be used."),
+            ]
+        );
+        assert_eq!(
+            doc.section("1").unwrap().step(2).unwrap().strength,
+            [Should]
+        );
+        assert_eq!(
+            steps(&doc, "2"),
+            [
+                (1, "   1.  Last step."),
+                // `10.` is at the list's indent but not next, so it ends
+                // the list.
+                (2, "   2. One space after the dot is enough."),
+            ]
+        );
+    }
+
+    #[test]
+    fn numbered_paragraph_lines() {
+        let line = |s: &str| numbered_paragraph(s);
+        assert_eq!(line("   2.  Support"), Some((3, 2)));
+        assert_eq!(line("1. x"), Some((0, 1)));
+        assert_eq!(line("   2.Support"), None);
+        assert_eq!(line("   2.  "), None);
+        assert_eq!(line("   02.  x"), None);
+        assert_eq!(line("   .  x"), None);
+        assert_eq!(line("   4.3.  x"), None);
+        assert_eq!(line("   99999999999999999999999.  x"), None);
+    }
+
+    #[test]
+    fn relations_from_the_header_and_the_manifest() {
+        let doc = split_spec(include_str!("../tests/fixtures/spec/rfc-text.html")).unwrap();
+        let header = SpecRelations {
+            obsoletes: vec!["rfc5849".into()],
+            ..SpecRelations::default()
+        };
+        assert_eq!(doc.relations, header);
+        let mut source = test_source("rfc6749");
+        assert_eq!(doc.relations.clone().with_manifest(&source), header);
+        source.obsoleted_by = Some("rfc9999".into());
+        source.updated_by = Some("rfc8252".into());
+        assert_eq!(
+            doc.relations.clone().with_manifest(&source),
+            SpecRelations {
+                obsoletes: vec!["rfc5849".into()],
+                updates: Vec::new(),
+                obsoleted_by: vec!["rfc9999".into()],
+                updated_by: vec!["rfc8252".into()],
+            }
+        );
+        let openid = split_spec(include_str!("../tests/fixtures/spec/openid.html")).unwrap();
+        assert_eq!(openid.relations, SpecRelations::default());
+    }
+
+    /// A document source with no relations.
+    fn test_source(name: &str) -> DocumentSource {
+        DocumentSource {
             name: name.into(),
-            url: url.into(),
-            sha256: sha256.into(),
+            url: format!("https://www.rfc-editor.org/rfc/{name}.html"),
+            sha256: "0".repeat(64),
             license: None,
-            license_class: LicenseClass::Quote,
+            license_class: crate::LicenseClass::Quote,
             current: true,
             aliases: Vec::new(),
             obsoleted_by: None,
             updated_by: None,
+        }
+    }
+
+    /// A real spec page, fetched through the pinned cache in the temp
+    /// directory, so only the first run downloads it.
+    fn fetch_real(name: &str, url: &str, sha256: &str) -> String {
+        let doc = DocumentSource {
+            url: url.into(),
+            sha256: sha256.into(),
+            ..test_source(name)
         };
-        let fetcher = Fetcher::new(std::env::temp_dir().join("codecite-documents"));
+        let fetcher = crate::Fetcher::new(std::env::temp_dir().join("codecite-documents"));
         String::from_utf8(fetcher.fetch_pinned(&doc).unwrap().bytes).unwrap()
     }
 
@@ -407,7 +794,11 @@ mod tests {
         );
         assert!(doc.section("13").is_none(), "References");
         assert!(doc.sections.iter().all(|s| s.number != "13.1"));
-        println!("RFC 9449: {} sections", doc.sections.len());
+        assert_eq!(s.steps.len(), 12);
+        assert!(s.step(12).unwrap().text.starts_with("12. If presented"));
+        assert_eq!(s.strength, [Must, Should]);
+        assert_eq!(doc.relations, SpecRelations::default());
+        report("RFC 9449", &doc);
     }
 
     #[test]
@@ -430,7 +821,14 @@ mod tests {
         assert!(!doc.text.contains("[Page "), "page footers are dropped");
         assert!(doc.section("12").is_none(), "References");
         assert!(!doc.sections.last().unwrap().text.contains("Hardt (editor)"));
-        println!("RFC 6749: {} sections", doc.sections.len());
+        assert_eq!(
+            doc.relations,
+            SpecRelations {
+                obsoletes: vec!["rfc5849".into()],
+                ..SpecRelations::default()
+            }
+        );
+        report("RFC 6749", &doc);
     }
 
     #[test]
@@ -457,7 +855,105 @@ mod tests {
         );
         assert!(!doc.text.contains("TOC"), "the TOC link boxes are dropped");
         assert!(doc.section("19").is_none(), "References");
-        println!("Core: {} sections", doc.sections.len());
+        assert_eq!(s.steps.len(), 13);
+        let step = s.step(13).unwrap();
+        assert!(step.text.contains("auth_time"), "{}", step.text);
+        assert_eq!(step.strength, [Should]);
+        assert!(s.steps[..12].iter().all(|s| !s.text.contains("auth_time")));
+        assert_eq!(doc.relations, SpecRelations::default());
+        report("Core", &doc);
+    }
+
+    #[test]
+    #[ignore = "downloads OpenID Connect Back-Channel Logout 1.0; run with `cargo test -- --ignored`"]
+    fn real_openid_connect_backchannel_logout() {
+        let html = fetch_real(
+            "openid-connect-backchannel-1_0",
+            "https://openid.net/specs/openid-connect-backchannel-1_0.html",
+            "3b5c933d5269047b39a65166d5c4d8ff6852b06451f54ed2531b8f97299fdc50",
+        );
+        let doc = split_spec(&html).unwrap();
+        assert_eq!(doc.format, SpecFormat::OpenId);
+        assert_ranges_match(&doc);
+        let s = doc.section("2.6").unwrap();
+        assert_eq!(s.title, "Logout Token Validation");
+        // Validation steps 1 to 11, the last three optional checks.
+        assert_eq!(s.steps.len(), 11);
+        assert!(
+            s.step(7)
+                .unwrap()
+                .text
+                .contains("does not contain a nonce Claim")
+        );
+        assert!(s.step(11).unwrap().text.contains("sid"));
+        assert_eq!(s.step(3).unwrap().strength, [MustNot, Should]);
+        report("Back-Channel Logout", &doc);
+    }
+
+    #[test]
+    #[ignore = "downloads RFC 8252; run with `cargo test -- --ignored`"]
+    fn real_rfc8252() {
+        let html = fetch_real(
+            "rfc8252",
+            "https://www.rfc-editor.org/rfc/rfc8252.html",
+            "fdda8f9e27f3bb4bb119742f4752df5bdfa9f8d9d3a6595ea0c3f581e13bfc79",
+        );
+        let doc = split_spec(&html).unwrap();
+        assert_eq!(doc.format, SpecFormat::RfcText);
+        assert_ranges_match(&doc);
+        // Appendix A, "Server Support Checklist", numbers its paragraphs.
+        let s = doc.section("A").unwrap();
+        assert_eq!(s.steps.len(), 5);
+        assert!(s.step(5).unwrap().text.contains("Support PKCE [RFC7636]."));
+        assert!(s.step(5).unwrap().text.ends_with("See Section 8.1"));
+        assert_eq!(
+            doc.relations,
+            SpecRelations {
+                updates: vec!["rfc6749".into()],
+                ..SpecRelations::default()
+            }
+        );
+        report("RFC 8252", &doc);
+    }
+
+    #[test]
+    #[ignore = "downloads RFC 9700; run with `cargo test -- --ignored`"]
+    fn real_rfc9700() {
+        let html = fetch_real(
+            "rfc9700",
+            "https://www.rfc-editor.org/rfc/rfc9700.html",
+            "c00e2234fcd79581bc29669c60d693afaa42f67d9205bddee848f757ae975419",
+        );
+        let doc = split_spec(&html).unwrap();
+        assert_eq!(doc.format, SpecFormat::RfcV3);
+        assert_ranges_match(&doc);
+        assert_eq!(
+            doc.relations,
+            SpecRelations {
+                updates: vec!["rfc6749".into(), "rfc6750".into(), "rfc6819".into()],
+                ..SpecRelations::default()
+            }
+        );
+        report("RFC 9700", &doc);
+    }
+
+    /// Prints how many sections and steps a real document has, and the
+    /// sections with the most steps, for the PR description.
+    fn report(name: &str, doc: &SpecDocument) {
+        let steps: usize = doc.sections.iter().map(|s| s.steps.len()).sum();
+        let mut most: Vec<_> = doc
+            .sections
+            .iter()
+            .filter(|s| !s.steps.is_empty())
+            .map(|s| (s.steps.len(), s.number.as_str()))
+            .collect();
+        most.sort_by(|a, b| b.cmp(a));
+        most.truncate(5);
+        println!(
+            "{name}: {} sections, {steps} steps; most steps: {most:?}; relations: {:?}",
+            doc.sections.len(),
+            doc.relations
+        );
     }
 
     #[test]
```

```diff
diff --git a/crates/codecite-index/src/lib.rs b/crates/codecite-index/src/lib.rs
index 8ea0455..a3f6a5e 100644
--- a/crates/codecite-index/src/lib.rs
+++ b/crates/codecite-index/src/lib.rs
@@ -34,7 +34,9 @@ mod test_repo;
 mod test_server;
 
 mod spec;
-pub use spec::{SpecDocument, SpecError, SpecFormat, SpecSection, split_spec};
+pub use spec::{
+    SpecDocument, SpecError, SpecFormat, SpecRelations, SpecSection, SpecStep, split_spec,
+};
 
 mod walk;
 pub use walk::{
```

`items.partition_point(|item| item.start < start)` is a binary search: the items are in document order, so it returns the first item at or after the section's heading in O(log n), and `take_while` stops at the section's end; walking every item for every section would be quadratic. `HashSet::insert` returns `false` for a value already present, which is the whole duplicate-number check. `..test_source(name)` is struct update syntax: the fields not written take their values from another instance.

Run: `cargo test -p codecite-index spec::`
Expected: 24 passed, 6 ignored.

- [ ] **Step 5: Run against the real documents**

Run: `cargo test -p codecite-index --lib spec:: -- --ignored --nocapture`
Expected: 6 passed, printing `Core: 182 sections, 98 steps; most steps: [(13, "3.1.3.7"), …]`, `Back-Channel Logout: 24 sections, 11 steps`, `RFC 9449: 48 sections, 12 steps`, `RFC 8252: 32 sections, 5 steps` with `updates: ["rfc6749"]`, `RFC 9700: 75 sections, 26 steps` with `updates: ["rfc6749", "rfc6750", "rfc6819"]`, and `RFC 6749: 116 sections, 0 steps` with `obsoletes: ["rfc5849"]`.

- [ ] **Step 6: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all pass.

```bash
git add crates/codecite-index/src/spec.rs crates/codecite-index/src/spec/render.rs crates/codecite-index/src/spec/relations.rs crates/codecite-index/src/lib.rs
git commit -m "feat(index): detect spec steps, requirement keywords and RFC relations (#24)"
```

### Task 3: Record steps, keywords and relations in design.md

**Files:**
- Modify: `docs/design.md`

- [ ] **Step 1: Edit**

The manifest section said relations are "never fetched", which reading them from an RFC's header now contradicts; it now says the manifest's win.

```diff
diff --git a/docs/design.md b/docs/design.md
index 8a50ea5..942b35d 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -180,7 +180,7 @@ BM25 over code-aware terms does most of the work; vectors and graph signals refi
 
 1. **Fetch:** `git clone --filter=blob:none` at the manifest's pinned commit, read a local working tree, or fetch a document by URL and check it against its pinned `sha256`. Every document is fetched before anything is written, so changed bytes stop the run with an error naming the document and both hashes, and the store is untouched until the pin is updated on purpose. Fetched documents are cached outside the repo, in `<cache dir>/codecite/documents/<sha256>` (`~/.cache` on Linux): a document whose pinned hash is in the cache is never fetched again, so indexing works offline after the first run. A cache file whose bytes no longer match its name is deleted and fetched again. A download is plain HTTP(S) GET (`ureq`, rustls with Mozilla's roots, the standard proxy variables), capped at 16 MiB and 60 seconds; the hash, not the transport or an ETag, is what pins the bytes. A document marked `excluded` is never fetched.
 2. **Walk:** list the files git sees (`git ls-files`, so `.gitignore`, `.git/info/exclude` and the global excludes apply and tracked files are never ignored), apply the manifest's `include`/`exclude` globs, and skip deny-listed secrets, symlinks, files over 1 MiB and binaries (a NUL byte in the first 8000 bytes, git's rule; extensions are never trusted). Every skipped path is reported with its reason; an ignored directory counts once. A path under a symlinked directory is skipped as a symlink. The repository's config is not trusted, since it can name programs for git to run: the walk turns `core.fsmonitor` off, allows git no transport (an empty `GIT_ALLOW_PROTOCOL`, so a partial clone never fetches a missing object from its promisor remote), and never runs `git status`; `dirty` compares HEAD's tree, the index and `git hash-object --no-filters` of the kept files only.
-3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174). Spec HTML comes in three layouts: xml2rfc v3 RFCs (RFC 8650 and later), the older RFC HTML on rfc-editor.org, and the xml2rfc-generated OpenID pages. `scraper` parses it (html5ever repairs malformed HTML as a browser does), and one iterative walk renders it to plain text: a line per paragraph, list item or table row, `<pre>` kept as written, and page headers, footers and ¶ links dropped. A section runs from its heading to the next heading, numbered or not, so its line range holds its own text and not its subsections'. Front matter, the table of contents, the references and unnumbered parts such as the authors' addresses are not sections. html5ever's parse is quadratic in nesting depth, which is fine for the pinned pages the owner chose but slow for a hostile one.
+3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174). Spec HTML comes in three layouts: xml2rfc v3 RFCs (RFC 8650 and later), the older RFC HTML on rfc-editor.org, and the xml2rfc-generated OpenID pages. `scraper` parses it (html5ever repairs malformed HTML as a browser does), and one iterative walk renders it to plain text: a line per paragraph, list item or table row, `<pre>` kept as written, and page headers, footers and ¶ links dropped. A section runs from its heading to the next heading, numbered or not, so its line range holds its own text and not its subsections'. Front matter, the table of contents, the references and unnumbered parts such as the authors' addresses are not sections. A step is an item of a numbered `<ol>` that sits in no other list, or in an older RFC's plain text a paragraph numbered `1.`, `2.`, … at one indent; it keeps the number the document writes, lists nested in it stay part of its text, and a later list in the same section that reuses a number gives no step for it. Requirement keywords are found in core (`requirement_keywords`), so `cite` can scan a claim the same way: whole uppercase words only, with `MUST NOT`, `SHALL NOT`, `SHOULD NOT` and `NOT RECOMMENDED` read as one keyword across any whitespace. An RFC's "Obsoletes", "Updates", "Obsoleted by" and "Updated by" header fields are read from its HTML (the v3 identifiers block, or the older plain-text header) as `rfc<N>` names. html5ever's parse is quadratic in nesting depth, which is fine for the pinned pages the owner chose but slow for a hostile one.
 4. **Terms:** split camelCase, snake_case and kebab-case identifiers into lowercase parts and keep the original, so `validateToken` matches "validate token" and the exact name. Any character that is not a letter or digit separates parts, an acronym ends before the capital that starts the next word (`HTTPServer` → `http server`), and digits stay with the part before them (`sha256`, `rfc9449`).
 5. **Write:** chunks, full-text rows and symbols for one document in a single transaction. A document whose sha256 matches the stored one is not rewritten; only its `dirty` flag is updated. A stored document whose file is gone, or is now skipped, is deleted with everything stored for it, and so is every source the run did not index (one removed from the manifest, or marked `excluded`). A chunk's `terms` and its full-text `terms` column hold the identifier terms of every word in its text; the `symbol` column holds its symbol name's terms and the `path` column its path's. A local source without a `name` is named after its checkout directory, and two sources with one name are an error. The run ends with a summary: documents written, unchanged and removed, chunks written, skipped files by reason, and the elapsed time.
 6. **Providers:** SCIP, CodeGraph and Graphify run after the base pass and add symbols and edges.
@@ -231,7 +231,7 @@ use = "quote"
 current = true                   # several versions of one name may be pinned; one is current
 ```
 
-`kind = "local"` reads a working tree in place (`path = "~/repos/auth/identity-model"`); its results cite the checkout's HEAD and carry `dirty: true` for files with uncommitted changes (modified, staged or untracked). `path` must be the top of a git work tree with at least one commit; anything else is an error. Document relations (`obsoleted_by`, `updated_by`) are written by hand in the manifest and never fetched. The first real corpus, `corpora/identity.toml`, is identity-model plus the OAuth and OpenID specs it cites; see `docs/superpowers/specs/2026-10-17-identity-corpus-design.md`.
+`kind = "local"` reads a working tree in place (`path = "~/repos/auth/identity-model"`); its results cite the checkout's HEAD and carry `dirty: true` for files with uncommitted changes (modified, staged or untracked). `path` must be the top of a git work tree with at least one commit; anything else is an error. Document relations (`obsoleted_by`, `updated_by`) are written by hand in the manifest and never looked up elsewhere; they win over what an RFC's own header says, since a document published before its successor can't name it. The first real corpus, `corpora/identity.toml`, is identity-model plus the OAuth and OpenID specs it cites; see `docs/superpowers/specs/2026-10-17-identity-corpus-design.md`.
 
 Each source needs `use`; `license` is optional until SPDX detection lands. A git source needs `url` and a 40-character lowercase `commit`; a local source needs `path` (`~` expands to the home directory); a document needs `name`, `url` and a 64-character lowercase `sha256`, and exactly one version of each document name has `current = true`. `name` is optional for git and local sources. Unknown keys are errors, and every error names the source and the field.
```

- [ ] **Step 2: Commit**

```bash
git add docs/design.md
git commit -m "docs: describe spec steps, keywords and relations (#24)"
```
