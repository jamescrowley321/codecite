# Generic Documents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Any Markdown, HTML or text document, in a repository or fetched and pinned, is split on its own headings and cited by line span (#182). A fetched document gets `name@sha256:<12 hex>#L<a>-L<b>`; `read` and `cite` also accept `…#<anchor>`; results carry `anchor` and `title`.

**Architecture:** Lines stay the one canonical span. Headings give chunks a `title` and a GitHub-style `anchor` (the `chunk.anchor` and `chunk.title` columns already exist, so no schema change, only a migration that makes Markdown re-split). A fetched page whose layout `render::detect` knows (RFC v3, RFC text, OpenID) still goes down the spec path unchanged, since moving that path behind `adapter = "spec"` is #191. Any other fetched page, which today stops the run with `UnknownFormat`, becomes a generic document: HTML is rendered to text lines and split at every `<h1>`–`<h6>`, Markdown at its ATX headings, and plain text into 60-line windows. Its rendered text is stored in `document.text`, so `read`, `grep` and `search` cite its lines the way they cite a file's. The rule everywhere: a document with stored text is cited by lines, a spec document (no stored text, numbered sections) by section.

**Tech Stack:** Rust (scraper/ego_tree, rusqlite, serde, clap), SQLite migrations, bash (`evals/history/run.sh`).

**Spec:** `docs/superpowers/specs/2026-10-29-general-purpose-design.md` (goal 3), issue #182 and its decisions comment; #191 is the follow-up.

## Global Constraints

- Decisions on #182, accepted by the owner: lines are the canonical span everywhere; a fetched document cites as `name@sha256:<12 hex>#L<a>-L<b>`; `read` and `cite` accept `…#<anchor>`; results carry `anchor` and `title`; older RFCs (no HTML headings) get line windows by default.
- The spec corpora keep indexing through the existing spec path until #191. No spec citation, spec chunk or `requirements`/`diff` output changes.
- Done when: a README, an ADR and an RFC index and cite the same way by default, and the history benchmark doesn't lose on any repository (Recall@1, Recall@10 or MRR of any repository's pooled `all` row for `codecite` lower than `scorecard.md` on `origin/feat/replay-issue-tasks` is a loss).
- `--json` output and MCP output stay the same shape; new fields are optional and skipped when absent.
- Indexed text stays escaped through `escape_controls` in every human form.
- Before every push: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.

## Decisions taken here (DEC-006 in the loop's journal)

- **"An RFC cites the same way by default" before #191.** The generic HTML splitter handles an RFC page (proved on the RFC fixtures in Task 3), but a page in a known spec layout still takes the spec path in this PR, because #182 keeps the spec corpora on that path and #191 adds the `adapter = "spec"` switch that makes generic the default. The PR body says so.
- **Fetched HTML lines are rendered-text lines.** A fetched page has no file to point into, and raw HTML lines are markup (or one minified line). The spec path already cites rendered lines; the generic path stores the rendered text in `document.text` so `read` returns exactly the cited lines.
- **Repository HTML lines are file lines.** A repository file's citation `repo@sha:path#L…` must point into the file. `.html`/`.htm` files are split at lines that open an `<h1>`–`<h6>` tag, over the raw lines.
- **Fetched format:** spec layout first; then HTML when the first non-blank bytes are `<`; then Markdown when the URL path ends in `.md` or `.markdown`; else plain text.

## Review Focus

1. **A heading slug that looks like a line span or a spec section.** Slugs are lowercase, so `#l10` is an anchor and `#L10` lines. Under `sha256:`, `#section-4.3` stays a spec section; `#section-four` is an anchor. Task 4 pins all four.
2. **Two headings with the same title.** The second gets `-1`, the GitHub rule guidance already uses. Task 1 pins it.
3. **A generic fetched page with no headings at all** (an older RFC as plain text). It is all windows, each cited by lines, `read` by anchor fails with `no_such_anchor`. Task 3 pins it.
4. **Existing indexes.** Markdown documents indexed before this change have NULL anchors; migration 0007 clears their content hashes so the next `index` re-splits them. Task 1 pins it with a store opened at version 6.
5. **A spec document with an anchor.** `name@sha256:<hash>#<anchor>` on a spec document resolves through the existing anchor lookup and returns the spec section citation, as a URL fragment does today. Task 6 pins it.

