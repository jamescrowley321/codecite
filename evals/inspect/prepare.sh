#!/usr/bin/env bash
# Stage a task's sandbox image build context in build/<task>/: the task's
# starting project, identity-model at the task's pinned commit, the spec
# documents its corpus adds, and a codecite binary.
#
#   inspect/prepare.sh [task]     (default fastapi-middleware)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
TASK_ID="${1:-fastapi-middleware}"
TASK="$REPO_ROOT/evals/tasks/$TASK_ID"
BUILD="$HERE/build/$TASK_ID"
IDENTITY_MODEL_REPO="${IDENTITY_MODEL_REPO:-$HOME/repos/auth/identity-model}"
CODECITE="${CODECITE:-$REPO_ROOT/target/release/codecite}"
[[ -f "$TASK/task.json" ]] || { echo "prepare: no task at $TASK" >&2; exit 1; }
COMMIT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["corpus"]["commit"])' "$TASK/task.json")"

rm -rf "$BUILD"; mkdir -p "$BUILD/identity-model"
git -C "$IDENTITY_MODEL_REPO" archive "$COMMIT" | tar -x -C "$BUILD/identity-model"
cp -r "$TASK/project" "$BUILD/project"
cp "$CODECITE" "$BUILD/codecite"
cp "$HERE/Dockerfile" "$HERE/pi-models.json" "$BUILD/"
echo "$COMMIT" > "$BUILD/COMMIT"
# The document sources of the manifest task.json's corpus names, copied as
# written (each pinned by sha256); empty when the corpus is code only.
python3 -I - "$TASK/task.json" "$REPO_ROOT" > "$BUILD/documents.toml" <<'PY'
import json, re, sys, tomllib
from pathlib import Path
manifest = json.load(open(sys.argv[1]))["corpus"].get("documents")
if manifest:
    text = (Path(sys.argv[2]) / manifest).read_text()
    blocks = re.split(r"^(?=\[\[source\]\])", text, flags=re.M)
    docs = [b.strip() for b in blocks if b.startswith("[[source]]") and tomllib.loads(b)["source"][0]["kind"] == "document"]
    print("\n\n".join(docs))
PY
echo "staged build/$TASK_ID at identity-model $COMMIT, $(grep -c '^\[\[source\]\]' "$BUILD/documents.toml" || true) documents" >&2
