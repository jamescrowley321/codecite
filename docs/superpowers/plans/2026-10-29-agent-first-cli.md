# Agent-first CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `codecite` the CLI an agent reaches for first (#181): compact one-line-per-hit output, `--help` written for an agent learning the tool cold, an AGENTS.md/CLAUDE.md snippet, MCP instructions without spec advice, and output bytes and query time measured against `rg` on the history benchmark.

**Architecture:** The human forms in `crates/codecite/src/main.rs` (`human`, `human_matches`, `human_definitions`) become one line per hit; `--json` is untouched, so ranking and the history scorecard's recall rows can't move. Help text lives in clap attributes on `Cli` and `Command`. A new `cost` mode in `codecite-bench` times `codecite search` against `rg` per mined query; `evals/history/run.sh` runs it and the scorecard gains a cost table. The snippet lives once, in `docs/usage.md`, and the eval's codecite-arm note is built from it.

**Tech Stack:** Rust (clap derive, std::process, std::time::Instant), Python 3 (Inspect evals, pytest), bash, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-29-general-purpose-design.md` (goal 2) and issue #181.

## Global Constraints

- Agents use the CLI first; the MCP server is a bonus over the same commands.
- "The history benchmark doesn't lose on any repository, and output bytes per query and query time are reported next to `rg`'s." (#181 Done when)
- A change that loses on any repository's pooled row doesn't merge.
- Anti-goals: outputs worse, significantly slower, or less correct.
- `--json` output stays byte-identical to the MCP tools' text.
- Indexed text stays escaped through `escape_controls` in every human form.
- Name no repository outside the benchmark corpora in any artifact.
- Before every push: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, and `cd evals && uv run --locked pytest`.

## Review Focus

1. **A snippet with newlines or a very long first line.** Only the first non-blank line shows, cut to `HUMAN_SNIPPET_CHARS`; a hit never spans two lines. Task 1 pins it.
2. **A result with no snippet line** (blank snippet). The line ends after the kind and symbol, with no trailing ` · `. Task 1 pins it.
3. **A mined query with no searchable words** (`terms()` empty). The cost mode skips it for both engines instead of timing a failure. Task 5 pins it.
4. **`rg` missing from PATH.** The cost mode fails with an error naming `rg`, and run.sh reports that repository as failed rather than writing a zero row. Task 5 pins it.
5. **Mixed license or `indexed_at` across hits.** Those move onto each hit's line, still one line per hit. Task 1 keeps the existing `shared` logic and pins the one-line shape for it.

---

### Task 1: One line per search hit

**Files:**
- Modify: `crates/codecite/src/main.rs:859-919` (`human`)
- Test: `crates/codecite/src/main.rs` (unit tests module), `crates/codecite/tests/search.rs:149-169` (`the_human_form_is_short`)

**Interfaces:**
- Produces: the search line format, reused by Tasks 2 and 4:
  `<citation> · <kind>[ <symbol>][ · dirty][ · older version pinned][ · <license> · indexed <at>] · <first snippet line>`
  The header line stays `N result(s) · <license> · indexed <at>` when shared. No rank number and no score in the human form; `--json` keeps both.

- [ ] **Step 1: Write the failing tests**

In `tests/search.rs`, rewrite `the_human_form_is_short`: every line after the header starts with the abbreviated citation (`lines[1].split_once(" · ")` gives a citation containing `@` and `#L`), `lines[1]` contains `"function checkToken · "`, and `text.lines().count() == 1 + results.len()` where `results` comes from the same query with `--json`. In the unit tests module of `main.rs` add:

```rust
#[test]
fn a_search_hit_is_one_line() {
    let mut r = result("repo@0123456789abcdef0123456789abcdef01234567:src/a.rs#L1-L3");
    r.snippet = "\n  fn a() {\n  body\n".into();
    assert_eq!(
        human(&[r]),
        "1 result · MIT · indexed 2026-10-29T00:00:00Z\n\
         repo@0123456789ab:src/a.rs#L1-L3 · function a · fn a() {\n"
    );
}

#[test]
fn a_hit_without_a_snippet_ends_at_its_symbol() {
    let mut r = result("repo@0123456789abcdef0123456789abcdef01234567:src/a.rs#L1");
    r.snippet = "  \n".into();
    assert!(human(&[r]).ends_with("src/a.rs#L1 · function a\n"));
}

#[test]
fn mixed_provenance_stays_on_each_hits_line() {
    let a = result("x@0123456789abcdef0123456789abcdef01234567:a#L1");
    let mut b = result("y@0123456789abcdef0123456789abcdef01234567:b#L1");
    b.license_class = "quote".into();
    let text = human(&[a, b]);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.lines().all(|l| l.contains(" · indexed ")), "{text}");
}
```

