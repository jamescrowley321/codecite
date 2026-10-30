# Spec Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the spec layer into a new `codecite-spec` crate behind a manifest switch, `adapter = "spec"`, so a fetched page is a generic document (#182) by default and the spec layer is an optional adapter (#191).

**Architecture:** The line is drawn between *numbered sections*, which specs and guidance share, and *spec semantics*, which only specs have (the accepted default on #191). Core keeps the schema, the canonical section citation (`SpecCitation`, `SectionNumber`, `ContentHash`), reading and citing it, the spec slot in `search`, and the `Keyword` enum as the typed `strength` column. `codecite-spec` takes the HTML renderer and the RFC and OpenID parsers with their relations and step splitting, BCP 14 detection (`requirement_keywords`, `normative`), the written spec forms (`RFC 6749 §4.1`, aliases, Markdown links to spec sections), superseded-by, `requirements` and `diff`. `codecite-spec` depends on core only, through a narrow `pub` read API that replaces its use of the private `Store::conn`. `codecite-index`, the CLI and MCP depend on `codecite-spec`; the CLI and MCP call its `read`, `cite`, `requirements`, `requirements_on` and `diff`, which accept everything core's do plus the written forms.

**Tech Stack:** Rust workspace (rusqlite, scraper/ego_tree, serde, clap, rmcp), bash (`evals/history/run.sh`).

**Spec:** issue #191 and its decision comment; the #182 decisions comment (decision 3 (a): schema in core, `pub` read API, parsers and spec semantics in `codecite-spec`).

## Global Constraints

- **Moved, not cut.** Nothing in the spec layer is deleted. Every moved test moves with its code and passes unchanged except for its imports and the call form (`store.diff(…)` → `codecite_spec::diff(&store, …)`). A test that needs its assertion changed is a behaviour change: stop and find the cause.
- **No output changes.** `--json`, human and MCP output of `read`, `cite`, `requirements`, `diff`, `search`, `grep` and `context` stay byte for byte the same for the spec corpora. No migration, no schema change.
- **Dependency direction.** `codecite-spec` → `codecite-core`. `codecite-index` → `codecite-spec`, `codecite-core`. `codecite-mcp` and `codecite` → `codecite-spec` too. Core depends on no codecite crate.
- **Every commit builds and its tests pass.** Each task leaves the workspace green, so the order below matters: the API core needs comes before the code that uses it moves.
- Done when: the spec corpora index and cite exactly as before with `adapter = "spec"`, a source without it indexes as a generic document, and the history benchmark doesn't lose on any repository (Recall@1, Recall@10 or MRR of any repository's pooled `all` row for `codecite` lower than `scorecard.md` on `origin/feat/assist-hook` is a loss → park, never open the PR).
- Before every push: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.

## Decisions taken here (DEC-014 in the loop's journal)

- **The HTML renderer moves into `codecite-spec`, `Layout::Generic` included.** `spec/render.rs` renders both spec layouts and generic pages, in one tree walk. The spec parser needs it and `codecite-spec` cannot depend on `codecite-index`, so it moves to the spec crate whole, as `codecite_spec::html`. The generic document splitter in `codecite-index` calls it with `Layout::Generic`. The alternatives were a sixth crate for HTML rendering, or scraper in core; both change more than the issue asks. The PR body flags this so the owner can ask for the sixth crate instead.
- **`Slugs` moves into core.** The GitHub heading slugger is used by the renderer's generic headings (spec crate) and by Markdown chunking and guidance (index crate). An anchor is already a core citation concept (`AnchorCitation`), so `Slugs` joins it in `codecite_core`.
- **`SpecTarget` stays in core.** A section is read by number or by anchor (`read` of `doc@sha256:<12>#<anchor>` on a spec document), which core keeps; only `SpecReference` and `SpecAliases`, the written forms, move.
- **`CiteResult.superseded_by` stays a field in core.** It is part of the output shape; core's `Store::cite` leaves it `None`, and `codecite_spec::cite` fills it. The JSON and MCP schema don't change.
- **Errors stay in core's `Error`.** The CLI and MCP map `Error` to exit codes and tool errors; moved code raises the same variants (`NotASpecCitation`, `NoSectionOnTopic`, `UnknownVersion`, …) so nothing downstream changes.
- **Free functions, not an extension trait.** `codecite_spec::read(&store, …)` and friends. An extension trait method named `read` would be silently shadowed by the inherent `Store::read`, which is exactly the bug this split must not have.
- **`adapter = "spec"` on a page in no spec layout falls back to the generic splitter**, as the spec path does today (`SpecError::UnknownFormat`). Without the adapter, a page is always generic, even in a spec layout.

