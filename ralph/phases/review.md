# Phase: review

Spawn independent reviewers with the `Agent` tool. Never review the diff in your own context: the point is a fresh, adversarial reader.

`cd <worktree>`.

1. Write the inputs:
   ```
   mkdir -p .claude/review
   git diff origin/<base_branch>...HEAD > .claude/review/diff.patch
   gh issue view <issue> > .claude/review/issue.md
   ```
   Record `git rev-parse HEAD` under `## Pre-Review SHA` in the task state.
2. Choose lenses from `git diff --stat origin/<base_branch>...HEAD`:
   - Rust code → `blind-peer-review:cold_read`, `blind-peer-review:edge_case`, `blind-peer-review:acceptance`
   - Also `.github/`, `deny.toml`, `Cargo.toml` dependency changes or any process/IO/parsing code → add `blind-peer-review:security` and `blind-peer-review:policy`
   - Docs or config only → `blind-peer-review:acceptance`
   Record them under `## Reviewers` in the task state.
3. In ONE message, make one `Agent` call per lens (`subagent_type` = the lens, `model: opus`). Each prompt says:
   - Review the change in `<abs worktree>/.claude/review/diff.patch`.
   - The codebase is at `<abs worktree>`.
   - The issue and its acceptance criteria are in `<abs worktree>/.claude/review/issue.md`.
   - Repo rules are in `AGENTS.md`.
   - Report each finding with a severity (Critical / Important / Minor), `file:line` and a concrete failure scenario.
   - Do not read anything under `.claude/` except `review/`.
   Never pass the plan, the task state or your own notes.
4. Write each reviewer's returned report to `.claude/review/<lens>.md`. A lens that returned nothing → re-spawn it once. If it fails again, set `phase: blocked` and end.
5. Set `phase: review-fix`. End your response.
