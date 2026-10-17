# Phase: review-fix

`cd <worktree>`. Read every `.claude/review/<lens>.md`.

1. Re-grade each finding by what a person using codecite would get if it shipped, not by the reviewer's label.
   - **Critical / Important:** fix now.
   - **Minor:** list under `## Deferred` in the task state. They go into the PR body; don't fix them.
   - **Disagree:** record `- Ruling: <finding> — <why the code stands>` under `## Review Summary`.
2. Fix each finding test-first: write the test that reproduces it, watch it fail, fix, watch it pass. Then run the router's gates. Commit with `fix: address review findings`.
3. If anything was fixed, re-run only the lenses that raised Critical/Important findings, on the fix delta alone (`git diff <Pre-Review SHA>...HEAD`), telling them which findings were addressed. At most 2 re-review rounds. Critical findings still open after that → set `phase: blocked` with the findings, end.
4. Write `## Review Summary` (counts, what was fixed, rulings). Set `phase: pr`. End your response.
