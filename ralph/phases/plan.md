# Phase: plan

`cd <worktree>`.

1. Read the issue: `gh issue view <issue> --comments`. Read `AGENTS.md`, the parts of `docs/design.md` the issue points to, and every file the change will touch.
2. Invoke the `superpowers:writing-plans` skill for this issue. Save the plan at `docs/superpowers/plans/<today YYYY-MM-DD>-<topic>.md`. Include the "Learn:" points from the issue as short notes at the steps where each idiom first appears. Skip the skill's execution handoff: the loop's `implement` phase executes the plan.
3. Commit: `docs: add the plan for <topic> (#<issue>)`.
4. Write to the task state: `plan: <path>`, `## Anchor` with `git rev-parse HEAD`, and `## Progress` listing every plan task as `- Task N`.
5. Set `phase: implement`. End your response.
