# Spend Guardrails Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eval runs, and OpenRouter spend above all, stay bounded however often PRs are pushed or labelled. The OpenRouter budget ran out because evals ran on nearly every commit; Claude usage isn't the concern (#297). The ralph queue is also pointed at the next phase.

**Architecture:**
- `evals.yml` counts its own runs in the last 24 hours and refuses to start past a cap.
- OpenRouter runs only on a dispatch that asks for it.
- The ralph router takes the issues listed in `ORDER` from any milestone.

There are no Claude caps: no per-iteration dollar cap and no review cap. Nothing new is installed and no repository setting changes.

**Tech Stack:** GitHub Actions, `gh api`, ralph-orchestrator 2.10.

**Spec:** Issue #297; `AGENTS.md`; `ralph/README.md`.

## Global Constraints

- `EVALS_DAILY_MAX` defaults to `6` runs per rolling 24 hours (a repo variable). One dispatch can bypass it with `ignore_daily_cap`.
- OpenRouter never runs on a `pull_request` event. It runs only on `workflow_dispatch` with `openrouter: true`.
- Evals stay quick tier on PRs, and run once per `eval` label event, as today.
- Workflow changes pass zizmor: actions pinned by SHA, no `${{ }}` of untrusted data inside `run:` (pass it through `env:`), and permissions no wider than today.
- A tripped cap says so in the job's `::error` and summary.
- Don't change GitHub settings or label definitions. Name no client or outside project.

## Review Focus

- **Runs whose eval job was skipped** (any label other than `eval` triggers a skipped run) must not count toward the daily cap.
- **A dispatch with `openrouter: true` on a PR's head** still uses OpenRouter; only `pull_request` events can't.
- **A run still in progress** counts toward the cap. A cancelled run that started its eval job counts too.
- **Paginated run lists** over 100 runs in 24 hours: the count must use `--paginate`.

---

### Task 1: Ralph queue scope

**Files:**
- Modify: `ralph/PROMPT.md` (Queue, step 1)

- [ ] **Step 1:** Replace the milestone scope with "open issues listed in `ORDER`, in any milestone". Set `ORDER = 298 269 299 300`, and keep the "not in `ORDER`, out of scope" rule. #299 and #300 depend on #287, so they wait until #294 merges.
- [ ] **Step 2:** Run `ralph run -c ralph/ralph.yml --dry-run --no-tui -P ralph/PROMPT.md` and check that it parses.
- [ ] **Step 3: Commit** `chore(ralph): queue the next phase by ORDER across milestones`.

### Task 2: Eval daily cap, and OpenRouter on dispatch only

**Files:**
- Modify: `.github/workflows/evals.yml` (the `on.workflow_dispatch.inputs`, a new `budget` job, the `eval` job's `needs`/`if`, and `env.OPENROUTER` at about line 98)

**Interfaces:**
- Produces:
  - a job `budget` with the same `if:` as `eval`, `permissions: actions: read` and an output `ok`;
  - the `eval` job gains `needs: budget` and `&& needs.budget.outputs.ok == 'true'`;
  - a new dispatch input `ignore_daily_cap`: a boolean, default false, described as "Run even if EVALS_DAILY_MAX runs started in the last 24 hours".

- [ ] **Step 1: OpenRouter.** Change `OPENROUTER` to `${{ (github.event_name == 'workflow_dispatch' && inputs.openrouter) && '1' || '0' }}`. Remove `vars.EVALS_OPENROUTER` from the expression and from the header comment. Update the `openrouter` input's description to say this is the only way to switch it on.
- [ ] **Step 2: The `budget` job.**
  - **Inputs:** `env: MAX: ${{ vars.EVALS_DAILY_MAX || '6' }}`, `IGNORE: ${{ github.event_name == 'workflow_dispatch' && inputs.ignore_daily_cap || false }}`, `GH_TOKEN: ${{ github.token }}`, `RUN_ID: ${{ github.run_id }}`.
  - **Count:** `since=$(date -u -d '24 hours ago' +%Y-%m-%dT%H:%M:%SZ)`, then `gh api --paginate "repos/$GITHUB_REPOSITORY/actions/workflows/evals.yml/runs?created=>=$since&per_page=100" --jq "[.workflow_runs[] | select(.id != $RUN_ID and .conclusion != \"skipped\")] | length"`.
  - **Under `MAX`, or `IGNORE` true:** set `ok=true`.
  - **Otherwise:** set `ok=false`, write `::error::` and a summary line, `Eval daily cap reached: <count> runs in the last 24 hours (EVALS_DAILY_MAX=<MAX>). Dispatch with ignore_daily_cap, or raise the repo variable.`, then `exit 1`.
- [ ] **Step 3:** Run `zizmor .github/` and check it is clean.
- [ ] **Step 4: Commit** `ci(evals): cap eval runs per day and run OpenRouter on dispatch only`.

### Task 3: Docs

**Files:**
- Modify: `ralph/README.md` (the scope is `ORDER` across milestones)
- Modify: `AGENTS.md` (step 6: the eval cap and OpenRouter on dispatch)
- Modify: `evals/README.md` (wherever it describes `EVALS_OPENROUTER`)

- [ ] **Step 1:** In `evals/README.md`, add a "Spend limits" section with a table: limit, default, where it's set, how to raise it for one run. Cover `EVALS_DAILY_MAX`, `ignore_daily_cap` and OpenRouter on dispatch. Add "Limits only James can set": the OpenRouter key's credit limit, in the OpenRouter dashboard, which is the only hard dollar cap on eval spend; and the Claude plan's own usage limits.
- [ ] **Step 2:** Update `AGENTS.md` and `evals/README.md` to match: no `EVALS_OPENROUTER`, OpenRouter via a dispatch with `openrouter: true`, and `EVALS_DAILY_MAX`.
- [ ] **Step 3:** Run `grep -rn EVALS_OPENROUTER .` and check that only historical specs or plans under `docs/superpowers/` mention it.
- [ ] **Step 4: Commit** `docs: document the spend limits`.

### Task 4: Gates and PR

- [ ] **Step 1:** Run `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked`, `bash ralph/tests/wait-review.test.sh` and `zizmor .github/`. All must pass.
- [ ] **Step 2:** Open a PR against `main`: `chore: spend guardrails for the ralph loop, CI review and evals`, `Closes #297`. Put each acceptance criterion, the AI provenance section and the ticked box in the body. No `eval` label.
- [ ] **Step 3:** In the PR body, ask James for two things after merging: run `gh variable delete EVALS_OPENROUTER`, and set the OpenRouter key's credit limit.
