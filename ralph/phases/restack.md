# Phase: restack

Keep the stack of open loop PRs based on what is actually upstream. Runs before every new issue.

The ledger `~/repos/codecite/.claude/ralph-stack.md` is a table, bottom of the stack first:
`| issue | branch | pr | parent | parent_sha |`. `parent` is the branch the PR targets, and `parent_sha` is the parent tip this branch was last rebased onto. A missing or empty ledger → nothing to do; end this phase.

1. `git -C ~/repos/codecite fetch --prune origin && git -C ~/repos/codecite pull --ff-only`
2. Drop rows from the bottom whose PR is merged (`gh pr view <pr> --json state`). A PR closed without merging → set `phase: blocked`, ask the owner under `## Blocked` what to do with its children, and end.
3. Walk the remaining rows bottom-up. The new parent is the previous row's branch, or `main` for the bottom row. If `origin/<parent>` differs from `parent_sha`, rebase in a scratch worktree:
   ```
   git -C ~/repos/codecite worktree add --detach ~/repos/codecite-worktrees/ralph-restack origin/<branch>
   cd ~/repos/codecite-worktrees/ralph-restack
   git rebase --onto origin/<parent> <parent_sha>
   ```
   - Conflict → `git rebase --abort`, set `phase: blocked` with the conflicting files, end.
   - Otherwise run the router's gates, then `git push --force-with-lease origin HEAD:<branch>`.
   - If the PR's base isn't `<parent>`, run `gh pr edit <pr> --base <parent>`.
   - Set `parent` and `parent_sha` (the new `origin/<parent>` SHA) in the row.
   - Remove the scratch worktree.
4. Done: return to the router to pick the next issue.
