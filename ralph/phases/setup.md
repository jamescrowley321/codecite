# Phase: setup

1. `git -C ~/repos/codecite fetch origin`
2. `git -C ~/repos/codecite worktree add <worktree> -b <branch> origin/<base_branch>`
3. `cd <worktree> && git log --oneline -3`. Run `cargo build --workspace` to warm the build cache.
4. Set `phase: plan`. End your response.
