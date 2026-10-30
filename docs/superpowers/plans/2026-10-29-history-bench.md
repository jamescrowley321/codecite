# History Benchmark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure codecite against plain grep on five of the owner's repositories, with labels mined from merged PRs, never written by hand (#180).

**Architecture:** A Python script in the `codecite-evals` package mines merged PRs through `gh` into a frozen queries file and a pinned manifest per repository, under `evals/history/<repo>/`. `codecite-bench` gains file-level answers and a `--grep` baseline that ranks files with `git grep`, so codecite and grep share one scorer. A runner indexes each repository, runs both, and writes the scorecard.

**Tech Stack:** Rust (`codecite-bench`), Python 3 with uv (`evals/`), `gh`, `git`.

**Spec:** `docs/superpowers/specs/2026-10-29-general-purpose-design.md` ("How it is measured"). This PR commits the spec with the plan.

## Global Constraints

- Repositories: `jamescrowley321/identity-model`, `identity-stack`, `blind-peer-review`, `terraform-provider-descope`, `codecite`.
- Classes: `history-title` (the PR title) and `history-issue` (the first closing issue's title, a blank line, then the body's first paragraph that isn't a heading, cut at a word boundary to at most 400 characters).
- An answer is a file the PR modified (`changeType` `MODIFIED`, or a rename's old path) that exists at the pinned commit. Leave out lockfiles (`Cargo.lock`, `uv.lock`, `poetry.lock`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `go.sum`) and `CHANGELOG*`.
- A PR is left out when it has no answer file, or its author is a bot (`dependabot`, `renovate`, `github-actions`, or a login ending `[bot]`).
- At most 40 PRs per repository, newest first. The pinned commit is the `baseRefOid` of the oldest sampled PR.
- Mined files are frozen: CI fails a PR that changes `evals/history/*/queries.toml` or `manifest.toml` except through `--remine`, a new version noted in the file header. Use the existing frozen-check pattern (`evals/src/codecite_evals/frozen.py`).
- The bench stays advisory in CI, never a required check.
- Before every push: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, and `uv run pytest` in `evals/`.

## Review Focus

1. A PR whose title or issue holds characters that are FTS5 or TOML syntax (`"`, `\`, `*`, a newline) must mine and search without error. Tested in Task 3 (TOML) and Task 2 (grep terms).
2. A renamed file's answer is its old path, the one that exists at the pinned commit. Tested in Task 3.
3. A query whose words match nothing returns an empty list from grep and from codecite, and scores 0, not an error. Tested in Task 2.
4. Squash-merged PRs on long-lived branches: an answer file deleted before the pinned commit is dropped, not left as an answer that nothing can find. Tested in Task 3.
5. One repository failing to index must not hide the others' results: the runner reports it and goes on. Tested in Task 4.

---

### Task 1: File-level answers in `codecite-bench`

**Files:**
- Modify: `crates/codecite-bench/src/answer.rs`

**Interfaces:**
- Produces: `Answer::File { path: String }`, parsed from a label with no `#`. It matches any `Citation::Code` whose path is equal. `Answer::parse` still rejects an empty label.

- [ ] **Step 1: Write the failing tests** in `answer.rs`:

```rust
#[test]
fn a_bare_path_is_a_file_answer() {
    assert_eq!(Answer::parse("src/app.py").unwrap(), Answer::File { path: "src/app.py".into() });
    assert!(Answer::parse("").is_err());
}

#[test]
fn a_file_answer_matches_any_lines_of_its_file() {
    let file = Answer::parse("src/app.py").unwrap();
    assert!(file.matches(&Citation::parse("repo@0123abcd:src/app.py#L5-L9")));
    assert!(!file.matches(&Citation::parse("repo@0123abcd:src/app.pyc#L5")));
    assert!(!file.matches(&Citation::parse("rfc9449@sha256:2d8657f07291#section-4.3")));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p codecite-bench answer`
Expected: FAIL, since the label has no `#`.

- [ ] **Step 3: Implement it.** In `Answer::parse`, a label with no `#` that is non-empty is `Answer::File`. Add the `matches` arm. Update the queries-file comment in `evals/retrieval/queries.toml` with the third answer form.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p codecite-bench`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codecite-bench/src/answer.rs evals/retrieval/queries.toml
git commit -m "feat(bench): accept a file path as an answer"
```

---

### Task 2: A grep baseline in `codecite-bench`

**Files:**
- Create: `crates/codecite-bench/src/grep.rs`
- Modify: `crates/codecite-bench/src/main.rs` (flags, dispatch, the report's `engine` field)

**Interfaces:**
- Produces:
  - CLI flags `--grep <git-dir>` and `--commit <sha>`, both required together. With them, the bench ranks with grep instead of running `codecite search`.
  - `fn grep_rank(git_dir: &Path, commit: &str, text: &str, depth: u32) -> anyhow::Result<Vec<String>>`, which returns citations `grep@<commit>:<path>#L1` that `Citation::parse` reads as code.
  - The text and JSON reports name the engine: `codecite` or `grep`.

Ranking uses only the question's words, as an agent's grep would. Terms are the lowercased runs of letters, digits and `_` of at least 3 characters, without the stopwords below, deduplicated. For each term, run `git -C <git-dir> grep -c -i -F -w -e <term> <commit>` (exit code 1 means no match, not an error). A file scores the number of distinct terms it matches, with ties broken by total matches and then by path. The top `depth` files are returned. The stopwords are `the and for with that this from into when what are not was but have has its our use add fix feat chore docs test`.

- [ ] **Step 1: Write the failing tests** in `grep.rs`, using a temporary git repository (`git init`, two commits, as `codecite-index`'s `test_repo.rs` builds one):

```rust
#[test]
fn files_matching_more_query_words_rank_first() {
    // a.py holds "token" and "refresh"; b.py holds only "token"
    let ranked = grep_rank(repo.path(), &head, "Refresh the token", 10).unwrap();
    assert_eq!(paths(&ranked), ["a.py", "b.py"]);
}

#[test]
fn a_query_with_no_matching_word_ranks_nothing() {
    assert!(grep_rank(repo.path(), &head, "zzzz qqqq", 10).unwrap().is_empty());
}

#[test]
fn query_syntax_is_searched_as_text() {
    assert!(grep_rank(repo.path(), &head, r#"fix "quoted" * \ path"#, 10).is_ok());
}

#[test]
fn ranks_the_pinned_commit_not_the_work_tree() {
    // c.py added after `head`
    assert!(!paths(&grep_rank(repo.path(), &head, "only_in_c", 10).unwrap()).contains(&"c.py"));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p codecite-bench grep`
Expected: FAIL, since `grep_rank` is not defined.

- [ ] **Step 3: Implement `grep_rank` and the flags.** `git grep -c` prints `<commit>:<path>:<count>`. Strip the commit prefix and split the count at the last `:`.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p codecite-bench`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codecite-bench/src
git commit -m "feat(bench): rank with git grep as a baseline"
```

---

### Task 3: Mine merged PRs into frozen queries

**Files:**
- Create: `evals/src/codecite_evals/history.py`
- Create: `evals/tests/test_history.py`, `evals/tests/fixtures/history/prs.json`
- Modify: `evals/pyproject.toml` (script `codecite-history-mine = "codecite_evals.history:main"`)

**Interfaces:**
- Produces:
  - `codecite-history-mine <owner/repo> --clone <dir> [--max 40] [--out evals/history]` writes `evals/history/<repo>/queries.toml` and `evals/history/<repo>/manifest.toml`.
  - Pure functions, tested without the network: `select_prs(prs: list[dict], exists: Callable[[str, str], bool], max_prs: int) -> tuple[str, list[Mined]]`, which returns the pinned commit and the mined PRs, and `issue_text(title: str, body: str) -> str`.
  - `queries.toml`: `corpus = "history-<repo>"`, a header comment with the repository, pinned commit, mining date and version `1`, and per PR one `[[query]]` per class, with `id = "<repo>-pr<n>-title"` or `"-issue"`, `class`, `text`, `answers` (paths) and `source` (the PR URL).
  - `manifest.toml`: `name = "history-<repo>"` and one `git` source (`url`, `commit`, `license` from `gh repo view --json licenseInfo`, `use = "adapt"`).

It reads PRs with `gh pr list -R <repo> --state merged --limit 500 --json number,title,url,author,baseRefOid,files,closingIssuesReferences`, then `gh issue view` for each closing issue. It clones or fetches `--clone` and checks existence with `git cat-file -e <commit>:<path>`. TOML is written with `tomli_w`, or with hand-escaped basic strings if adding that dependency is refused.

- [ ] **Step 1: Write the failing tests** in `test_history.py`, over `prs.json` (six PRs: a normal one; one by `dependabot[bot]`; one touching only `uv.lock`; one with a renamed file; one whose file is gone at the pinned commit; one closing an issue whose title holds `"`, `\` and a newline):
  - `test_bots_and_lockfile_only_prs_are_left_out`
  - `test_the_pinned_commit_is_the_oldest_sampled_base`
  - `test_a_renamed_file_answers_with_its_old_path`
  - `test_a_file_missing_at_the_pinned_commit_is_dropped`
  - `test_issue_text_is_title_and_first_paragraph_capped_at_400`, with a body that starts with `## Why`, which is skipped
  - `test_written_toml_round_trips_special_characters` (write, then `tomllib.loads`, and compare)
  - `test_max_keeps_the_newest`

- [ ] **Step 2: Run them to see them fail**

Run: `cd evals && uv run pytest tests/test_history.py -q`
Expected: FAIL on import.

- [ ] **Step 3: Implement `history.py`.**

- [ ] **Step 4: Run them to see them pass**

Run: `cd evals && uv run pytest -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add evals/src/codecite_evals/history.py evals/tests evals/pyproject.toml evals/uv.lock
git commit -m "feat(evals): mine merged PRs into frozen history queries"
```

---

### Task 4: Mine the five repositories, run, and score

**Files:**
- Create: `evals/history/<repo>/queries.toml`, `evals/history/<repo>/manifest.toml` (five repositories, mined)
- Create: `evals/history/run.sh`
- Create: `evals/history/results/<repo>-codecite.json`, `<repo>-grep.json`, `evals/history/results/scorecard.md`
- Modify: `evals/src/codecite_evals/frozen.py` and its test (freeze `evals/history/*/{queries,manifest}.toml`)

**Interfaces:**
- Consumes: Task 1's file answers, Task 2's `--grep`, Task 3's miner.
- Produces: `evals/history/run.sh [repo …]`. For each repository it runs `codecite index history-<repo> --manifest evals/history/<repo>/manifest.toml`, then `codecite-bench --queries evals/history/<repo>/queries.toml --json`. Then it runs `codecite-bench --grep <clone> --commit <pinned> --json` over the same queries, writes both JSON files, and rebuilds `scorecard.md`. A repository that fails to index is reported in the scorecard as `failed: <first error line>`, and the run goes on and exits non-zero at the end.

- [ ] **Step 1: Mine.** Run `codecite-history-mine` for each repository, with clones under `~/.cache/codecite/history/`. Check each file has at least 20 queries; for a repository below that, say so in the PR rather than loosening the rules.
- [ ] **Step 2: Freeze them.** Add the paths to `frozen.py`, with a test that a change to a mined file fails unless its header's version moves. Run `cd evals && uv run pytest -q`; expected PASS.
- [ ] **Step 3: Write `run.sh`, and run it** with a release build (`cargo build --release --locked -p codecite -p codecite-bench`).
Expected: `scorecard.md` has one table, a row per repository × class × engine with Recall@1, Recall@10 and MRR and the query count, and each repository's index time and chunk count from `codecite index`'s summary.
- [ ] **Step 4: Commit**

```bash
git add evals/history evals/src/codecite_evals/frozen.py evals/tests
git commit -m "feat(evals): the history benchmark's first scorecard"
```

---

### Task 5: CI, docs, and the PR

**Files:**
- Modify: `.github/workflows/ci.yml` (the `bench` job runs `evals/history/run.sh` after the identity bench and appends `scorecard.md` to the job summary, caching `~/.cache/codecite` keyed on `hashFiles('evals/history/*/manifest.toml')`)
- Modify: `evals/README.md` (a History benchmark section: what it measures, how to run and remine it, and that its labels are never edited by hand)
- Modify: `docs/design.md` (Evaluation plan: the history benchmark beside the retrieval benchmark, and the multi-repo rule from the spec: a change that loses on any repository doesn't merge)

- [ ] **Step 1: Make the edits.** Keep `bench` advisory, and pin any new action by SHA, as the workflow's others are. zizmor must pass.
- [ ] **Step 2: Run what CI runs**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && (cd evals && uv run pytest -q)`
Expected: all pass.

- [ ] **Step 3: Commit, push, open the PR**

```bash
git add .github/workflows/ci.yml evals/README.md docs/
git commit -m "docs: document the history benchmark"
git fetch origin && git push -u origin feat/history-bench
gh pr create --title "feat(evals): a retrieval benchmark mined from merged PRs across the owner's repositories" --body-file <body>
```

The body has `Closes #180`, its acceptance criteria, the scorecard table, any repository below 20 queries, the AI provenance section (Claude Code, claude-opus-5-5) with `- [x] A human has reviewed this change and is accountable for it`, and `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. No `eval` label: this PR doesn't change codecite's search.
