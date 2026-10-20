# Phase: implement

`cd <worktree>`. Execute the plan with `superpowers:test-driven-development`: watch each test fail before writing the code that passes it.

1. Read the plan and `## Progress` in the task state. Drift check: if `## Anchor` is no longer an ancestor of `HEAD`, re-read the changed files before continuing.
2. Take the plan's tasks in order, starting at the first one without `done` in `## Progress`. For each task, follow its steps exactly, compare every `Expected:` line with real output, and commit as the plan says. Then mark it `- Task N: done (<sha>)`.
3. A plan step that can't work as written → make the smallest change that satisfies the issue, and record it as `- Ruling: <what and why>` under `## Progress`.
4. Stop starting new tasks once ~20 minutes have passed. The next iteration resumes from `## Progress`.
5. When every task is `done`, run the router's gates. They must pass. If this change made any doc stale (`README.md`, `AGENTS.md`, `docs/design.md`), fix it in this PR.
6. Only when all tasks are done and the gates pass: set `phase: pr`. End your response.