---

### Task 1: Markdown sections carry anchors

**Files:**
- Modify: `crates/codecite-index/src/chunk.rs` (`Chunk` gains `anchor: Option<String>`), `crates/codecite-index/src/chunk/text.rs` (`sections` fills it), `crates/codecite-index/src/guidance.rs` (move `Slugs` out), `crates/codecite-index/src/index.rs:397-412` (write `anchor: c.anchor.as_deref()`)
- Create: `crates/codecite-core/migrations/0007_heading_anchors.sql`; add it to `MIGRATIONS` in `crates/codecite-core/src/store.rs:51-58`
- Test: unit tests in `chunk/text.rs`, `chunk.rs`, `index.rs`; migration test in `store.rs`

- [ ] **Step 1: Failing tests.** In `chunk/text.rs`: `markdown_sections_carry_github_anchors` — `# Getting Started`, `## Install (Linux)`, `## Install (Linux)` give anchors `getting-started`, `install-linux`, `install-linux-1`. In `index.rs`: a git source with `README.md` stores `chunk.anchor` and `chunk.title` for each section. In `store.rs`: open a store, set `user_version` to 6 with a Markdown and a Rust document, reopen read-write: the Markdown document's `content_hash` is cleared, the Rust one's is kept.
- [ ] **Step 2:** Move `Slugs` from `guidance.rs` into `chunk/text.rs` as `pub(crate) struct Slugs`, unchanged; `guidance.rs` imports it. `sections` keeps one `Slugs` per file. Every other `Chunk { … }` literal sets `anchor: None`.
  - *Learn:* moving a type and re-importing it with `use crate::chunk::text::Slugs;` keeps one definition; `pub(crate)` makes it visible across the crate and nowhere else.
- [ ] **Step 3:** `0007_heading_anchors.sql`: `UPDATE document SET content_hash = x'' WHERE language = 'markdown' AND text IS NOT NULL;` (guidance documents keep `text` NULL, and are re-split every run anyway). Check `open_read_write`'s per-version hooks still run in order.
- [ ] **Step 4:** Run the tests; commit `feat(index): give Markdown sections GitHub heading anchors (#182)`.

### Task 2: Repository HTML split at its headings

**Files:**
- Modify: `crates/codecite-core/src/language.rs` (`Language::Html` for `.html`, `.htm`; name `html`; no tree-sitter grammar), `crates/codecite-index/src/chunk.rs` (`Chunker::chunk` routes Html to `text::html_sections`), `crates/codecite-index/src/chunk/text.rs`
- Test: `chunk/text.rs`, `chunk.rs`, `language.rs`

- [ ] **Step 1: Failing tests.** `html_sections_start_at_heading_tags`: a page with `<h1 id="intro">Intro</h1>` on line 3 and `<h2>Set <em>up</em></h2>` on line 9 gives sections L3–L8 (title `Intro`, anchor `intro` from `id`) and L9–end (title `Set up`, anchor `set-up`); lines before L3 fall into a window. A heading split across lines (`<h2>\nSet up\n</h2>`) takes its title from the lines up to `</h2>`. A heading inside `<pre>` or `<!-- -->` on its own lines is not a heading. Every exhaustive `match` on `Language` gets an arm; `Role::of` still counts `.html` as docs or code as it does today (check `role.rs` and leave its result as it is).
- [ ] **Step 2:** `html_sections(lines)`: scan raw lines for `<h[1-6][\s>]` (case-insensitive) outside `<pre>`/comments; title = the heading element's text with tags stripped and whitespace collapsed (parse the heading's lines with `scraper::Html::parse_fragment`); anchor = the tag's `id` when present, else `Slugs::slug(title)`. Section end mirrors `section_end` for Markdown (to the line before the next heading, trailing blank lines trimmed).
  - *Learn:* adding an enum variant makes the compiler list every `match` that must handle it: the exhaustiveness check is the to-do list.
- [ ] **Step 3:** Run tests; commit `feat(index): split repository HTML at its headings (#182)`.

