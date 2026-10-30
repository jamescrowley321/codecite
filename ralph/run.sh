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

# The history benchmark compares with ripgrep. An interactive shell's `rg` may
# be a function the loop's shells never see, so name a binary when there is one.
if [[ -z "${RG:-}" ]]; then
  RG="$(command -v rg || true)"
  [[ -n "$RG" ]] || { [[ -x "$HOME/.pi/agent/bin/rg" ]] && RG="$HOME/.pi/agent/bin/rg"; }
  [[ -n "$RG" ]] && export RG
fi

if [[ -f "$REPO/.claude/task-state.md" ]]; then
  echo "Resuming: $(grep -E '^(issue|phase):' "$REPO/.claude/task-state.md" | tr '\n' ' ')"
fi

cd "$LOOP"
exec ralph run -c ralph/ralph.yml --idle-timeout 0 "$@"
