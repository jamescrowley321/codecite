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
| Autonomous loop over the open issues | `ralph/` (see `ralph/README.md`) |

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
  stays on a clean `main`.
- Conventional commits. Every change lands through a squash-merged PR; the PR
  title becomes the commit.
- `main` is protected: a PR merges only when it is up to date with `main` and
  the CI and security gates pass (`fmt`, `clippy`, `test`, `deny`, `zizmor`,
  `gitleaks`, `dependency-review`, CodeQL, OSV-Scanner). Renaming a gate's job
  means renaming its required check in oss-admin in step.

## Repository settings

GitHub settings for this repo are managed in Terraform in the private
`oss-admin` repo (`terraform/codecite`). Change them there, not in the GitHub UI.
