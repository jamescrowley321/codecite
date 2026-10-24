#!/usr/bin/env bash
# Stage the sandbox image's build context in build/: the task's starting
# project, identity-model at the task's pinned commit, and a codecite binary.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
TASK="$REPO_ROOT/evals/tasks/fastapi-middleware"
IDENTITY_MODEL_REPO="${IDENTITY_MODEL_REPO:-$HOME/repos/auth/identity-model}"
CODECITE="${CODECITE:-$REPO_ROOT/target/release/codecite}"
COMMIT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["corpus"]["commit"])' "$TASK/task.json")"

rm -rf "$HERE/build"; mkdir -p "$HERE/build/identity-model"
git -C "$IDENTITY_MODEL_REPO" archive "$COMMIT" | tar -x -C "$HERE/build/identity-model"
cp -r "$TASK/project" "$HERE/build/project"
cp "$CODECITE" "$HERE/build/codecite"
cp "$HERE/Dockerfile" "$HERE/pi-models.json" "$HERE/build/"
echo "$COMMIT" > "$HERE/build/COMMIT"
echo "staged build/ at identity-model $COMMIT" >&2