`result(citation)` is a test helper building a `SearchResult` with kind `function`, symbol `a`, license `MIT`, `indexed_at` `2026-10-29T00:00:00Z`, `dirty: None`, `older_versions: None`; add it if the module has none.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p codecite a_search_hit_is_one_line a_hit_without mixed_provenance the_human_form_is_short`
Expected: FAIL, the current form has a rank, a score and a second line.

- [ ] **Step 3: Rewrite `human(results: &[SearchResult]) -> String`**

Drop the rank and `{:.1}` score; append ` · <snippet line>` to the same line instead of `\n   `. Update its doc comment to describe the new line.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite`
Expected: PASS.

- [ ] **Step 5: Update `docs/usage.md` § Search** so its sample output shows the new form, then commit

```bash
git commit -am "feat(cli): print one line per search hit"
```

### Task 2: One line per grep match and per definition

**Files:**
- Modify: `crates/codecite/src/main.rs:673-722` (`human_matches`), `:803-850` (`human_definitions`)
- Test: `crates/codecite/tests/search.rs:228-262` (`grep_prints_cited_lines`), `crates/codecite/tests/def.rs:147-180`

**Interfaces:**
- Consumes: Task 1's line shape.
- Produces: grep line `<citation>[ · dirty][ · <license> · indexed <at>]: <matched line, trimmed and cut>` (a colon, as `rg` prints `path:line:text`); definition line `<citation> · <kind> <symbol> · <role>[ · dirty] · <first snippet line>`, with the license and `indexed_at` moved into the header (`3 definitions of callback · ambiguous · adapt · indexed …`) when every definition shares them, otherwise onto each line.

- [ ] **Step 1: Write the failing tests**

`grep_prints_cited_lines`: `text.contains(":src/lib.rs#L1 · dirty: pub fn validate(")` and `text.lines().count() == 1 + matches.len()`. `an_ambiguous_symbol_lists_every_definition_source_first`: `lines[0]` starts with `"3 definitions of callback · ambiguous · adapt · indexed "`; `lines[1]` contains `":src/client.rs#L4-L6 · method callback · source · pub fn callback(&self) -> bool {"`; `lines.len() == 4`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p codecite --test search --test def`
Expected: FAIL.

- [ ] **Step 3: Rewrite `human_matches(&[GrepMatch]) -> String` and `human_definitions(&Definitions) -> String`** to the formats above; keep `escape_controls` on every indexed field and the `, the first N` header note.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite`
Expected: PASS.

- [ ] **Step 5: Update `docs/usage.md` § Grep and § Find a definition samples, then commit**

```bash
git commit -am "feat(cli): print one line per grep match and definition"
```

### Task 3: Help written for an agent learning the tool cold

**Files:**
- Modify: `crates/codecite/src/main.rs:16-228` (`Cli`, `Command` attributes)
- Test: `crates/codecite/src/main.rs` unit tests, `crates/codecite/tests/cli.rs`

**Interfaces:**
- Produces: `codecite --help` text that Task 4's snippet points at.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_command_has_examples_in_its_help() {
    use clap::CommandFactory as _;
    for sub in Cli::command().get_subcommands() {
        let help = sub.get_after_long_help().map(ToString::to_string).unwrap_or_default();
        assert!(help.contains("Examples:"), "`{}` has no examples", sub.get_name());
    }
}

#[test]
fn the_top_level_help_says_where_to_start() {
    use clap::CommandFactory as _;
    let help = Cli::command().render_long_help().to_string();
    for line in ["codecite corpora", "codecite search", "codecite read", "--json"] {
        assert!(help.contains(line), "{help}");
    }
    assert!(!help.contains("spec would use"), "{help}");
}
```

In `tests/cli.rs`, `codecite search --help` exits 0 and its stdout contains `Examples:` and `Output:`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p codecite every_command_has_examples the_top_level_help --test cli`
Expected: FAIL.

- [ ] **Step 3: Write the help**

`Cli` gets `long_about` (what a corpus and a citation are, in two sentences) and `after_long_help` with this start-here block, verbatim:

```text
Start here:
  codecite corpora                       what is indexed, and how fresh
  codecite search <CORPUS> "<words>"     ranked hits, one line each, cited
  codecite grep <CORPUS> '<regex>'       every matching line, cited
  codecite def <CORPUS> <SYMBOL>         where a symbol is defined
  codecite read <CORPUS> '<CITATION>'    the exact text a citation points at
Add --json to any command for structured output. Quote citations as printed.
```

