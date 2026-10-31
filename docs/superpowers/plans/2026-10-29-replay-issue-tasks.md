# Replay Issue Tasks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Agent tasks replayed from real past work: the prompt is a past issue, the project is its repository at the fixing PR's base, the hidden tests are the tests that PR added; run Claude-only, gated on correctness and wall time (#185).

**Architecture:** A miner in the `codecite-evals` package (`replay.py`, beside `history.py`) lists merged PRs that close an issue and add tests, and writes each chosen one as a task directory `evals/tasks/replay-<repo>-<pr>/`: `task.json` with `"kind": "replay"`, the repository, the base commit, the language and the hidden test paths; `prompt.md` (the issue); `hidden/` (the PR's version of its test files); `reference/fix.patch` (the PR's non-test diff); and `check.sh`, which proves the hidden tests fail at the base and pass with the fix. `prepare.sh` stages the repository itself as both the project and the `eval` corpus, and the Dockerfile installs the language's toolchain and warms its dependencies so scoring runs offline. `task.py` scores a replay task by overlaying the hidden files on the agent's tree and running the task's test command. `report.py` gives a replay task its own gate, and `panel.sh` runs it on the Claude-only panel.

**Tech Stack:** Python 3 with uv (`evals/`), Inspect AI, Docker, bash, `gh`, `git`; Go, Rust and Node toolchains inside the sandbox image only.

**Spec:** `docs/superpowers/specs/2026-10-29-general-purpose-design.md` (Goal 6, "The gate for agent evals", Order step 6).

## Global Constraints

