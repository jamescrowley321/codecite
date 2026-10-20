# Phase: pr

`cd <worktree>`. Run the router's gates one last time.

1. `git push -u origin <branch>`. Record `review_requested_at: <date -u +%Y-%m-%dT%H:%M:%SZ>` and `review_round: 1` in the task state just before the next step: opening the PR starts the CI review.
2. `gh pr create --base <base_branch> --head <branch> --title "<conventional title>"`. The body (plain, specific prose) has these parts:
   - `Closes #<issue>`. If `<base_branch>` isn't `main`: "Stacked on #<parent PR>. Review and merge that one first."
   - **What this adds:** what changed and why, file by file where useful.
   - **Rust notes:** each idiom or design choice the owner should learn from this change, a few sentences each. This is the most important part of the PR for a learning repo.
   - **Review:** "The blind peer review runs in CI and posts its verdict on this PR; the loop answers each round in a PR comment."
   - **Checks:** the gate commands and their results.
   - End with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
3. Append to `~/repos/codecite/.claude/ralph-stack.md` (create it with its header row if missing): `| <issue> | <branch> | <pr> | <base_branch> | <git rev-parse origin/<base_branch>> | |` (the last column, `review`, is filled in by `complete`)
4. Record `pr: <number>` in the task state. Set `phase: ci`. End your response.