Each `Command` variant gets `after_long_help` with an `Examples:` block of two or three real invocations and, for `search`, `grep`, `def` and `read`, an `Output:` line describing the human form from Tasks 1–2. The `search` query's doc becomes general: search with the words the code or docs would use (identifiers, error text, headings), not the question's phrasing. `cite`, `requirements` and `diff` keep their spec wording; they are the spec adapter (#182).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(cli): write --help for an agent learning codecite cold"
```

### Task 4: The agent snippet, the eval note, and MCP instructions without spec advice

**Files:**
- Modify: `docs/usage.md` (new section `## Tell your agent`, after `## Connect an agent`), `crates/codecite-mcp/src/server.rs:549` (`instructions`) and its tests at `:840-860`, `:1196-1205`, `evals/src/codecite_evals/task.py:64-77` (`codecite_note`)
- Test: `crates/codecite-mcp/src/server.rs` tests, `evals/tests/test_task.py`

**Interfaces:**
- Produces: `task.snippet(corpus: str) -> str` returning the snippet with `<CORPUS>` replaced; `codecite_note(task_dir)` = `"codecite indexes <what> as its corpus `eval`.\n\n" + snippet("eval") + MCP line`, where the MCP line names every entry of `CODECITE_TOOLS` (pi still reaches codecite through MCP).

- [ ] **Step 1: Write the failing tests**

server.rs: replace `the_instructions_say_when_to_list_requirements` with `the_instructions_give_no_spec_advice`: the instructions contain none of `requirements`, `older_versions`, `BCP 14`, `spec`; they still contain `corpora`, `search`, `grep`, `definition`, `read`, `context`, and the existing injection-safety phrases. test_task.py:

```python
def test_the_snippet_comes_from_usage_md():
    text = (config.ROOT / "docs" / "usage.md").read_text()
    s = task.snippet("eval")
    assert "<CORPUS>" not in s and 'codecite search eval "' in s
    assert s.replace("eval", "<CORPUS>") in text

def test_the_note_is_cli_first():
    note = task.codecite_note(config.task_dir("fastapi-middleware"))
    assert note.index("codecite search eval") < note.index("mcp__codecite__search")
```

(Use whatever `config` already exposes for the repo root; add `ROOT` if it has none.) Keep `test_the_codecite_note_names_the_specs_only_when_the_corpus_has_them` and the `CODECITE_TOOLS in note` check.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p codecite-mcp instructions && (cd evals && uv run --locked pytest tests/test_task.py)`
Expected: FAIL.

- [ ] **Step 3: Write the snippet, the note and the instructions**

The snippet sits in `docs/usage.md` between `<!-- agent-snippet -->` and `<!-- /agent-snippet -->`, inside a fenced `markdown` block, verbatim:

```markdown
## codecite

This repository is indexed by codecite as the corpus `<CORPUS>`. Search it before reading files wholesale or grepping wide:

- `codecite search <CORPUS> "<words the code would use>"`: ranked hits, one line each, cited
- `codecite grep <CORPUS> '<regex>'`: every matching line, cited
- `codecite def <CORPUS> <SYMBOL>`: where a symbol is defined
- `codecite read <CORPUS> '<CITATION>'`: the exact text a citation points at

Quote citations as codecite prints them. `codecite <command> --help` has the flags; `--json` gives structured output.
```

`task.snippet` reads the fenced block between the markers and fails loudly if the markers are missing. The MCP `instructions` keep: start with `corpora`; `search`, `grep`, `definition`, then `read` a citation; search with the words the source would use, not the asker's; `context` before editing a file; quote `quote` sources exactly; and the injection-safety sentence. They drop the spec-terms example, `requirements`, `older_versions` and `diff`. Those tools' own descriptions are unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite-mcp && (cd evals && uv run --locked pytest)`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat: an agent snippet for AGENTS.md, used by the eval, and MCP instructions without spec advice"
```

### Task 5: Output bytes and query time against `rg`

**Files:**
- Create: `crates/codecite-bench/src/cost.rs`
- Modify: `crates/codecite-bench/src/main.rs` (flag, dispatch, `engine`), `crates/codecite-bench/src/grep.rs` (`terms` becomes `pub(crate)`), `evals/history/run.sh`, `evals/src/codecite_evals/history.py` (`scorecard`, `scorecard_main`), `.github/workflows/ci.yml` (bench job), `evals/README.md`
- Test: `crates/codecite-bench/src/cost.rs` unit tests, `evals/tests/test_history.py`

**Interfaces:**
- Consumes: `grep::terms(text: &str) -> Vec<String>`.
- Produces: `codecite-bench --queries <q.toml> --cost <checkout> [--codecite <bin>]` printing JSON `{"engine":"cost","queries":N,"codecite":Cost,"rg_lines":Cost,"rg_files":Cost}`, `Cost = {"median_bytes":u64,"median_ms":f64,"p90_ms":f64}`; run.sh writes it as `results/<repo>-cost.json`; the scorecard gets a second table, `## Cost per query`, one row per repository: `| Repository | Queries | codecite bytes | codecite ms | rg -n bytes | rg -n ms | rg -l bytes | rg -l ms |` with medians, and p90 ms in parentheses.

- [ ] **Step 1: Write the failing tests**

cost.rs:

```rust
#[test]
fn rg_args_search_each_term_as_a_whole_word_in_any_case() {
    assert_eq!(
        rg_args(&["token".into(), "refresh".into()], Path::new("/c"), RgMode::Lines),
        ["-n", "-i", "-w", "-F", "-e", "token", "-e", "refresh", "--", "/c"]
    );
    assert_eq!(rg_args(&["x_y".into()], Path::new("/c"), RgMode::Files)[0], "-l");
}

#[test]
fn medians_and_p90_of_measurements() {
    let c = Cost::of(&[(10, 1.0), (30, 3.0), (20, 2.0), (40, 4.0)]);
    assert_eq!((c.median_bytes, c.median_ms), (25, 2.5));
    assert!((c.p90_ms - 3.7).abs() < 1e-9);
}

#[test]
fn a_query_with_no_words_is_not_measured() {
    assert!(measured_terms("a to of").is_none());
}

#[test]
fn a_missing_rg_is_named() {
    let err = run_timed(Path::new("rg-not-installed-here"), &[]).unwrap_err();
    assert!(err.to_string().contains("rg-not-installed-here"), "{err}");
}
```

test_history.py: `scorecard()` given a run with a `cost` report renders a `## Cost per query` table with that repository's medians; given none, it omits the table.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p codecite-bench cost && (cd evals && uv run --locked pytest tests/test_history.py)`
Expected: FAIL.

- [ ] **Step 3: Implement**

In cost.rs: `enum RgMode { Lines, Files }`; `rg_args(terms: &[String], dir: &Path, mode: RgMode) -> Vec<String>`; `measured_terms(text: &str) -> Option<Vec<String>>` (`None` when `terms` is empty); `run_timed(program: &Path, args: &[String]) -> anyhow::Result<(u64, f64)>` returning stdout bytes and wall milliseconds, the error naming the program; `Cost::of(&[(u64, f64)]) -> Cost` with linear-interpolated percentiles; `pub fn cost(queries: &[Query], corpus: &str, codecite: &Path, checkout: &Path) -> anyhow::Result<serde_json::Value>`. Each command runs three times per query and the fastest counts (bytes are the same every run). codecite runs as `search --allow-stale -- <corpus> <text>`: the default limit and the human form, what an agent sees; `rg` runs with exit code 1 (no match) counted as 0 bytes. `--cost` conflicts with `--grep` and `--popular`.

run.sh: after the popular report, extract the pinned tree with `git -C "$clone" archive "$commit" | tar -x -C "$checkout/$repo"` under the run's temp dir, then `report cost --codecite "$bin/codecite" --cost "$checkout/$repo"`. history.py reads `<repo>-cost.json` and renders the table after the recall table. ci.yml's bench job gets `sudo apt-get install -y ripgrep` before the history step. evals/README.md says what the cost table measures.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite-bench && (cd evals && uv run --locked pytest)`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(bench): measure output bytes and query time against rg"
```

### Task 6: Run the benchmark, align the docs, open the PR

**Files:**
- Modify: `evals/history/results/*`, `docs/design.md` (CLI output and MCP instructions where it describes them)

- [ ] **Step 1: Run the history benchmark**

Run: `cargo build --release --locked -p codecite -p codecite-bench && evals/history/run.sh` (in the background; it takes minutes).
Expected: exit 0; `scorecard.md` recall rows identical to main's (the human form doesn't reach `--json`), plus the cost table.

- [ ] **Step 2: Update `docs/design.md`** wherever it describes the CLI's human output or the MCP instructions, so the code and the docs agree.

- [ ] **Step 3: Run everything CI runs**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && (cd evals && uv run --locked pytest)`
Expected: all PASS.

- [ ] **Step 4: Commit, push and open the PR**

Title `feat(cli): make the CLI an agent's natural search tool`; body has `Closes #181`, the acceptance criteria, the recall scorecard (unchanged) and the cost table, and the AI provenance section with the ticked accountability box. Add the `eval` label.
