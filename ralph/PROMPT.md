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

1. Scope: open issues in milestones numbered up to `LAST_MILESTONE = 3` (M3 MCP server), across milestones, in dependency order. List them with `gh issue list --state open --limit 200 --json number,title,body,milestone,labels`.
2. A candidate is an in-scope issue without the `blocked` label, not in the stack ledger and not in the parked list. Take the lowest number whose every `Depends on #N` (from its body) is closed or in the ledger. An issue that depends on a parked issue is never a candidate.
3. No candidate → output `<promise>LOOP_COMPLETE</promise>`. There is no cap on the stack's depth.

## Stacked PRs

The loop never merges. The owner reviews and squash-merges from the bottom of the stack up. Each new issue branches from the tip of the stack: the last ledger row's branch, or `main` when the ledger is empty. Its PR targets that branch.

## Parking

The parked list `~/repos/codecite/.claude/ralph-parked.md` holds issues the loop set aside so the run can continue: `| issue | branch | reason |`. To park the current issue:

1. Comment the question or findings on the issue (`gh issue comment`), ending with "Parked by the ralph loop; the rest of the queue continues."
2. Commit any work in the worktree, then `git -C ~/repos/codecite worktree remove --force <worktree>`. The branch stays local and unpushed.
3. Append the row to the parked list (create it with its header row if missing). Delete the task state. End your response.

Park, don't block, for a design question the issue and `docs/design.md` cannot settle. Never park an issue whose PR is already open: by then it is in the ledger and its children build on it.

## Routing

Read the task state.

- **Does not exist** → read and execute `restack.md`. If it ends without blocking, pick the next issue (Queue), write the task state below, and execute `setup.md`.
- **phase is `blocked`** → do nothing; output `<promise>LOOP_COMPLETE</promise>`. The owner answers `## Blocked` and resets the phase.
- **Any other phase** → read and execute ONLY that phase file.

Phase order: `setup → plan → implement → pr → ci → review → review-fix → complete`. review-fix loops back to `ci` for each re-review.

Task state for a new issue:
```
issue: <N>
title: <issue title>
branch: <type>/<short-description>
base_branch: <tip of stack, or main>
worktree: ~/repos/codecite-worktrees/<branch with / replaced by ->
phase: setup
```

## Review

The blind peer review runs in CI (`.github/workflows/claude.yml`): opening a PR starts it, and the comment `@claude review this PR` re-runs it. The loop waits for its verdict on the PR, answers every round in a PR comment, and falls back to running the `blind-peer-review:check` skill locally only when CI does not review. The plugin decides which lenses run: never name, add or drop lenses.

## Gates (mirror CI; run before every push)

`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, plus `cargo deny check` once `deny.toml` exists. Never push a red gate.

## Rules

- ONE issue per iteration's work. Never start a second issue.
- Follow `AGENTS.md`: conventional commits, `<type>/<short-description>` branches, no private ID schemes (refer to `#N`), the code wins over the docs, so fix any stale doc in the same PR.
- `git add <paths>`, never `git add .` or `-A`. Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Never push to `main`, never merge, never force-push a branch that isn't in the ledger, never create or edit labels, and never change GitHub repo settings (those live in Terraform in oss-admin).
- A spec gap or decision the issue and `docs/design.md` cannot settle → park the issue (see Parking). Never guess at a design decision.
- Block (`phase: blocked`, the reason under `## Blocked`, end) only when the stack itself is in trouble: a restack conflict, a loop PR closed without merging, or CI that still fails after 3 fix attempts. A PR still at BLOCK after two re-reviews stays open in the stack with a comment for the owner; the run continues.
