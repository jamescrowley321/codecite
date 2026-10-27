# Phase: ci

Watch CI on the commit just pushed and fix what it reports. This covers every workflow on the commit (CI, Security, CodeQL, OSV-Scanner, Dependency Review), because main's required checks span them all. The review workflow (Claude Code) is the review phase's concern, so never treat it as a CI failure here.

`cd <worktree>`.

1. Run `~/repos/codecite-worktrees/ralph-loop/ralph/wait-ci.sh <branch> $(git rev-parse HEAD)`. It sleeps a minute so GitHub registers the push, then waits up to 8 minutes for the runs on exactly that commit. Run it in the FOREGROUND with the Bash tool's `timeout: 600000`. Never `run_in_background`: a background command dies when this iteration ends, and its result is lost.
2. `PASS` → set `phase: review`, end.
3. `NO_RUN` → set `phase: review`, end.
4. `WAITING` (CodeQL is slow) → increment `ci_waits` in the task state (start at 1). If it is now above 4, treat it as `FAIL` with the workflows still running as the reason. Otherwise run the script again with the same arguments, up to two calls in this iteration in all; if the second also says `WAITING`, keep `phase: ci` and end.
5. `FAIL` → increment `ci_attempts` in the task state (start at 1). If it is now above 3, set `phase: blocked` with the failing job and log excerpt, end. Otherwise read the failed log the script printed, reproduce the failure locally, fix the cause, run the router's gates and push. A security finding (a vulnerable dependency, a leaked-looking value, a CodeQL or zizmor alert) is fixed at its cause: bump or replace the dependency, or narrow an allowlist to that exact value and file, never to a rule or path wholesale. Reset `ci_waits`. Never push an empty commit to re-trigger CI, and never weaken a gate (no `#[allow]`, skipped tests or relaxed `deny.toml`, scanner config or workflow) to get green. Keep `phase: ci`, end. The next iteration watches the new commit.
