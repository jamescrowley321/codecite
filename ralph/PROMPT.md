Self-referential implementation loop for codecite. Execute ONE phase of ONE issue per iteration, then end. Fresh context each iteration: persist all state to files.

## Context

Repo `jamescrowley321/codecite`: a Rust workspace (`crates/*`). Read `~/repos/codecite/AGENTS.md` and `~/repos/codecite/docs/design.md` before planning. The owner is learning Rust through this project: PRs explain the idioms they use.

## Paths (absolute only; phases `cd` into worktrees that get deleted)

- Primary checkout: `~/repos/codecite`. It stays clean on `main`, advanced only by `git pull --ff-only`. Loop-owned exceptions: `.claude/task-state.md` and `.claude/ralph-stack.md`.
- Task state: `~/repos/codecite/.claude/task-state.md`
- Stack ledger: `~/repos/codecite/.claude/ralph-stack.md`
- Phase files: `~/repos/codecite-worktrees/ralph-loop/ralph/phases/<phase>.md`
- Issue worktrees: `~/repos/codecite-worktrees/<branch with / replaced by ->`

## Queue

GitHub issues are the only queue and the only status. No status words in files.

1. The current milestone is the lowest-numbered open milestone that has open issues: `gh api repos/jamescrowley321/codecite/milestones --jq 'sort_by(.number)[] | select(.open_issues>0) | .title' | head -1`.
2. A candidate is an open issue in it, without the `blocked` label and not already in the stack ledger. Take the lowest number whose every `Depends on #N` (from its body) is closed or in the ledger.
3. No candidate, or the ledger already holds 4 open PRs → output `<promise>LOOP_COMPLETE</promise>`.

## Stacked PRs

The loop never merges. The owner reviews and squash-merges from the bottom of the stack up. Each new issue branches from the tip of the stack: the last ledger row's branch, or `main` when the ledger is empty. Its PR targets that branch.

## Routing

Read the task state.

- **Does not exist** → read and execute `restack.md`. If it ends without blocking, pick the next issue (Queue), write the task state below, and execute `setup.md`.
- **phase is `blocked`** → do nothing; output `<promise>LOOP_COMPLETE</promise>`. The owner answers `## Blocked` and resets the phase.
- **Any other phase** → read and execute ONLY that phase file.

Phase order: `setup → plan → implement → review → review-fix → pr → ci → complete`

Task state for a new issue:
```
issue: <N>
title: <issue title>
branch: <type>/<short-description>
base_branch: <tip of stack, or main>
worktree: ~/repos/codecite-worktrees/<branch with / replaced by ->
phase: setup
```

## Gates (mirror CI; run before every push)

`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, plus `cargo deny check` once `deny.toml` exists. Never push a red gate.

## Rules

- ONE issue per iteration's work. Never start a second issue.
- Follow `AGENTS.md`: conventional commits, `<type>/<short-description>` branches, no private ID schemes (refer to `#N`), the code wins over the docs, so fix any stale doc in the same PR.
- `git add <paths>`, never `git add .` or `-A`. Commit messages end with `Co-Authored-By: Claude <noreply@anthropic.com>`.
- Never push to `main`, never merge, never force-push a branch that isn't in the ledger, and never change GitHub repo settings (those live in Terraform in oss-admin).
- A spec gap or decision the issue and `docs/design.md` cannot settle → write the question under `## Blocked` in the task state, comment it on the issue, set `phase: blocked`, and end. Never guess at a design decision.