## Review Focus

1. **A written spec citation through the CLI and MCP.** `codecite read "RFC 9449 §4.3"` and the MCP `read` tool must go through `codecite_spec::read`; core's `Store::read` now refuses it with `not_a_citation`. Task 5 pins both entry points.
2. **Errors for written forms.** A written reference resolves through the moved `read_reference` and core's `read_section`, so its errors (`no_such_section` with candidates, `unknown_version`, `ambiguous_section`) carry the same text as before. The moved read and cite tests pin them.
3. **A spec page without the adapter.** It indexes as a generic document, cited `name@sha256:<12>#L<a>-L<b>`, and `requirements` on it is `not_a_spec_citation`. Task 8 pins it.
4. **Identical spec corpora.** Task 10 indexes `corpora/identity.toml` with the base binary and the new one and compares the chunk rows and a set of `read`/`cite`/`requirements`/`diff` outputs.

---

### Task 1: An empty `codecite-spec` crate in the workspace

**Files:**
- Create: `crates/codecite-spec/Cargo.toml`, `crates/codecite-spec/src/lib.rs`, `crates/codecite-spec/LICENSE` and `NOTICE` (symlinks, as the other crates have them)
- Modify: `Cargo.toml` (`codecite-spec` in `workspace.dependencies`, and the comment counting the crates), `crates/codecite-index/Cargo.toml`, `crates/codecite-mcp/Cargo.toml`, `crates/codecite/Cargo.toml` (depend on it)

- [ ] **Step 1:** Copy `codecite-core`'s `Cargo.toml` layout: `description = "The spec adapter for codecite: RFC and OpenID parsing, BCP 14, requirements and diff"`, workspace version, lints, `codecite-core.workspace = true`. `lib.rs` holds only the crate doc (what moves here, and that it depends on core alone).
  - *Learn:* a workspace dependency is declared once in the root `[workspace.dependencies]` and taken with `foo.workspace = true`; the version there must match `workspace.package.version` exactly for `0.0.x`.
- [ ] **Step 2:** `cargo build --workspace --locked` fails until `Cargo.lock` is updated: run `cargo build --workspace` once, then the gates. `cargo deny check` passes (no new external crates yet).
- [ ] **Step 3:** Commit `build(spec): add an empty codecite-spec crate (#191)`.

### Task 2: Core's `pub` read API for the spec crate

**Files:**
- Modify: `crates/codecite-core/src/store/read.rs` (`SourceRow` → `pub struct DocumentVersion`, `sources`/`current_version`/`source_names`/`read_target`/`only_document_with` helpers), `store/search.rs` (`check_fresh`), `store/cite.rs` (`CiteResult::from_read`, `Finding::from_error`), `store/write.rs` and `read.rs` (`set_aliases`/`aliases` over plain pairs), `crates/codecite-core/src/store.rs` and `lib.rs` (exports)
- Test: unit tests in `read.rs` for each new `pub` method

- [ ] **Step 1: Failing tests** for the API the spec crate needs, each on a store with two versions of one document:
  - `document_versions(name)` lists both, oldest first, each with `pinned()` (`name@sha256:<12>`), `is_current()`, `obsoleted_by()`, `license_class()`, `indexed_at()`.
  - `current_document(name)` is the current one; `document_names()` lists the names.
  - `current_documents_with_section(&SectionNumber, Option<u32>)` returns the current versions holding that section (and step).
  - `read_section(&DocumentVersion, &SpecTarget, Option<u32>, &ReadOptions)` reads `#section-2.4/step-3` as `read` of the canonical citation does.
  - `version_chunks(&DocumentVersion, "section" | "step")` returns `VersionChunk { id, number, anchor, title, parent_id, start_line, end_line, text }` in document order; it serves both `requirements`' and `diff`'s queries (each keeps its own filtering).
  - `aliases()` returns `Vec<(String, String)>`; `set_aliases` takes `impl IntoIterator<Item = (&str, &str)>`.
