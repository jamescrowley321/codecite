# Definition Line Order Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `codecite def` and the MCP `definition` tool order definitions by where they sit in the code, so `#L82` comes before `#L116` (#298).

**Architecture:** `Store::definition` in `crates/codecite-core/src/store/definition.rs` sorts its matches by `(rank(role), citation)`. The citation is a string, so its line numbers compare as text. Keep the role rank first, then sort by the parts the citation is built from: source name, path, start line as a number, end line as a number, and the symbol ID last so equal places still have one order. `Found` carries those parts next to the `Definition` it already holds. CLI and MCP both print `Store::definition`'s result, so both change together and stay identical.

**Tech Stack:** Rust, rusqlite.

**Spec:** issue #298; `docs/design.md` (MCP tools: `definition`); `docs/usage.md` (`codecite def`).

## Global Constraints

- Role still comes first (source, docs, example, test): the issue fixes the tie order inside a role, not the role ranking.
- No SQL change: the rows come back as before and Rust sorts them, so the `symbol_name` index plan is untouched.
- Only `definition` changes. `references` and the graph tools sort their own way and are outside #298.

## Review Focus

- Two definitions in one file at lines 82 and 116: 82 first (the Keycloak case from #287).
- Two definitions with the same source, path and lines (an alias and its target can share a chunk's lines): the symbol ID breaks the tie, so the order never depends on SQLite's row order.
- The existing `an_ambiguous_symbol_returns_every_definition_in_a_fixed_order` and `at_most_50_are_returned_with_the_full_count` keep their exact expectations.

---

### Task 1: Sort definitions by source, path and numeric lines

**Files:**
- Modify: `crates/codecite-core/src/store/definition.rs` (`Found`, `Store::named`, `Store::definition`, the `Definitions::results` and `Store::definition` doc comments)
- Test: the `tests` module in the same file

**Interfaces:**
- Produces: `Found { role, id, source: String, path: String, lines: (u32, u32), definition }`; no public API change.

- [ ] Test `definitions_in_one_file_come_in_line_order`: a fixture source `app` with one `src/TokenEndpoint.java`-style file of at least 120 lines, holding a definition named `TokenEndpoint` at lines 82–120 (the class) and another named `TokenEndpoint` at 116–118 (its constructor), written in the reverse order (116 first) so neither write order nor citation text can pass the test. `definition("TokenEndpoint")` returns `#L82-L120` then `#L116-L118`. Build it with the existing `file(...)` helper and a generated text (`(1..=120).map(|i| format!("line {i}\n")).collect::<String>()`).
- [ ] Test `equal_places_fall_back_to_the_symbol_id`: two symbols with the same name at the same lines in the same file (different descriptors, e.g. `a().` and `b().`); the result is in ID order, twice the same.
- [ ] Run `cargo test -p codecite-core --lib definition`; the first test fails (`#L116` sorts before `#L82` as text).
- [ ] In `named`, fill `Found.source` (row 6), `Found.path` (row 4) and `Found.lines` (rows 2 and 3) from the values already read; read rows 2 and 3 once into `start`/`end` and pass them to `code_citation` too.
- [ ] In `definition`, replace the sort with
  ```rust
  found.sort_by(|a, b| {
      (rank(a.role), &a.source, &a.path, a.lines, &a.id)
          .cmp(&(rank(b.role), &b.source, &b.path, b.lines, &b.id))
  });
  ```
  > **Learn:** tuples implement `Ord` lexicographically: Rust compares the first fields, and only on a tie the next, so one tuple says "role, then source, then path, then lines". `(u32, u32)` compares numerically, which is the fix: as `String`s, `"116"` < `"82"` because `'1'` < `'8'`. Borrowing (`&a.source`) compares the strings without cloning them. `sort_by` is stable and takes a comparator; `sort_by_key` can't be used here because its key can't borrow from the element it was given.
- [ ] Update the doc comments: `Definitions::results` says "each by source, path and line", and `Store::definition`'s says "then by source, path, starting and ending line".
- [ ] `cargo test -p codecite-core --lib definition` passes, the existing ordering tests unchanged.
- [ ] Commit: `fix(def): order definitions by line number, not citation text (#298)`.

### Task 2: Describe the order in the docs

**Files:**
- Modify: `docs/design.md` (MCP tools, the `definition` bullet: "ties broken by citation in ascending order")
- Modify: `docs/usage.md` (`codecite def`: "each in citation order")

- [ ] `docs/design.md`: "ranked by role (`source`, then `docs`, `example`, `test`), then by source, path, starting line and ending line, numbers compared as numbers, and the symbol ID last, so a corpus always gives the same order".
- [ ] `docs/usage.md`: "source files first, then docs, examples and tests, each by source, path and line, so a file's definitions come top to bottom".
- [ ] `grep -n "citation order\|by citation" docs/usage.md docs/design.md` shows no remaining claim about `definition`'s order (the `references` ones stay; that tool is outside #298).
- [ ] Commit: `docs: describe the definition order (#298)`.

### Task 3: Verify end to end

- [ ] CLI and MCP parity: `cargo test -p codecite --test mcp` (the byte-for-byte case for `definition`) and `cargo test -p codecite --test def --test java_def` pass.
- [ ] Real CLI: build release, index a small local corpus with a Java file holding a class at a later line than a short method of the same name earlier in the text order (e.g. a class from line 82 and its constructor at 116), run `codecite def <corpus> <Name>` and `--json`, and see line 82 first. Adversarial: a name defined at `#L9` and `#L10` in one file (single- vs double-digit) also orders 9 then 10.
- [ ] History benchmark: `cargo build --release --locked -p codecite -p codecite-bench`, then `evals/history/run.sh`; commit `evals/history/results/`. `def` is not on the benchmark's path, so each repository's pooled `all` row for `codecite` must equal the committed `scorecard.md` on `origin/main` (no Recall@1, Recall@10 or MRR drop).
- [ ] Full gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.
- [ ] Commit the benchmark results: `test(evals): rerun the history benchmark for the definition order (#298)`.
