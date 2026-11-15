# codecite assist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `codecite assist <corpus> --tool grep|glob --input <json>` prints a short, cited block of what codecite knows about the agent's own grep or glob call, and a hook attaches it to that call's result, so codecite reaches the agent with no prompt asking for it (#162).

**Architecture:** One new module in the CLI crate, `crates/codecite/src/assist.rs`, built on the store methods the other commands already use (`definition`, `search`, `context`), and a thin `assist` command over it that never fails: any error prints nothing and exits 0.

1. **Read the call.** `--input` is the tool's input JSON: `pattern`, and optional `path` and `glob` (Claude Code's `Grep`, pi's `grep`) or `pattern` and `path` (Claude Code's `Glob`, pi's `find`). Stdin, when not a terminal, is the tool's output as the agent saw it; the files it names are "already matched".
2. **Decide what to look up.**
   - grep, a pattern that is one identifier (`^[A-Za-z_][A-Za-z0-9_]*$` after dropping `\b` anchors, or `Type::name` / `a.b`, whose last segment is taken) → `definition(symbol)`; nothing defined → fall through to search.
   - grep, any other pattern → the words of the pattern (regex metacharacters and escapes become spaces, words under 3 characters dropped) as a `search` query, specs included, limit 5.
   - glob → the words of the glob's literal parts (`**/*logout*.py` → `logout`; extensions and wildcard-only segments dropped) as a `search` query; no words → nothing.
   - a `path` that is a file → `context` for it: the guidance rules whose scope matches (#161's push, reused).
3. **Leave out what grep found.** A hit whose file appears in the tool's output is dropped (matched by the cited path being a suffix of an output path, so absolute and relative forms both match). Nothing left → print nothing.
4. **Print within a budget.** One line per hit, `<citation> · <kind>[ <symbol>] · <first line>`, the line cut at 120 characters, then the rules as `context` prints them, all under `--max-bytes` (default 2048): whole lines only, and a final `… N more: codecite search <corpus> "<query>"` line when lines were dropped. A one-line header names codecite, so the agent knows where the block came from: `codecite (<corpus>) also found, not in this result:`.

**Tech Stack:** Rust (`serde_json` for the input, `regex` already in the workspace for the identifier check), Python (the eval's arm), shell and TypeScript (the hook recipes).

## Global Constraints

- Done when (issue #162): `assist` exists with tests for a symbol pattern, a free-text pattern, a glob, an empty result and a missing or stale corpus (no output, exit 0); `codecite-bench` reports its p95 latency on the identity corpus; `docs/usage.md` has the Claude Code hook recipe and the pi equivalent; the eval has an `assist` arm and a quick-tier scorecard on at least one task is posted on the PR.
- Guardrails: p95 under 150 ms on the identity corpus, already indexed; never blocks (every error → no output, exit 0); no hits into files grep already matched.
- Add, never replace: the hook returns `additionalContext` (Claude Code) or appends a content block (pi). `updatedToolOutput` and PreToolUse input rewriting are out of scope.
- Nothing is installed into an agent's config automatically: recipes only.
- The history benchmark is still run (the CLI gains a command; `search` output is unchanged) and its results committed; any repository's pooled `all` row for `codecite` lower on Recall@1, Recall@10 or MRR than `scorecard.md` on `origin/feat/rework-tracking` is a loss.
- Before every push: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`, and `(cd evals && uv run --locked pytest)`.
- The owner is learning Rust: the PR body explains the idioms marked **Learn:** below.

## Decisions taken here (DEC-013 in the loop's journal)

- **What grep found comes on stdin, as the tool's output text**, not as a parsed list. Claude Code's `tool_response` and pi's result text differ in shape; both name the files as path strings, so assist scans the text for the corpus's own paths. The hook passes `.tool_response` (serialized) or the result's text. This keeps assist independent of either agent's response schema.
- **Symbol detection is lexical**, not a lookup of every word: only a pattern that is one identifier (or a qualified path) goes to `def`. A free-text or regex pattern goes to `search`.
- **The `assist` eval arm keeps `/opt/codecite` but off `PATH`**, with no MCP config and no prompt note: the hook calls the binary by its absolute path, so the only way codecite reaches the agent is the hook.
- **Measuring the block:** the arm's hook appends one JSON line per call to `/tmp/codecite-assist.jsonl` (`{"tool", "bytes"}`); the agent wrapper reads it after the run into the store as `assist_calls`, `assist_blocks` (non-empty) and `assist_bytes`. Tokens added are reported as bytes / 4, labelled as an estimate. An `assist` run counts as having used codecite when `assist_blocks > 0`, in place of `codecite_calls`.

## Files

- Create `crates/codecite/src/assist.rs` — `Call::parse`, `lookup`, `render`.
- Modify `crates/codecite/src/main.rs` — `Command::Assist`, `fn assist`.
- Create `crates/codecite/tests/assist.rs`.
- Modify `crates/codecite-bench/src/main.rs`; create `crates/codecite-bench/src/assist.rs` — `--assist` latency mode.
- Create `experiments/assist-latency/` — `run.sh`, `results.txt`.
- Modify `evals/src/codecite_evals/config.py`, `task.py`, `claude_code.py`, `report.py`; create `evals/src/codecite_evals/assist.py` (the hook script and pi extension text the sandbox gets); tests in `evals/tests/test_task.py`, `test_claude_code.py`, `test_report.py`.
- Modify `.github/workflows/evals.yml` — `assist` in the `arms` choice.
- Modify `docs/usage.md` (a "Add codecite to the agent's own searches" section after "Push the rules on every edit"), `docs/design.md` (Interfaces row for `assist`; the evaluation plan's arms), `evals/README.md` (the `assist` arm).

## Task 1: parse the call (`assist.rs`)

- [ ] Write failing unit tests in `assist.rs`'s `#[cfg(test)] mod tests`:
  - `{"pattern": "validate_logout_token"}` with tool grep → `Lookup::Symbol("validate_logout_token")`; `\bvalidate_logout_token\b` the same; `Store::open_corpus` → `Symbol("open_corpus")`.
  - `{"pattern": "fn\\s+validate.*token"}` → `Lookup::Search("validate token")` (`fn` dropped, under 3 characters).
  - glob `{"pattern": "**/*logout*.py"}` → `Search("logout")`; `{"pattern": "**/*.rs"}` → `Lookup::Nothing`.
  - `{"path": "src/a.py", ...}` keeps `path`; a missing or non-string `pattern` → an error.
- [ ] Implement `#[derive(Deserialize)] struct ToolInput { pattern: String, path: Option<String>, glob: Option<String> }`, `enum Tool { Grep, Glob }` (a `clap::ValueEnum`), `enum Lookup { Symbol(String), Search(String), Nothing }` and `fn lookup(tool, &ToolInput) -> Lookup`.
  **Learn:** an enum with data (`Lookup::Symbol(String)`) carries exactly what each case needs; `match` makes every case handled.
- [ ] `cargo test -p codecite assist::` passes; commit `feat(cli): read an agent's grep or glob call for assist (#162)`.

## Task 2: look up, drop what grep found, render

- [ ] Failing unit tests for `render`: hits whose path is in the output text are dropped; no hits left → empty string; the budget keeps whole lines and adds the `… N more` line; the header names the corpus.
- [ ] Implement `fn already_matched(output: &str, path: &str) -> bool` (a line of the output ends with the path, or contains `/<path>`), `fn run(store, &Call, output, max_bytes) -> anyhow::Result<String>` calling `store.definition(symbol, false)` (a not-found error → search instead), `store.search(&SearchQuery { query, limit: 5, .. })` and, when `path` names an existing file, `store.context` through `ContextTarget::inspect`, and `fn render(...) -> String`.
  **Learn:** returning `String` from a pure `render` and printing in `main` keeps the never-fail rule in one place.
- [ ] Commit `feat(cli): look up and render the assist block (#162)`.

## Task 3: the `assist` command

- [ ] Failing integration tests in `crates/codecite/tests/assist.rs`, built on the fixture corpora the `def` and `search` tests use (`tests/def.rs`, `tests/search.rs`): a symbol pattern prints its definition; a free-text pattern prints search hits; a glob prints hits; stdin naming the only hit's file → no output; a pattern with nothing behind it → no output; a missing corpus → no output, exit 0; a stale corpus → no output, exit 0; malformed `--input` → no output, exit 0.
- [ ] Add `Command::Assist { corpus, tool: Tool, input: String, max_bytes: usize (default 2048), root: Option<PathBuf> }` with help and `Examples:` (the `every_command_has_examples_in_its_help` test checks this), and `fn assist` that reads stdin only when it is not a terminal (`std::io::IsTerminal`), calls `assist::run`, prints on `Ok` and swallows `Err`, always returning `Ok(())`. An env var `CODECITE_ASSIST_DEBUG=1` prints the error to stderr, for people writing hooks.
  **Learn:** `std::io::IsTerminal` and why a hook's stdin must not block.
- [ ] Hand run against the identity corpus: `codecite assist identity --tool grep --input '{"pattern":"validate_logout_token"}' </dev/null`, a regex pattern, a glob, and an adversarial input (`--input '[]'`, a 10 KB pattern). Commit `feat(cli): codecite assist (#162)`.

## Task 4: check Claude Code's PostToolUse payload, write the recipes

- [ ] In a scratch directory under `/var/tmp`, run `claude -p` with a PostToolUse hook on `Grep|Glob` that copies its stdin to a file, ask it to grep and glob once, and record the real `tool_input` and `tool_response` shapes (keep the captured JSON as `crates/codecite/tests/fixtures/assist/claude-grep.json` and `claude-glob.json`, and test that `assist` reads them). Confirm user-level `~/.claude/settings.json` hooks load under `claude -p` (the eval relies on it).
- [ ] `docs/usage.md`: a section "Add codecite to the agent's own searches" with the Claude Code hook script (jq: `.tool_input` → `--input`, `.tool_response | tojson` → stdin, output wrapped as `{hookSpecificOutput: {hookEventName: "PostToolUse", additionalContext: ...}}`, nothing printed for an empty block), the settings JSON, and the pi extension (`tool_result` on `grep` and `find`, appending a text block, as the `context` extension does). State the guardrails: quiet when it has nothing, exit 0 on errors, a 2 KB budget.
- [ ] `docs/design.md`: the Interfaces row for `assist`.
- [ ] Commit `docs: the assist hook recipes for Claude Code and pi (#162)`.

## Task 5: p95 latency in `codecite-bench`

- [ ] `codecite-bench --assist` runs `codecite assist <corpus> --tool grep --input {"pattern": <query text>}` for every query in the queries file (the identity corpus's labelled queries, `evals/retrieval/queries.toml`), and once more per query with the query's first word as a symbol-style pattern; three runs each, fastest counts, reports median and p95 ms and the share of non-empty blocks. Reuse `cost.rs`'s timing and percentile code (extend `Cost` with `p95_ms` if needed).
- [ ] `experiments/assist-latency/run.sh` builds release, runs it on `identity-bench`, writes `results.txt`; README line naming the 150 ms guardrail and the result. A p95 over 150 ms → profile the slow case before going on (likely the `definition` miss falling back to `search`).
- [ ] Commit `feat(bench): measure assist's latency (#162)`.

## Task 6: the `assist` eval arm

- [ ] Failing pytest: `config.ARMS["assist"] == (("baseline", "baseline"), ("assist", "assist"))`; `pi_command("assist", ...)` loads only the assist extension (`-e <path>`), no MCP, no codecite tools; `claude_code.command("assist", ...)` passes no MCP config; `prepare_arm` for `assist` keeps `/opt/codecite`, leaves it off `PATH` and writes the hook files; a measures function over a sample `/tmp/codecite-assist.jsonl` returns `assist_calls`, `assist_blocks`, `assist_bytes`; report.py counts an assist run as used when `assist_blocks > 0` and shows blocks and estimated tokens for the arm.
- [ ] Implement: `assist.py` holds the hook script (absolute `/opt/codecite/bin/codecite`, corpus from the image's one corpus) and the pi extension source; `prepare_arm` writes them (Claude Code: `/root/.claude/settings.json` hooks + `/root/.claude/hooks/codecite-assist.sh`; pi: `/root/.pi/agent/codecite-assist.ts`); the agents read the log after the run. The prompt for `assist` is the baseline prompt (no note).
- [ ] `.github/workflows/evals.yml`: `assist` in the `arms` options and description; `evals/README.md` names the arm.
- [ ] `(cd evals && uv run --locked pytest)` passes. Commit `feat(evals): an assist arm, codecite only through the grep and glob hook (#162)`.

## Task 7: gates and the history benchmark

- [ ] Release build, `evals/history/run.sh`, compare the pooled `all` rows with `origin/feat/rework-tracking`'s `scorecard.md`; commit `evals/history/results/`.
- [ ] Full gates. The quick-tier `assist` scorecard is dispatched from the PR (`gh workflow run evals.yml --ref feat/assist-hook -f task=backchannel-logout -f arms=assist`, then `client-auth-uri`); the house-rules task (#160) does not exist yet, so it is not run. The PR says the quick tier is directional.
