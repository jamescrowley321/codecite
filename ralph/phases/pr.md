# Phase: pr

`cd <worktree>`. Run the router's gates one last time.

1. `git push -u origin <branch>`. Record `review_round: 1` in the task state.
2. `gh pr create --base <base_branch> --head <branch> --title "<conventional title>"`, plus `--label eval` when the change touches indexing, search, ranking or the CLI's output (AGENTS.md step 6). The body (plain, specific prose) has these parts:
   - `Closes #<issue>`. If `<base_branch>` isn't `main`: "Stacked on #<parent PR>. Review and merge that one first."
   - **What this adds:** what changed and why, file by file where useful.
   - **Rust notes:** each idiom or design choice the owner should learn from this change, a few sentences each. This is the most important part of the PR for a learning repo.
   - **Review:** "The blind peer review runs in CI (claude.yml) and posts its verdict as a PR comment; the loop answers each round in a PR comment."
   - **Checks:** the gate commands and their results.
   - **History benchmark:** for a change it judges (see the router), each repository's pooled `codecite` row next to the base branch's.
   - **AI provenance:** a section with `- Harness: Claude Code (ralph loop)`, `- Model: <the model running this session>` and the ticked box `- [x] A human has reviewed this change and is accountable for it`. The owner has delegated the tick for loop PRs; the CI review's policy lens blocks a PR without it.
   - End with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
3. Append to `~/repos/codecite/.claude/ralph-stack.md` (create it with its header row if missing): `| <issue> | <branch> | <pr> | <base_branch> | <git rev-parse origin/<base_branch>> | |` (the last column, `review`, is filled in by `complete`)
4. Record `pr: <number>` in the task state. Set `phase: ci`. End your response.