- [ ] **Step 2:** Rename `SourceRow` to `DocumentVersion`, fields private, read through methods; `pinned` and `instruction` become methods/`pub fn`s. Make `check_fresh` `pub`. Move `SpecTarget` from `citation/written.rs` to `citation/spec.rs` (still re-exported from the crate root). Add `CiteResult::from_read(ReadResult)` and make `Finding::from_error` `pub`, and have `Store::cite` use them. Core's own code switches to the new names; no behaviour changes, every existing test passes.
  - *Learn:* private fields with `pub fn` getters let core change the struct without breaking the spec crate; `pub(super)` → `pub` is the visibility step from "this module tree" to "any crate".
- [ ] **Step 3:** Gates; commit `refactor(core): a pub read API over document versions and sections (#191)`.

### Task 3: `Slugs` into core

**Files:**
- Modify: `crates/codecite-index/src/chunk/text.rs` (remove `Slugs`), `chunk.rs`, `guidance.rs`, `spec/render.rs` (import from core)
- Create: `crates/codecite-core/src/slugs.rs`; export `Slugs` from `lib.rs`

- [ ] **Step 1:** Move `Slugs` and its tests unchanged; `pub struct Slugs` with `pub fn slug`. Index imports `codecite_core::Slugs`.
- [ ] **Step 2:** Gates; commit `refactor(core): move the heading slugger into core (#191)`.

### Task 4: `requirements` and `diff` into `codecite-spec`

**Files:**
- Move: `crates/codecite-core/src/store/requirements.rs` → `crates/codecite-spec/src/requirements.rs`; `store/diff.rs` → `crates/codecite-spec/src/diff.rs`, with their tests and their result types (`Requirements`, `RequirementSection`, `Requirement`, `RequirementsOptions`, `DiffOptions`, `DiffResult`, `VersionChange`)
- Modify: core `store.rs`, `lib.rs` (drop the exports), `crates/codecite/src/main.rs`, `crates/codecite-mcp/src/server.rs` (call `codecite_spec::requirements`, `requirements_on`, `diff`), `crates/codecite-index/src/index.rs` (the `backchannel_diff` test), `crates/codecite/tests/diff.rs`, `requirements.rs`

It reads through core's `store.read`, which still takes the written forms until Task 5 moves them and switches it to `crate::read`; it keeps importing `requirement_keywords` and `normative_statements` from core until Task 6 moves them.

- [ ] **Step 1:** Rewrite the `impl Store` blocks as free functions over `&Store`, replacing `self.conn` queries with `store.version_chunks`, `self.sources` with `document_versions`, `self.read` with `store.read`. The test helper that builds a store with sections and steps uses core's write API, which is already `pub`.
  - *Learn:* moving `impl Store { fn x(&self) }` out of the defining crate is not possible (inherent impls live with the type), which is why the functions become `fn x(store: &Store)`.
- [ ] **Step 2:** The MCP tool schemas come from the result types through `schemars`; moving the types must not change the generated schema. Compare `codecite mcp`'s `tools/list` output before and after (save both to `/var/tmp/spec-crate/`).
- [ ] **Step 3:** Gates; commit `refactor(spec): move requirements and diff into codecite-spec (#191)`.

### Task 5: The written spec forms, `read` and `cite` in `codecite-spec`

**Files:**
- Move: `crates/codecite-core/src/citation/written.rs` (`SpecReference`, `SpecAliases`, with tests) → `crates/codecite-spec/src/written.rs`; `read_reference`, `only_document_with` (now over `current_documents_with_section`), `cite_link`, `markdown_link`, `section_of`, `superseded_by` and their tests out of core `store/read.rs` and `store/cite.rs` → `crates/codecite-spec/src/read.rs` and `cite.rs`
- Modify: core `read.rs` (`Store::read` no longer tries written forms: after canonical and anchor, `NotACitation` or the canonical parse error), core `cite.rs` (no Markdown-link branch; `superseded_by` left `None`), core `write.rs` (`set_aliases` over pairs), `codecite-index/src/index.rs` (`document_aliases` builds `SpecAliases` from `codecite_spec`), `crates/codecite/src/main.rs` and `crates/codecite-mcp/src/server.rs` (call `codecite_spec::read`/`cite`), `crates/codecite/tests/cite.rs`, `diff.rs` (imports)

