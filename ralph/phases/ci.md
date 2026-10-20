# Phase: ci

Watch CI on the commit just pushed and fix what it reports. This covers `ci.yml` only (fmt, clippy, test, deny); the `Claude` review job is the review phase's concern, so never treat it as a CI failure here.

`cd <worktree>`.

1. Run `~/repos/codecite-worktrees/ralph-loop/ralph/wait-ci.sh <branch> $(git rev-parse HEAD)`. It sleeps a minute so GitHub registers the push, then waits for the run for exactly that commit.
2. `PASS` → set `phase: review`, end.
3. `NO_RUN` → set `phase: review`, end.
4. `FAIL` → increment `ci_attempts` in the task state (start at 1). If it is now above 3, set `phase: blocked` with the failing job and log excerpt, end. Otherwise read the failed log the script printed, reproduce the failure locally, fix the cause, run the router's gates and push. Never push an empty commit to re-trigger CI, and never weaken a gate (no `#[allow]`, skipped tests or relaxed `deny.toml`) to get green. Keep `phase: ci`, end. The next iteration watches the new commit.
