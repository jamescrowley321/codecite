#!/usr/bin/env bash
# Launch (or resume) the codecite ralph loop from its dedicated runner worktree.
#
#   ralph/run.sh [extra `ralph run` args]
#   RALPH_REF=origin/<branch> ralph/run.sh   # run the loop files from a branch instead of main
#
# Syncs the primary checkout, points the runner worktree at $RALPH_REF (detached),
# copies the router in as PROMPT.md, and runs ralph. Loop state lives in the primary
# checkout by absolute path, so an interrupted issue resumes on the next launch.
set -euo pipefail

REPO="$HOME/repos/codecite"
LOOP="$HOME/repos/codecite-worktrees/ralph-loop"
REF="${RALPH_REF:-origin/main}"

git -C "$REPO" fetch --quiet --prune origin
git -C "$REPO" pull --quiet --ff-only

if git -C "$REPO" worktree list --porcelain | grep -qx "worktree $LOOP"; then
  git -C "$LOOP" checkout --quiet --detach "$REF"
  git -C "$LOOP" reset --quiet --hard "$REF"
else
  git -C "$REPO" worktree prune
  git -C "$REPO" worktree add --quiet --detach "$LOOP" "$REF"
fi

cp "$LOOP/ralph/PROMPT.md" "$LOOP/PROMPT.md"

if [[ -f "$REPO/.claude/task-state.md" ]]; then
  echo "Resuming: $(grep -E '^(issue|phase):' "$REPO/.claude/task-state.md" | tr '\n' ' ')"
fi

cd "$LOOP"
exec ralph run -c ralph/ralph.yml --idle-timeout 0 "$@"