### Task 3: A generic document splitter for fetched pages

**Files:**
- Modify: `crates/codecite-index/src/spec/render.rs` (heading detection becomes a parameter), `crates/codecite-index/src/spec.rs`
- Create: `crates/codecite-index/src/document.rs` (`mod document;` in `lib.rs`)
- Test: `document.rs` with fixtures `crates/codecite-index/tests/fixtures/spec/rfc-v3.html`, `rfc6749.html`, and a new `crates/codecite-index/tests/fixtures/document/plain.html`

**Interfaces:**
- Produces, used by Task 5:
  ```rust
  pub(crate) enum Format { Html, Markdown, Text }
  pub(crate) struct GenericDocument {
      /// The text lines are counted in, joined with `\n`.
      pub(crate) text: String,
      pub(crate) format: Format,
  }
  /// Spec layouts are not checked here; the caller tries `split_spec` first.
  pub(crate) fn split_document(bytes: &str, url: &str) -> GenericDocument;
  impl GenericDocument { pub(crate) fn chunks(&self) -> Vec<Chunk<'_>>; }
  ```

- [ ] **Step 1: Failing tests.** `plain.html` (a `<title>`, `<nav>`, `<h1>`, two `<h2>` with `id`s, a `<script>`) renders without nav or script text and gives three sections with titles and anchors from `id`, cited lines being lines of `text`. The RFC v3 fixture through the generic splitter gives sections whose titles include the section heading text (for example `4.3. Checking DPoP Proofs`) and anchors from the `<section>`/heading ids. The `rfc6749.html` fixture (old one-`<pre>` layout) gives windows only, no sections (decision 2). A Markdown body at a `.md` URL splits like a repository Markdown file; a text body is windows. Format detection: leading whitespace then `<` is HTML regardless of URL.
- [ ] **Step 2:** In `render.rs`, replace the `format: SpecFormat` argument's heading role with a `Layout` the caller passes: `Layout::Spec(SpecFormat)` keeps today's behaviour exactly (spec tests unchanged), `Layout::Generic` treats every `h1`–`h6` as a heading with `Numbered { number: String::new(), title, anchor }` where anchor is the heading's or its parent `<section>`'s `id`, else a slug. Page-footer and plain-heading passes stay spec-only. Make what `document.rs` needs `pub(crate)`.
  - *Learn:* an enum argument instead of a boolean keeps call sites readable (`render(&html, Layout::Generic)`), and `match` on it keeps each layout's rule in one place.
- [ ] **Step 3:** `document.rs`: detect format, render or take the text as is, then build chunks with `chunk::text` (`sections`/`windows` over `Lines`) from the heading lines, with uncovered lines windowed exactly like `Chunker::chunk` does.
- [ ] **Step 4:** Run tests; commit `feat(index): split a fetched page that is not a spec at its own headings (#182)`.

### Task 4: The fetched-document citation and anchor references

**Files:**
- Modify: `crates/codecite-core/src/citation.rs`, `crates/codecite-core/src/citation/spec.rs`, `crates/codecite-core/src/lib.rs` (exports)
- Create: `crates/codecite-core/src/citation/document.rs`, `crates/codecite-core/src/citation/anchor.rs`

**Interfaces:**
- Produces, used by Tasks 6–8:
  ```rust
  pub struct DocumentCitation { pub doc: String, pub hash: ContentHash, pub lines: LineRange }
  // Display: "{doc}@sha256:{hash}#{lines}"
  pub enum Citation { Code(CodeCitation), Spec(SpecCitation), Document(DocumentCitation) }
  pub enum Pin { Commit(CommitSha), Hash(ContentHash) }
  pub struct AnchorCitation { pub source: String, pub pin: Pin, pub path: Option<String>, pub anchor: String }
  // "repo@sha:path#anchor" or "doc@sha256:<12 hex>#anchor"
  ```

