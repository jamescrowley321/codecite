# Phase: ci

`cd <worktree>`.

1. `gh pr checks <pr> --watch --fail-fast`. If no checks appear within 60 seconds (no CI yet, or no workflow triggers on this base) → set `phase: complete`, end.
2. All pass → set `phase: complete`, end.
3. Failure → `gh run view <run id> --log-failed`, then fix the cause locally, run the router's gates and push. Never push an empty commit to re-trigger CI. Keep `phase: ci`, end.
