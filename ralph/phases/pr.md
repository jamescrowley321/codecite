# Phase: pr

`cd <worktree>`. Run the router's gates one last time.

1. `git push -u origin <branch>`
2. `gh pr create --base <base_branch> --head <branch> --title "<conventional title>"`. The body (plain, specific prose) has these parts:
   - `Closes #<issue>`. If `<base_branch>` isn't `main`: "Stacked on #<parent PR>. Review and merge that one first."
   - **What this adds:** what changed and why, file by file where useful.
   - **Rust notes:** each idiom or design choice the owner should learn from this change, a few sentences each. This is the most important part of the PR for a learning repo.
   - **Review:** what the reviewers found, what was fixed, the rulings, and the deferred minors.
   - **Checks:** the gate commands and their results.
   - End with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
3. Append to `~/repos/codecite/.claude/ralph-stack.md` (create it with its header row if missing): `| <issue> | <branch> | <pr> | <base_branch> | <git rev-parse origin/<base_branch>> |`
4. Record `pr: <number>` in the task state. Set `phase: ci`. End your response.