- [ ] **Step 1: Failing tests.** `rfc9449@sha256:1a2b3c4d5e6f#L120-L140` parses as `Citation::Document` and round-trips; `#section-4.3` still parses as `Spec`; `#L10-L10` is `SingleLineRange`. `AnchorCitation::from_str`: `repo@abc1234:README.md#getting-started`, `notes@sha256:1a2b3c4d5e6f#install-linux-1`, `#l10` (anchor), `#section-four` (anchor). It refuses `#L10` (that's lines), `#section-4` under `sha256:` (that's a spec section), an empty anchor, and an anchor with characters a slug never has (`BadAnchor`).
- [ ] **Step 2:** Under `sha256:`, `Citation::from_str` routes on the fragment: `L<digit>` → document lines, else spec. Reuse `LineRange` from `code.rs` (make it `pub(crate)`-reachable if not already).
- [ ] **Step 3:** Run tests; commit `feat(core): cite a fetched document by lines and anything by heading anchor (#182)`.

### Task 5: Index a fetched page that is not a spec as a document

**Files:**
- Modify: `crates/codecite-index/src/index.rs:434-493` (`index_document`), `crates/codecite-core/src/store/write.rs` (`spec_fts` rows only for chunks with a `number`; check every reader of `spec_fts`)
- Test: `index.rs` tests (replace `a_page_that_is_not_a_spec_is_an_error`)