- [ ] **Step 1: Failing tests first.** In `crates/codecite/tests/` and the MCP server tests: `read "RFC 9449 §4.3"` and `cite "[OIDC Core §3.1.3.6](https://openid.net/specs/…#IDTokenValidation)"` still resolve through the CLI and the MCP tool (they pass now and must keep passing after the move); a core unit test that `Store::read("RFC 9449 §4.3")` is `NotACitation` (fails now).
- [ ] **Step 2:** `codecite_spec::read(store, text, options)`: core's `Store::read` first; on `NotACitation` or `BadCitation` from it, `SpecReference::parse(text, &SpecAliases::from(store.aliases()?))`, and map the outcome exactly as core's `read` did (`Ok(None)` → the error core returned; `UnknownDocument` → `UnknownSource` with `document_names()`). A canonical citation is still tried first, so a code path holding `§` keeps reading as code.
- [ ] **Step 3:** `codecite_spec::cite(store, text, options)`: a Markdown link → the moved `cite_link`; anything else → `codecite_spec::read`, then `CiteResult::from_read` / `Finding::from_error`, then `superseded_by` filled from `DocumentVersion`.
  - *Learn:* a free function taking `&Store` adds behaviour to a type from another crate without a trait; the orphan rule would allow an extension trait here, but its `read` would be shadowed by the inherent method.
- [ ] **Step 4:** Point the CLI's `read`/`cite` commands and the MCP `read`/`cite` tools at the spec functions, and switch `codecite_spec::requirements` (Task 4) from `store.read` to `crate::read`, so it keeps taking written forms. Move the tests; run them unchanged.
- [ ] **Step 5:** Gates; commit `refactor(spec): move the written spec forms, superseded-by and spec read and cite into codecite-spec (#191)`.

### Task 6: BCP 14 detection and normative statements into `codecite-spec`

**Files:**
- Move: `requirement_keywords` (and its tests) from `crates/codecite-core/src/keyword.rs` to `crates/codecite-spec/src/keyword.rs`; `crates/codecite-core/src/normative.rs` → `crates/codecite-spec/src/normative.rs`
- Modify: core `lib.rs` (drop their exports; `Keyword` and `UnknownKeyword` stay), `codecite-index/src/guidance.rs`, `codecite-index/src/spec.rs` (import from `codecite_spec`)

- [ ] **Step 1:** After Tasks 4 and 5, core has no user of `requirement_keywords` or `normative` left. Move both modules with their tests; `normative::collapse` becomes `pub(crate)` in the spec crate.
- [ ] **Step 2:** Gates; commit `refactor(spec): move BCP 14 detection and normative statements into codecite-spec (#191)`.

### Task 7: The spec parsers and the HTML renderer into `codecite-spec`

**Files:**
- Move: `crates/codecite-index/src/spec.rs` → `crates/codecite-spec/src/parse.rs` (`split_spec`, `SpecDocument`, `SpecSection`, `SpecStep`, `SpecError`, `SpecFormat`, `SpecRelations`, `line_number`), `spec/relations.rs` → `crates/codecite-spec/src/parse/relations.rs`, `spec/render.rs` → `crates/codecite-spec/src/html.rs` (`pub fn render`, `Rendered`, `Layout`, `detect`); their fixtures under `crates/codecite-index/tests/fixtures/` that only these tests read move to `crates/codecite-spec/tests/fixtures/`
- Modify: `crates/codecite-index/src/lib.rs` (re-export nothing spec), `document.rs` (`codecite_spec::html`), `index.rs`, `crates/codecite-index/Cargo.toml` and `crates/codecite-spec/Cargo.toml` (`scraper`, `ego-tree` move to spec if index no longer uses them directly)

