# Phase: ci

`cd <worktree>`.

1. `gh pr checks <pr> --watch --fail-fast`. If no checks appear within 60 seconds → set `phase: review`, end.
2. All pass → set `phase: review`, end.
3. Failure → increment `ci_attempts` in the task state (start at 1). If it is now above 3, set `phase: blocked` with the failing job and log excerpt, end. Otherwise `gh run view <run id> --log-failed`, fix the cause locally, run the router's gates and push. Never push an empty commit to re-trigger CI. Keep `phase: ci`, end.
