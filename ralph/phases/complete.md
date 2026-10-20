# Phase: complete

The PR stays open for the owner. Never merge it.

1. Write the task state's `verdict` (PASS or BLOCK) into this PR's `review` column in `~/repos/codecite/.claude/ralph-stack.md`.
2. `git -C ~/repos/codecite worktree remove <worktree>`. The branch stays: the owner may push review fixes to it, and restack rebases it.
3. Delete `~/repos/codecite/.claude/task-state.md`.
4. Output `<promise>TASK COMPLETE</promise>`.