- Repositories are the history benchmark's (`evals/history/`). blind-peer-review has no merged PR that closes an issue and changes a test file, so it gets no task; the other four each get at least one.
- A candidate PR: merged, by a person (`history.is_bot`), closes an issue (`closingIssuesReferences`), changes at least one non-test file and adds or modifies at least one test file kept apart from source code:
  - Python: a path under a `tests/` or `test/` directory, or named `test_*.py` / `*_test.py`, or `conftest.py` beside them.
  - Go: `*_test.go`. A Go test that needs `TF_ACC` or network (terraform-provider-descope's acceptance tests, `resource.Test(`/`resource.ParallelTest(`) is no hidden test.
  - Rust: `crates/<crate>/tests/*.rs`. A PR whose only tests are inline `#[cfg(test)]` modules in a source file it changes is no candidate: the overlay would overwrite the agent's fix.
  - JS/TS: `*.test.{js,ts,jsx,tsx}`, `*.spec.*`, or under `__tests__/`.
- Hidden tests are the PR's version (at its merge commit) of those files, overlaid on the agent's tree after the agent exits. The test command runs only those files.
- A task is kept only when `check.sh` shows its hidden tests **fail** at the base and **pass** with `reference/fix.patch` applied. Otherwise the miner skips it.
- The project is `git archive` of the base commit, committed into a fresh one-commit repository, so no later history reaches the agent. The `eval` corpus indexes that same tree.
- The prompt is the issue's title and body as it was asked, with a header saying the repository is at `/work/project` and to make the change the issue asks for. No PR text, no test names.
- Panel and gate for `"kind": "replay"` tasks (`.ralph` decisions, flagged in the PR body for the owner): Claude Code only, on claude-haiku-5-5, claude-sonnet-5-5 and claude-opus-5-5 at both tiers; OpenRouter is skipped for a replay task even when switched on. Pass rate loses when its 95% interval's upper bound is below 0; wall time loses when its lower bound is above +25%; tokens get an interval and never a loss. Other tasks keep their panels and the design.md Success criteria gate.
- Calibrate on the baseline arm alone, on the quick tier, in CI (`gh workflow run evals.yml -f task=<id> -f arms=baseline -f tier=quick --ref <branch>`). Never run the codecite arm while calibrating, never run evals locally (the owner's budget rule). In band: 40–80% pooled, no model passes every run. A task out of band is replaced by the next candidate of its repository, not adjusted, since adjusting a real issue's prompt stops it being a replay.
- Freezing: `frozen_at` is set to the commit of the task's first A/B run, in the PR that adds the task to the panel (`evals/README.md`, Calibrating). This PR is that PR: once calibrated, each kept task's `frozen_at` names the pushed commit its first quick A/B run is dispatched at.
- Before every push: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo deny check`, and `(cd evals && uv run --locked pytest)`.

## Review Focus

1. A hidden test file the PR added must not exist in the project the agent sees; one it modified must be the base version there. Tested in Task 2.
2. The hidden tests must fail at the base: a task whose tests pass without the fix measures nothing. Proven by `check.sh` in Task 3.
3. No later commit or the PR's diff reaches the sandbox before scoring: the project is a one-commit repo, `reference/` and `hidden/` are copied in only after the agent exits. Tested in Task 5.
4. Scoring runs offline: dependencies are warmed in the image, test commands set the offline switches (`UV_OFFLINE=1`, `GOFLAGS=-mod=mod` with `GOPROXY=off`, `CARGO_NET_OFFLINE=true`, `npm --offline`). Tested in Task 4 by building and running `check.sh` in the image with `--network none`.
5. A replay task's verdict never uses the 5% margin, and other tasks' verdicts don't change. Tested in Task 6.

---

### Task 1: Find candidate PRs

**Files:**
- Create: `evals/src/codecite_evals/replay.py`
- Create: `evals/tests/test_replay.py`
- Modify: `evals/pyproject.toml` (script `codecite-replay-mine = "codecite_evals.replay:main"`)

**Interfaces:**
- `is_test_path(path: str) -> bool` and `language(paths: list[str]) -> str` (`python`, `go`, `rust`, `node`, by the most common source extension among the PR's non-test files).
- `Candidate(repo, pr, issue, title, body, base, merge, tests: list[str], sources: list[str], language)`.
- `candidates(prs: list[dict], repo: str) -> list[Candidate]`: pure, over `gh pr list --json number,author,baseRefOid,mergeCommit,closingIssuesReferences,files` records, newest first.
- `acceptance_only(text: str) -> bool`: a Go test file whose every `func Test` calls `resource.Test(`/`resource.ParallelTest(`.

- [ ] **Step 1: Write failing tests**: test paths per language (including `conftest.py`, `crates/x/tests/a.rs` yes, `crates/x/src/a.rs` no, `src/a.test.ts` yes); a bot's PR, a PR closing no issue, a PR with only test files and a PR with only inline Rust tests are skipped; candidate order is newest first; `acceptance_only` on a two-function sample.
- [ ] **Step 2: Run them to see them fail** (`cd evals && uv run --locked pytest tests/test_replay.py -q`).
- [ ] **Step 3: Implement** reusing `history.is_bot` and `history.gh_json`; nothing that shells out is in the pure functions.
- [ ] **Step 4: Run the tests to see them pass**; commit `feat(evals): find past PRs that close an issue and add tests (#185)`.

> Learn: a `@dataclass(frozen=True)` gives value equality and hashing for free, the Python cousin of `#[derive(PartialEq, Eq, Hash)]`.

### Task 2: Write a candidate as a task directory

**Files:**
- Modify: `evals/src/codecite_evals/replay.py`, `evals/tests/test_replay.py`

**Interfaces:**
- `write_task(c: Candidate, clone: Path, out: Path) -> Path` writes `out/replay-<repo>-<pr>/` with `task.json` (`id`, `kind: "replay"`, `gate: "replay"`, `repo`, `url`, `base`, `merge`, `pr`, `issue`, `language`, `hidden` (repo paths), `test` (the command, a list, with `{hidden}` standing for the hidden paths), `setup`, `epochs`), `prompt.md`, `hidden/<repo path>` (from `git show <merge>:<path>`), `reference/fix.patch` (`git diff <base> <merge> -- <sources>`), and `check.sh` copied from a template.
- Test commands by language: python `uv run --with pytest==9.1.1 pytest -q -p no:cacheprovider {hidden}`; go `go test` over the packages holding the hidden files with `-run` unset; rust `cargo test --locked --offline` with `--test <stem>` per hidden file; node the repo's `npm test --` with the files.
- `main()`: `codecite-replay-mine OWNER/REPO --clone DIR [--max N] [--out evals/tasks]` lists the candidates and writes the first `N` (default 3) that `check.sh` accepts, printing why each other was skipped.

- [ ] **Step 1: Write failing tests** against a throwaway git repo made in `tmp_path`: an added test file appears in `hidden/` and not at the base; a modified one's `hidden/` copy is the merge version; `fix.patch` holds no test file; the prompt holds the issue's title and body and no PR text; `task.json` round-trips.
- [ ] **Step 2: Run them to see them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests to see them pass**; commit `feat(evals): write a past PR as a replay task (#185)`.

### Task 3: Prove a replay task with `check.sh`

**Files:**
- Create: `evals/tasks/_replay/check.sh` (the template `write_task` copies)
- Modify: `evals/src/codecite_evals/frozen.py` if it walks `tasks/*` and would treat `_replay` as a task (skip names starting `_`)

`check.sh` needs the repository (`REPLAY_REPO`, a clone holding both commits) and the language's toolchain. It archives the base into a temp dir, runs `setup`, overlays `hidden/`, runs `test` and requires failure; applies `reference/fix.patch`, runs `test` again and requires success. Exit 0 only for fail-then-pass.

- [ ] **Step 1:** Write the template; test it from `test_replay.py` on the throwaway Python repo (a fix that makes a test pass, and a "fix" that leaves it failing).
- [ ] **Step 2:** Run the tests; commit `feat(evals): prove a replay task's hidden tests need its fix (#185)`.

### Task 4: Stage and build a replay task's sandbox

**Files:**
- Modify: `evals/inspect/prepare.sh`, `evals/inspect/Dockerfile`
- Modify: `.github/workflows/evals.yml` (fetch the task's repository at `base` for a replay task, identity-model otherwise), `.github/workflows/ci.yml` (`eval-task` runs one replay task's `check.sh` per language it can host cheaply: python and go)

- `prepare.sh <id>` for a replay task: `git archive <base>` of `REPLAY_REPO` (default `~/repos/<repo>`) into `build/project`, an empty `build/identity-model`, `LANGUAGE` written to `build/`, and the corpus manifest `build/corpus.toml` naming the project.
- Dockerfile: a `LANGUAGE` build arg; `go` copies the Go toolchain from `golang:<pinned>`, `rust` from `rust:<pinned>-slim`; the project is committed as a one-commit repo; its setup runs and its test command is warmed (`go mod download && go test -count=1 -run '^$' ./...`, `cargo fetch && cargo test --no-run`, `npm ci`, `uv sync`); the `eval` corpus indexes `/work/project` when a replay task's manifest is staged. Pin every image by digest, as the existing ones are.
- [ ] **Step 1:** Stage and build one python and one go task locally; run `check.sh`'s fail-then-pass inside the image with `--network none`.
- [ ] **Step 2:** Commit `feat(evals): build a replay task's sandbox from its repository (#185)`.

> Learn: a multi-stage `COPY --from=<image>` takes one toolchain out of another image without inheriting its OS.

### Task 5: Run and score a replay task in `task.py`

**Files:**
- Modify: `evals/src/codecite_evals/task.py`, `evals/src/codecite_evals/config.py`, `evals/src/codecite_evals/_registry.py`
- Modify: `evals/tests/test_task.py`

- `codecite_note` names the corpus from `task.json` (`the <repo> source at /work/project` for a replay task) instead of always identity-model.
- `hidden_tests_and_checks` for `kind: "replay"`: write each `hidden/<path>` to `/work/project/<path>` (overwriting the agent's copy), run `test` with `{hidden}` expanded, cwd `/work/project`, the language's offline env, timeout 300; no `checks.py`. Everything else (stopped, sandbox errors, why_stopped) unchanged.
- `codecite_evals/ab -T task_id=replay-...` runs it; no per-task registration needed.
- [ ] **Step 1: Write failing tests** with the existing fake sandbox in `test_task.py`: hidden files land at their repo paths only at scoring, the test command gets them, the note names the repo.
- [ ] **Step 2:** Implement; tests pass; commit `feat(evals): score a replay task with its PR's tests (#185)`.

### Task 6: The replay gate and the Claude-only panel

**Files:**
- Modify: `evals/src/codecite_evals/report.py`, `evals/tests/test_report.py`, `evals/inspect/panel.sh`

- `report.py`: `GATES = {"default": {"pass": 0.05, "tokens": 0.05, "time": 0.05}, "replay": {"pass": 0.0, "tokens": None, "time": 0.25}}`; the gate comes from the task's `task.json` (`gate`, default `default`); `None` means reported, never a loss. Intervals keep their `*` at the task's margins.
- `panel.sh`: for a task whose `task.json` has `kind: "replay"`, `CLAUDE_PANEL=(claude-haiku-5-5 claude-sonnet-5-5 claude-opus-5-5)` at both tiers and `OPENROUTER_MODELS=()`.
- [ ] **Step 1: Write failing tests**: a replay task at −3 points pass with the interval's top below 0 loses; at +20% time doesn't; tokens +40% doesn't; the same runs on a default task give today's verdicts.
- [ ] **Step 2:** Implement; tests pass; commit `feat(evals): judge replay tasks on correctness and a 25% time rise, Claude-only (#185)`.

### Task 7: Mine the tasks

- [ ] For each of identity-model, identity-stack, terraform-provider-descope and codecite: `uv run --locked codecite-replay-mine jamescrowley321/<repo> --clone ~/repos/<repo or auth/repo> --max 3`. Keep every task `check.sh` accepts. Copy `evals/tasks/_replay/check.sh` checks into CI only for python and go.
- [ ] Commit `feat(evals): replay tasks from identity-model, identity-stack, terraform-provider-descope and codecite (#185)`.

### Task 8: Calibrate on the baseline arm

- [ ] Push the branch; for each task dispatch `gh workflow run evals.yml --ref feat/replay-issue-tasks -f task=<id> -f tier=quick -f arms=baseline`; run them in the background and read each calibration table from its job summary.
- [ ] Keep, per repository, the first task in band (40–80% pooled, no model at 100%). Delete the other tasks of that repository. A repository with none in band keeps its closest task below the band, frozen as hard by design, the way `backchannel-logout` and `client-auth-uri` were; say so in design.md.
- [ ] Record each kept task's calibration (per-model pass rate, pooled rate) in `docs/design.md`; commit `feat(evals): calibrate the replay tasks on the baseline arm (#185)` and push.
- [ ] Freeze: set each kept task's `frozen_at` to that pushed commit, commit `feat(evals): freeze the replay tasks (#185)`, push, and dispatch the first quick A/B run of each at the frozen commit's branch head (`-f arms=ab`). `codecite-eval-frozen` must pass. Report the scorecards on the PR as directional, never a pass.

### Task 9: Docs

**Files:**
- Modify: `evals/README.md` (Tasks: replay tasks, how they're mined and proved; Two tiers: the Claude-only replay panel; the scorecard: the replay gate), `docs/design.md` (Evaluation plan: Agent benchmark replay tasks, the replay gate in Success criteria)
- [ ] Commit `docs: describe replay tasks, their panel and gate (#185)`.