- [ ] **Step 1: Failing tests.** `a_page_that_is_not_a_spec_is_indexed_as_a_document`: the test server serves `plain.html`; indexing succeeds, the document has `text` set and `language = "html"`, chunks are `section`/`window` with `number` NULL and anchors set. A Markdown page at `/notes.md` gets `language = "markdown"`. Reindexing the same pin is a no-op (`unchanged` 1). A spec page is unchanged: `a_document_is_indexed_as_sections_and_steps` still passes as is.
- [ ] **Step 2:** `index_document`: try `split_spec` only when `render::detect` finds a layout; otherwise `document::split_document`, write `language: Some(format name)`, `text: Some(&generic.text)`, chunks from `generic.chunks()` with `anchor`/`title`. The unchanged-check becomes "same hash and not stored as guidance" (guidance stores `language = markdown` with `text` NULL; read `store.document_language` and the text's presence, add a small store query if needed).
- [ ] **Step 3:** Run tests; commit `feat(index): index a fetched page that is not a spec as a document (#182)`.

### Task 6: `read` by lines of a document and by anchor

**Files:**
- Modify: `crates/codecite-core/src/store/read.rs`
- Test: `read.rs` unit tests; `crates/codecite/tests/read.rs`

- [ ] **Step 1: Failing tests.** `ReadResult` gains `anchor: Option<String>` (skipped when `None`). Reading `notes@sha256:<hash>#L3-L8` returns those lines of the stored text, `kind: "document"`. Reading `repo@sha:README.md#install` returns the canonical `repo@<full sha>:README.md#L<a>-L<b>`, `kind: "section"`, its `title` and `anchor`. A lines read whose span equals a section's also carries its title and anchor. `#nope` errors `no_such_anchor` naming the document. A spec document read by `name@sha256:<hash>#<anchor>` returns the spec section citation as a URL fragment does today. `kind` stays `code` for a file's lines outside a section.
- [ ] **Step 2:** `Store::read` matches `Citation::Document` → `read_lines` on the document source (reuse `read_code`'s line slicing over `document.text`; `sources(name, true)`), then `AnchorCitation` → look up `chunk.anchor` in the one document (`path`, or the document source's single document), then fall back to `SpecReference` as now. Add `Error::NoSuchAnchor` with kind `no_such_anchor` in `error.rs`.
  - *Learn:* trying parsers in order with `if let Ok(c) = s.parse::<Citation>() { … } else if let Ok(a) = s.parse::<AnchorCitation>() { … }` keeps each form's grammar in its own type.
- [ ] **Step 3:** Run tests; commit `feat(core): read a document by lines or by heading anchor (#182)`.

### Task 7: `cite` resolves any citation

**Files:**
- Modify: `crates/codecite-core/src/store/cite.rs` (`CiteResult` gains `anchor`; code, document and anchor citations resolve through `read`; `findings` stay spec-only), `crates/codecite-core/src/error.rs` (keep `NotASpecCitation` for `requirements`)
- Test: `cite.rs` unit tests (the `not_a_spec_citation` row at :816 becomes a resolved code citation), `crates/codecite/tests/cite.rs`

- [ ] **Step 1: Failing tests.** `cite` on `app@0123456:src/lib.rs#L1` resolves with `resolved: true` and the full-sha citation; on `repo@sha:README.md#install` resolves to the section's lines with `title` and `anchor`; on a document citation resolves; spec behaviour and findings are unchanged.
- [ ] **Step 2:** Implement; run tests; commit `feat(core): cite repository and fetched documents, by lines or anchor (#182)`.

### Task 8: Search and grep cite generic documents by lines; results carry title and anchor

**Files:**
- Modify: `crates/codecite-core/src/store/search.rs` (`SearchResult` gains `title`, `anchor`; `ranked()` selects `c.title`, `c.anchor`, `d.text IS NOT NULL`; a document-source chunk with no `number` cites as `DocumentCitation`), `crates/codecite-core/src/store/grep.rs` (a document source whose document has text greps like code, cited by document lines), `crates/codecite-core/src/store/context.rs` (check every `kind = 'document'` branch), `crates/codecite/src/main.rs` (`human`: a section hit shows its title where a code hit shows its symbol), `crates/codecite-mcp/src/server.rs` (tool descriptions list the new forms)
- Test: `search.rs`, `grep.rs` unit tests; `crates/codecite/tests/search.rs`; `main.rs` unit tests

- [ ] **Step 1: Failing tests.** A repository Markdown hit carries `title` and `anchor` in `--json`; its human line reads `<citation> · section <title> · <snippet>`. A generic fetched document's hit cites `name@sha256:<12 hex>#L<a>-L<b>`; a spec hit is unchanged. Grep on a generic fetched document returns line citations; grep on a spec is unchanged.
- [ ] **Step 2:** Implement; run all gates; commit `feat(search): cite generic documents by lines and show heading titles (#182)`.

### Task 9: README, ADR and RFC end to end

**Files:**
- Create: `crates/codecite/tests/documents.rs` (Linux-only like its neighbours)

- [ ] **Step 1:** A git repo with `README.md` and `docs/adr/0001-use-sqlite.md`, and a fetched generic HTML page (the RFC v3 fixture rendered generically is not reachable here, so use `plain.html` served from a local HTTP server or a `file`-backed fixture the index tests already use; check how `crates/codecite/tests/index.rs` serves documents). For each: `search --json` finds a section with `title` and `anchor`; `read` by its lines and by `#anchor` return the same text and canonical citation; `cite` resolves both forms. This is the "index and cite the same way" acceptance test.
- [ ] **Step 2:** Run; commit `test(cli): a README, an ADR and a fetched page cite the same way (#182)`.

### Task 10: Docs

**Files:**
- Modify: `docs/design.md` (:13 summary, :103-104 diagram, :174-176 chunk schema comment, :338-341 chunking and "a page in no known layout" — now indexed as a document, :489-499 citation forms, `read` fields, `cite`), `docs/usage.md` (:6-25 concepts, :50-59 document sources, :198-237 search output with `title`/`anchor`, :289-315 `read` forms table, :323-375 `cite`)

- [ ] **Step 1:** Describe: heading split for Markdown, HTML and text, anchors (GitHub slugs, `-1` for repeats), the document citation form, `#<anchor>` input to `read`/`cite`, the rule "spec layouts keep the spec path until #191", rendered-text lines for fetched HTML. Remove "a page in no known layout stops the run".
- [ ] **Step 2:** Commit `docs: describe generic documents and their citations (#182)`.

### Task 11: History benchmark

- [ ] **Step 1:** `cargo build --release --locked -p codecite -p codecite-bench`, then `evals/history/run.sh`.
- [ ] **Step 2:** Compare each repository's pooled `all` rows for `codecite` with `git show origin/feat/replay-issue-tasks:evals/history/results/scorecard.md`. Lower Recall@1, Recall@10 or MRR anywhere is a loss: park the issue with both rows, no PR.
- [ ] **Step 3:** Commit `evals/history/results/` as `chore(evals): rerun the history benchmark on generic documents (#182)`.
