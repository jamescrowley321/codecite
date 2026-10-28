# Spec version diff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `diff` MCP tool and `codecite diff` command that list the normative statements that differ between two pinned versions of a spec document (#158).

**Architecture:** `Store::diff` in a new `codecite-core/src/store/diff.rs` reads both versions' section and step chunks, splits them into units (`version_units`), pairs and classifies them as the spec says, and returns a `DiffResult`. The CLI and MCP server print that result as the other commands do. Search and read results gain `older_versions`.

**Tech Stack:** Rust, rusqlite, serde/schemars, rmcp, clap.

**Spec:** `docs/superpowers/specs/2026-10-28-spec-version-diff-design.md`

## Global Constraints

- No schema change, no re-index: only the stored `section`/`step` chunks.
- Keywords come from `codecite_core::requirement_keywords`.
- Texts are held to `MAX_SNIPPET_BYTES`; at most `MAX_SEARCH_LIMIT` (50) changes, with `total`.
- Errors reuse `Error::UnknownSource`, `Error::UnknownVersion`, `Error::NoSuchSection`; a stale index is refused unless `allow_stale`.
- CLI JSON and MCP structured content are the same bytes (`serde_json::Value` compact, sorted keys).
- Before every push: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.

## Review Focus

- A document with only one pinned version: `UnknownVersion` naming the versions indexed, not an empty diff. Test in Task 1.
- `from` equal to `to`: an empty diff, not an error. Test in Task 1.
- A section number in neither version: `NoSuchSection` with candidates from `to`. Test in Task 1.
- A section only one version has: all its statements added or removed, the other side's citation left out. Test in Task 1.
- An alias (`Back-Channel Logout`) in place of the document name resolves as `read` resolves aliases. Test in Task 3 (CLI).

---

### Task 1: `Store::diff` in core

**Files:**
- Create: `crates/codecite-core/src/store/diff.rs`
- Modify: `crates/codecite-core/src/store.rs`, `crates/codecite-core/src/lib.rs` (exports)

**Interfaces:**
- Produces: `pub struct DiffOptions<'a> { pub section: Option<&'a str>, pub from: Option<&'a str>, pub to: Option<&'a str>, pub allow_stale: bool }`; `pub struct DiffResult { document, from, to, section: Option<String>, license_class, indexed_at, total: usize, truncated: bool, changes: Vec<VersionChange>, reworded: Vec<VersionChange> }`; `pub struct VersionChange { change: String, section: String, from, to, from_text, to_text, from_strength, to_strength }` (options skipped when `None`); `impl Display for DiffResult`; `Store::diff(&self, document: &str, options: &DiffOptions) -> Result<DiffResult>`; `fn version_units(text: &str, start_line: u32, steps: &[(u32, u32, String, String)]) -> Vec<Unit>`.

- [ ] **Step 1: Unit tests for `version_units`, pairing and classifying** — term line joins its description (`"exp\nREQUIRED. Expiration time"` is one unit with term `exp`); a step is one unit and its lines leave the section's units; an indented run up to a blank line is one unit; date-only differences pair as same content; `occured`→`occurred` is `reworded`; `OPTIONAL`→`REQUIRED` is `strength_changed`; `iat` → `iat, and exp` is `changed`; terms `exp` and `iat` never pair.
- [ ] **Step 2: Store tests** with fixture rows written through `Store` (as `read.rs` tests do): added/removed/strength, one version only → `UnknownVersion`, `from == to` → empty, missing section → `NoSuchSection`, section only in one version.
- [ ] **Step 3: Run, see them fail** — `cargo test -p codecite-core diff`.
- [ ] **Step 4: Implement** as the spec describes.
- [ ] **Step 5: Run, see them pass; commit** `feat(core): diff the normative statements of two pinned spec versions`.

### Task 2: Back-Channel Logout fixtures (AC1, AC2)

**Files:**
- Create: `crates/codecite-index/tests/fixtures/spec/backchannel-1_0.html`, `backchannel-1_0-final.html` — each page's head and §2.3 to §2.8, cut from the pinned pages (sha256 `3b5c933d…` and `02343424…`) without other edits.
- Modify: `crates/codecite-index/src/index.rs` tests (they serve fixtures over the test `Server` and index them).

- [ ] **Step 1: Test `backchannel_logout_exp_is_added_and_required`** — index both fixtures as versions of `openid-connect-backchannel-1_0` (the final one not current), `diff` with `section: Some("2.4")`: exactly one change, `change == "added"`, `to_text` starts `"exp\nREQUIRED."`, `to_strength == ["REQUIRED"]`, `from` and `to` are the two versions' `#section-2.4` citations; `reworded` is empty.
- [ ] **Step 2: Test `backchannel_logout_response_has_no_normative_change`** — §2.8: `changes` empty (its one difference, "occured" → "occurred", is in `reworded`).
- [ ] **Step 3: Run; fix; commit** `test(index): diff Back-Channel Logout 1.0 against its pre-errata version`.

### Task 3: CLI `codecite diff`, MCP `diff`, `older_versions`

**Files:** `crates/codecite/src/main.rs`, `crates/codecite-mcp/src/server.rs`, `crates/codecite-core/src/store/{search,read}.rs`, tests in `crates/codecite/tests/diff.rs` and `crates/codecite/tests/mcp.rs`.

- [ ] **Step 1: Tests** — `codecite diff demo <doc> --section 4.3 --json` prints `Store::diff`'s JSON; an alias works; the MCP `diff` tool returns the same text; spec search and read results of a current version with an older one carry `older_versions`, others don't.
- [ ] **Step 2: Implement**: `Command::Diff { corpus, document, section, from, to, json, allow_stale }`; `DiffParams { document, section?, from?, to?, corpus?, allow_stale }`; `older_versions: Option<Vec<String>>` on `SearchResult` and `ReadResult`; server instructions mention `diff`.
- [ ] **Step 3: Run the whole suite; commit** `feat(mcp): diff a spec's pinned versions to show what changed in its rules`.

### Task 4: Docs

- [ ] `docs/usage.md`: the command and the tool. `docs/design.md`: how versions are compared, the CLI/MCP tables, `older_versions`. Commit `docs: describe the spec version diff`.
