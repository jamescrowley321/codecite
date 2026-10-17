# ralph loop

[Ralph Orchestrator](https://github.com/mikeyobrien/ralph-orchestrator) works through the open issues of the current milestone, one phase per iteration, and opens a stack of PRs for review.

- `PROMPT.md`: the router. It reads the queue from GitHub issues and routes each iteration to one phase file.
- `phases/`: `restack → setup → plan → implement → review → review-fix → pr → ci → complete`. Planning and TDD use the superpowers skills; review uses the `blind-peer-review` agents.
- `run.sh`: launches or resumes the loop from the runner worktree `~/repos/codecite-worktrees/ralph-loop`.

## Run

```bash
~/repos/codecite/ralph/run.sh                                  # TUI
~/repos/codecite/ralph/run.sh --no-tui -a                      # headless
RALPH_REF=origin/<branch> ~/repos/codecite/ralph/run.sh        # loop files from a branch
```

State lives in the primary checkout and is git-ignored:
- `~/repos/codecite/.claude/task-state.md`: the current issue and phase. Re-running `run.sh` resumes from it.
- `~/repos/codecite/.claude/ralph-stack.md`: the open PR stack, bottom first.

## Reviewing the stack

The loop never merges.
- Each new issue branches from the previous loop PR, and its PR targets that branch.
- Squash-merge from the bottom up. Before each new issue, the loop rebases the rest of the stack onto what merged.
- You can push review fixes to any branch in the stack; the next restack carries them upward.
- The stack holds at most four open PRs. At that point the loop stops until you merge.

## Stopping and unblocking

- **Stopping:** stop ralph between iterations. The task state keeps your place.
- **Blocked:** the loop stops with `phase: blocked` when it hits:
  - a design question it can't settle;
  - a rebase conflict;
  - a reviewer that won't run.

  It writes the reason under `## Blocked` in the task state and, for design questions, on the issue. Answer the question, set the phase back, and re-run `run.sh`.
