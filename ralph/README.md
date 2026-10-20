# ralph loop

[Ralph Orchestrator](https://github.com/mikeyobrien/ralph-orchestrator) works through the open issues of M2 and M3 in dependency order (`LAST_MILESTONE` in `PROMPT.md`), one phase per iteration, and opens a stack of PRs for review.

- `PROMPT.md`: the router. It reads the queue from GitHub issues and routes each iteration to one phase file.
- `phases/`: `restack → setup → plan → implement → pr → ci → review → review-fix → complete`. Planning and TDD use the superpowers skills.
- Review runs in CI: `.github/workflows/claude.yml` reviews each PR when it opens. The loop waits for that verdict, answers each round in a PR comment, and asks `@claude review this PR` after its fixes, for up to two re-reviews. If CI doesn't review, the loop runs `blind-peer-review:check` locally and posts the result. The plugin chooses the lenses, so tuning the review means changing the plugin pin in `claude.yml`.
- `run.sh`: launches or resumes the loop from the runner worktree `~/repos/codecite-worktrees/ralph-loop`.

## Run

```bash
~/repos/codecite/ralph/run.sh                                  # TUI
~/repos/codecite/ralph/run.sh -a                               # headless
RALPH_REF=origin/<branch> ~/repos/codecite/ralph/run.sh        # loop files from a branch
```

State lives in the primary checkout and is git-ignored:
- `~/repos/codecite/.claude/task-state.md`: the current issue and phase. Re-running `run.sh` resumes from it.
- `~/repos/codecite/.claude/ralph-stack.md`: the open PR stack, bottom first.
- `~/repos/codecite/.claude/ralph-parked.md`: issues set aside with a question, and their local branches.

## Reviewing the stack

The loop never merges.
- Each new issue branches from the previous loop PR, and its PR targets that branch.
- Squash-merge from the bottom up. Before each new issue, the loop rebases the rest of the stack onto what merged.
- You can push review fixes to any branch in the stack; the next restack carries them upward.
- The stack has no depth cap, so an overnight run can leave a long stack. Merge from the bottom; the loop restacks the rest before its next issue.

## Stopping and unblocking

- **Stopping:** stop ralph between iterations. The task state keeps your place.
- **Parked:** a design question parks that issue instead of stopping the run. The loop comments on the issue, keeps its work on a local branch, lists it in `ralph-parked.md`, and moves on; issues that depend on it are skipped. Answer on the issue, delete its row, and the next run picks it up again.
- **Blocked:** the loop stops with `phase: blocked` only when the stack itself is in trouble:
  - a rebase conflict;
  - a loop PR closed without merging;
  - CI still failing after 3 fix attempts.

  It writes the reason under `## Blocked` in the task state. Fix the cause, set the phase back, and re-run `run.sh`.
- **Still BLOCK after review:** a PR the CI review still blocks after two re-reviews does not stop the run. It stays in the stack with a comment for you, and its `review` column in `ralph-stack.md` says BLOCK.
