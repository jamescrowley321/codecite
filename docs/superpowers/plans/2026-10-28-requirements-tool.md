# Requirements Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `requirements` MCP tool and `codecite requirements` command (#157) that, given a spec section citation (or a topic and a document), return every normative statement governing it, cited and grouped by section, MUSTs first.

**Architecture:** Read-only over what the index already stores: a section's text, its steps, and its subsections (`parent_id`). A pure function in codecite-core, `normative_statements`, splits a section's text into sentences and claim/parameter-table rows and keeps the normative ones. `Store::requirements` resolves the citation as `read` does, gathers the governing sections from the one document version, and runs `normative_statements` over each. The CLI and MCP server print the same JSON.

**Tech Stack:** Rust, rusqlite, serde/schemars, rmcp, clap.

**Spec:** issue #157; `docs/design.md` (spec chunking, `requirement_keywords`).

## Global Constraints

- No new indexing and no schema change: everything comes from `chunk` rows already stored.
- Keywords that make a statement normative: MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD NOT, and RECOMMENDED / NOT RECOMMENDED (BCP 14 defines them as SHOULD and SHOULD NOT). MAY and OPTIONAL do not.
- A table row is normative when its leading word is REQUIRED or PROHIBITED, or its text holds a normative keyword.
- Every statement carries a pinned citation (`doc@sha256:<12 hex>#section-N[/step-K]`). Only the same document version is spanned.
- A `quote` source's result carries `QUOTE_INSTRUCTION`.
- The extraction function is public and reusable (#158 needs a section's normative statements): `codecite_core::normative_statements`.

## Design decisions

**Statements.** A section's text, with the lines its steps cover blanked, is split into paragraphs. In the older RFC layout text is wrapped, so consecutive indented lines join into one paragraph; in the OpenID and xml2rfc v3 layouts every paragraph is already one line, unindented. A definition-list row is a line whose first word is a row keyword followed by `.` (`REQUIRED.`, `OPTIONAL.`, `RECOMMENDED.`, `NOT RECOMMENDED.`, `PROHIBITED.`) right after a one-word term line; it continues over the following lines indented as deep as it and deeper than the term. A row becomes one statement, `term: text`. Other paragraphs split into sentences at `.`, `?` or `!` followed by whitespace and an uppercase letter, digit, quote or bracket, except after a single capital (an initial) or `e.g`, `i.e`, `etc`, `cf`, `Ed`, `vs`. Whitespace inside a statement collapses to one space.

**Governing sections.** For the asked section A, in one document version:

1. `asked`: A itself, every step of it included whatever its keywords (a procedure's steps are its rules).
2. `subsection`: A's descendants by `parent_id`, all their steps included.
3. `referenced`: sections that A or its descendants name: `Section N` / `Sections N, M and K` in their text, except a reference into another document (`Section 2 of [OpenID.Core]`, `[RFC3986] Section 3.4`); and sections whose title, of two or more words, is a whole-word phrase of A's title (Back-Channel Logout §2.6 "Logout Token Validation" names §2.4 "Logout Token" this way, and in no other: its text has no `Section 2.4`).
4. `referring`: sections whose text names A or a descendant; and `context`: the parent of each referring subsection, since a subsection's rules apply in its parent's scope (RFC 6749 §3.2.1 "Client Authentication" names §2.3; its parent §3.2 "Token Endpoint" holds "MUST NOT be included more than once", which no text links to §2.3 directly).

Outside A and its descendants, only steps with a normative keyword are kept. A section with no number that parses as `SectionNumber` (an appendix) is skipped. Groups come in that order (asked, subsections, then the rest in document order), each section once, under its first relation. Within a group: MUST-level statements, then steps in document order, then SHOULD-level statements; document order within each tier. The output is capped at `MAX_READ_BYTES` of statement text; past it statements are dropped and `truncated` is set.

**Topic.** With a topic and a document (name or alias) instead of a citation, the best spec `search` hit in that document's current version picks A; no hit is `Error::NoSectionOnTopic`.

## Review Focus

- A citation to a step: the requirements are its section's (test in Task 2).
- A code citation: `not_a_spec_citation`, with a message that doesn't say only `cite` (Task 2).
- A section with no normative text and no steps: an empty `statements` list for it, never an error (Task 2).
- Initials and `e.g.` in a reference string must not split a sentence (`and C. Mortimore`) (Task 1).
- A huge section (RFC 6749 §4) stays bounded: `truncated` true (Task 2).

---

### Task 1: `normative_statements` in codecite-core

**Files:**
- Create: `crates/codecite-core/src/normative.rs`
- Modify: `crates/codecite-core/src/lib.rs` (export)

**Interfaces:**
- Produces:
  - `pub fn normative_statements(text: &str) -> Vec<NormativeStatement>`
  - `pub struct NormativeStatement { pub kind: StatementKind, pub line: usize /* 0-based first line in text */, pub keywords: Vec<Keyword>, pub prohibited: bool, pub text: String }`
  - `pub enum StatementKind { Sentence, Row }`
  - `impl NormativeStatement { pub fn level(&self) -> Level }`, `pub enum Level { Must, Should }` (`Must` for MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT or a PROHIBITED row)
  - `pub fn normative_level(keywords: &[Keyword]) -> Option<Level>`

- [ ] **Step 1: Failing tests** (`normative.rs` `#[cfg(test)]`): the OpenID claim list from Back-Channel Logout §2.4 gives rows `exp: REQUIRED. Expiration time, …`, `events: REQUIRED. … MUST be a JSON object …`, `nonce: PROHIBITED. A nonce Claim MUST NOT be present. …`, and no `sub: OPTIONAL.` row; the sentence `A Logout Token MUST contain either a sub or a sid Claim, and MAY contain both.`; a wrapped RFC 6749 paragraph gives `The client MUST NOT use more than one authentication method in each request.` as one sentence; the indented `client_id` / `REQUIRED.` rows of RFC 6749 §2.3.1 join their continuation line; `(… B., and C. Mortimore, “OpenID Connect Core 1.0,” …).` does not split at `C.`; a MAY-only sentence is dropped; `line` is the paragraph's first line.
- [ ] **Step 2:** `cargo test -p codecite-core normative` fails (no module).
- [ ] **Step 3:** Implement as in Design decisions.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit `feat(core): find a spec section's normative statements`.

### Task 2: `Store::requirements`

**Files:**
- Create: `crates/codecite-core/src/store/requirements.rs`
- Modify: `crates/codecite-core/src/store.rs`, `lib.rs` (exports), `error.rs` (`NoSectionOnTopic`, generic `NotASpecCitation` message), `store/read.rs` (`pub(super)` helpers as needed)

**Interfaces:**
- Consumes: `normative_statements`, `Store::read`, `Store::search`.
- Produces:
  - `pub struct RequirementsOptions { pub allow_stale: bool }`
  - `impl Store { pub fn requirements(&self, citation: &str, options: &RequirementsOptions) -> Result<Requirements>; pub fn requirements_on(&self, topic: &str, document: &str, options: &RequirementsOptions) -> Result<Requirements> }`
  - `pub struct Requirements { pub citation: String, pub title: Option<String>, pub license_class: String, pub indexed_at: String, pub sections: Vec<RequirementSection>, pub truncated: bool, pub instruction: Option<String> }` with `Display` (human form)
  - `pub struct RequirementSection { pub citation: String, pub title: Option<String>, pub relation: String /* asked|subsection|referenced|referring|context */, pub statements: Vec<Requirement> }`
  - `pub struct Requirement { pub citation: String, pub kind: String /* sentence|row|step */, pub keywords: Vec<String>, pub text: String }`

- [ ] **Step 1: Failing tests** in `requirements.rs`, on chunks written through `Store`: a step citation yields its section as `asked`; subsections follow; `Section 4.1` in text gives a `referenced` group but `Section 2 of [JWT]` doesn't; a referring subsection brings its parent as `context`; a title phrase gives `referenced`; MUST before steps before SHOULD; a code citation is `NotASpecCitation`; stale refused unless allowed; `requirements_on` picks the section the topic finds and errors with `no_section_on_topic` otherwise.
- [ ] **Step 2:** fail. **Step 3:** implement. **Step 4:** pass.
- [ ] **Step 5:** Commit `feat(core): list the requirements governing a spec section`.

### Task 3: CLI `codecite requirements` and the AC tests on real pages

**Files:**
- Modify: `crates/codecite/src/main.rs`
- Create: `crates/codecite/tests/requirements.rs`, fixtures `crates/codecite-index/tests/fixtures/spec/openid-connect-backchannel-1_0.html` and `rfc6749.html` (the pinned pages, unmodified, so their sha256 matches `corpora/identity.toml`)

**Interfaces:** `codecite requirements <corpus> [<citation>] [--topic <t> --document <d>] [--json] [--allow-stale]`; `--json` prints `Requirements`.

- [ ] **Step 1: Failing test** `back_channel_logout_2_6_lays_out_its_rules`: seed `XDG_CACHE_HOME/codecite/documents/<sha256>` with the fixture, write a manifest pinning it, `codecite index`, then `codecite requirements specs "Back-Channel Logout §2.6" --json`; assert a `#section-2.4` group holds statements containing `exp: REQUIRED`, `MUST be a JSON object` and `nonce Claim MUST NOT be present`, and the `#section-2.6` group holds `step-2` … `step-8`. Test `rfc_6749_2_3_lays_out_its_rules`: `MUST NOT use more than one authentication method` under `#section-2.3`, `MUST NOT be included in the request URI` under `#section-2.3.1`, `MUST NOT be included more than once` under `#section-3.2`. Plus the human form names each section.
- [ ] **Step 2:** fail. **Step 3:** add the subcommand. **Step 4:** pass.
- [ ] **Step 5:** Commit `feat(cli): add codecite requirements`.

### Task 4: MCP `requirements` tool, instructions and docs

**Files:**
- Modify: `crates/codecite-mcp/src/server.rs`, `docs/usage.md`, `docs/design.md`

- [ ] **Step 1: Failing tests**: the tool list includes `requirements`; its input schema has `citation`, `topic`, `document`, `corpus`, `allow_stale`; a call returns what `Store::requirements` does; neither citation nor topic+document is a tool error (`bad_arguments`); results fit the output schema; the server instructions contain "before implementing or fixing anything a spec governs".
- [ ] **Step 2:** fail. **Step 3:** implement. **Step 4:** pass.
- [ ] **Step 5:** docs: usage.md section "List a section's requirements" and the MCP tool table; design.md data flow and the store bullets. Commit `feat(mcp): add the requirements tool`.

### Task 5: Gates and PR

- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- [ ] Push over HTTPS, open the PR (`Closes #157`, ACs, provenance), add the `eval` label.