- [ ] **Step 1:** `SpecRelations::with_manifest(&DocumentSource)` takes the manifest type, which stays in index: change it to take the two fields (`obsoleted_by`, `updated_by`) so the spec crate needs no manifest.
- [ ] **Step 2:** Move the modules and tests unchanged; `pub(crate)` items the index still uses become `pub` (and documented, `missing_docs` is on).
- [ ] **Step 3:** Gates; `cargo deny check` (the dependency graph changed); commit `refactor(spec): move the RFC and OpenID parsers and the HTML renderer into codecite-spec (#191)`.

### Task 8: `adapter = "spec"` in the manifest

**Files:**
- Modify: `crates/codecite-index/src/manifest.rs` (`DocumentSource.adapter: Option<Adapter>`, `enum Adapter { Spec }`, `#[serde(rename_all = "lowercase")]`), `crates/codecite-index/src/index.rs` (`index_document`: `split_spec` only when `adapter == Some(Adapter::Spec)`), `corpora/identity.toml` and `evals/retrieval/identity-bench.toml` (`adapter = "spec"` on every spec document source), every test manifest that indexes a spec page (`crates/codecite/tests/documents.rs`, `index.rs`, `requirements.rs`, and the `index.rs` unit tests)
- Test: `manifest.rs`, `index.rs`, `crates/codecite/tests/documents.rs`

- [ ] **Step 1: Failing tests.** Manifest: `adapter = "spec"` parses; `adapter = "openapi"` is a manifest error naming the key; no `adapter` is `None`. Index: the RFC v3 fixture with `adapter = "spec"` stores sections cited `#section-N`; the same fixture without it is a generic document with `document.text` set and line citations; `requirements` on it is `not_a_spec_citation`; with the adapter and a page in no spec layout, the page is generic as before.
  - *Learn:* `Option<Adapter>` with `#[serde(default)]` makes the key optional; a one-variant enum still rejects any other string, so a typo fails loudly instead of indexing generically.
- [ ] **Step 2:** Implement; add the key to every spec source in the two corpus manifests and the test manifests that rely on spec chunks. A content hash that matches but was stored by the other splitter must re-split: check the "unchanged" shortcut in `index_document` (compare stored `document.text` NULL vs set, as `stored_as_guidance` does), with a test that flips the adapter between two runs.
- [ ] **Step 3:** Gates; commit `feat(index): index a document through the spec adapter only with adapter = "spec" (#191)`.

### Task 9: Docs

**Files:**
- Modify: `docs/design.md` (six crates, the dependency diagram and "never on each other" sentence, `codecite-spec`'s role, `Store::requirements` → `codecite_spec::requirements` and friends, the publish order), `docs/usage.md` (the `adapter` key of a document source), `README.md` if it lists crates, crate docs in `codecite-core/src/lib.rs` and `codecite-index/src/lib.rs`

- [ ] **Step 1:** Edit in place; no status words. Commit `docs: describe the codecite-spec crate and adapter = "spec" (#191)`.

### Task 10: The spec corpora index and cite exactly as before

- [ ] **Step 1:** Build the base (`git worktree add /var/tmp/spec-crate/base origin/feat/assist-hook`, `cargo build --release --locked -p codecite`) and this branch. Index `corpora/identity.toml` with each into separate data dirs (`CODECITE_DATA_DIR` or `--data-dir`).
- [ ] **Step 2:** Compare: `sqlite3 … "SELECT d.path, c.kind, c.number, c.anchor, c.title, c.start_line, c.end_line, c.strength, c.text FROM chunk c JOIN document d ON d.id = c.document_id ORDER BY 1,2,3,6"` dumps are identical; and the `--json` output of `read`, `cite`, `requirements` and `diff` for ten fixed inputs (written forms, an alias, a Markdown link, a superseded version, a guidance rule, a topic) is identical. Keep the script and its outputs in `experiments/spec-crate/`.
- [ ] **Step 3:** Commit `test(spec): the spec corpora index and cite as before the move (#191)`.

### Task 11: History benchmark

- [ ] **Step 1:** `cargo build --release --locked -p codecite -p codecite-bench`, then `evals/history/run.sh` (it exports `RG`). Compare each repository's pooled `all` row for `codecite` with `scorecard.md` on `origin/feat/assist-hook`. A loss on Recall@1, Recall@10 or MRR → park.
- [ ] **Step 2:** Commit `evals/history/results/` as `test(evals): history benchmark on the spec crate (#191)`.
