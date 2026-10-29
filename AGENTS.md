# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

codecite: a single Rust binary that indexes code and specs into one SQLite file
per corpus and answers AI agents with cited results over a CLI and an MCP
server. Read `docs/design.md` before any non-trivial change.

## Where things live

| What | Where |
| --- | --- |
| Why and what: goals, architecture, decisions | `docs/design.md` — living, edited in place |
| How to use it: manifests, commands, citation forms, MCP | `docs/usage.md` — living, edited in place |
| Work and status | GitHub milestones and issues — the only place status lives |
| Design for one open question | `docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md` |
| Steps for one issue | `docs/superpowers/plans/YYYY-MM-DD-<topic>.md` |
| A/B agent eval and retrieval benchmark | `evals/` (see `evals/README.md`) |
| Experiments and spikes, with their scripts and results | `experiments/<topic>/` — kept for re-running or scaling up |
| Autonomous loop over the open issues | `ralph/` (see `ralph/README.md`) |
| Working issues in Claude Code cloud sessions | `docs/cloud-sessions.md` |

Specs and plans are committed with the PR that implements them and frozen once
it merges. Do not go back and update them; update `docs/design.md` instead.

## Planning

Pick the flow by size, using the superpowers skills:

- **Small fix:** issue → branch → PR.
- **Normal work:** issue → `superpowers:writing-plans` → execute.
- **Open design question:** `superpowers:brainstorming` → spec → plan → execute.

Rules:

- No status words (pending, done, blocked) in any doc. Status is the issue's
  state and its milestone.
- No private ID schemes (FR-1, T12, EPIC-3). Refer to issues by number (`#12`)
  and milestones by name.
- The code wins over the docs. When they disagree, fix the doc in the same PR.
- No vendored methodology. Skills come from installed plugins; this repo holds
  only what they produce.

## Git

- Work on a `<type>/<short-description>` branch in its own worktree under
  `~/repos/codecite-worktrees/`. The primary checkout at `~/repos/codecite`
  stays on a clean `main`. A Claude Code cloud session works on its own
  branch in its own checkout instead.
- Never force-push. James and other sessions push to the same branches; fetch
  before you push.
- Conventional commits. Every change lands through a squash-merged PR; the PR
  title becomes the commit.
- `main` is protected: a PR merges only when it is up to date with `main` and
  the CI and security gates pass (`fmt`, `clippy`, `test`, `deny`, `zizmor`,
  `gitleaks`, `dependency-review`, CodeQL, OSV-Scanner). Renaming a gate's job
  means renaming its required check in oss-admin in step.

## Rules for every artifact

- Name no client or outside project in commits, PRs, issues or docs. The
  public corpora codecite indexes are fine; anything else is "another project".
- Every PR body has an AI provenance section naming the harness and the model,
  and the box `- [x] A human has reviewed this change and is accountable for
  it`, ticked. James has delegated the tick for agent PRs; the Policy lens
  blocks without it.

## Working an issue end to end

This is how an agent, local or in a Claude Code cloud session, takes one
issue from open to a reviewed PR. Ralph (`ralph/`) runs the same steps in a
loop.

1. **Pick.** Take the issue James names, or else the next open issue in the
   earliest open milestone. Read it and `docs/design.md`.
2. **Plan.** Follow Planning above. If the superpowers skills aren't
   installed, as in a cloud session, write the plan by hand in
   `docs/superpowers/plans/` and commit it with the PR.
3. **Build test-first.** Before every push, run what CI runs:
   `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D
   warnings` and `cargo test --workspace --locked`.
4. **Open the PR against `main`**, never stacked. Use a conventional title and put `Closes #<n>`, the issue's
   acceptance criteria and the provenance section in the body.
5. **Read the feedback.** CI and the blind peer review come back on the PR.
   `claude.yml` runs the review when the PR opens and posts one comment,
   "Blind peer review — PR #<n>", with a table per lens and a `Verdict:` line
   (PASS or BLOCK). Fix every MUST FIX at its cause, test-first. Lenses see
   only diff hunks, so before fixing a "missing" line read the whole file; for
   a false positive, reply on the PR saying why. Fix every SHOULD FIX and
   actionable NITPICK (an Acceptance Criteria NITPICK that only restates a
   pass needs nothing). After pushing fixes, comment `@claude review this PR`
   to get a fresh review of the new head. After three review rounds, stop and
   summarise what's left in a PR comment.
6. **Ask for an eval.** If the PR changes indexing, search, ranking or the
   CLI's output, add the `eval` label. The eval workflow runs the quick A/B
   eval once, on the Claude panel, and keeps its scorecard in one PR comment;
   to rerun it on a new head, take the label off and put it back. OpenRouter
   bills per token and is off unless James switches it on. The quick tier is directional: a quick loss means run the full
   eval (the workflow dispatched with `tier: full`), so say so on the PR.
   Never call it a pass.
7. **Stop there.** James merges. Don't merge your own PR.

## Repository settings

GitHub settings for this repo are managed in Terraform in the private
`oss-admin` repo (`terraform/codecite`). Change them there, not in the GitHub UI.
