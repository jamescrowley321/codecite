#!/usr/bin/env bash
# Stage the sandbox image's build context in build/: the task's starting
# project, identity-model at the task's pinned commit, the document sources of
# the task's corpus (task.json corpus.documents, a manifest), and a codecite
# binary. The image holds one task: run this for the task you are about to
# run, then build.
#
#   inspect/prepare.sh [task-id]    (default fastapi-middleware)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
TASK_ID="${1:-fastapi-middleware}"
TASK="$REPO_ROOT/evals/tasks/$TASK_ID"
[[ -f "$TASK/task.json" ]] || { echo "prepare: no task $TASK_ID at $TASK" >&2; exit 1; }
IDENTITY_MODEL_REPO="${IDENTITY_MODEL_REPO:-$HOME/repos/auth/identity-model}"
CODECITE="${CODECITE:-$REPO_ROOT/target/release/codecite}"
COMMIT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["corpus"]["commit"])' "$TASK/task.json")"

rm -rf "$HERE/build"; mkdir -p "$HERE/build/identity-model"
git -C "$IDENTITY_MODEL_REPO" archive "$COMMIT" | tar -x -C "$HERE/build/identity-model"
cp -r "$TASK/project" "$HERE/build/project"
cp "$CODECITE" "$HERE/build/codecite"
cp "$HERE/Dockerfile" "$HERE/pi-models.json" "$HERE/build/"
# The [[source]] tables of kind "document" from the corpus.documents manifest,
# appended to the image's corpus manifest; empty when the task names none.
python3 - "$TASK/task.json" "$REPO_ROOT" >"$HERE/build/documents.toml" <<'PY'
import json, re, sys, tomllib
from pathlib import Path
manifest = json.load(open(sys.argv[1]))["corpus"].get("documents")
if manifest:
    text = (Path(sys.argv[2]) / manifest).read_text()
    blocks = [b for b in re.split(r"(?m)^(?=\[\[source\]\])", text) if b.startswith("[[source]]")]
    docs = []
    for b in blocks:
        b = "\n".join(line for line in b.splitlines() if line.strip() and not line.startswith("#"))
        if tomllib.loads(b)["source"][0]["kind"] == "document":
            docs.append(b)
    print("\n\n".join(docs))
PY
echo "$COMMIT" > "$HERE/build/COMMIT"
echo "staged build/ for $TASK_ID at identity-model $COMMIT, $(grep -c '^\[\[source\]\]' "$HERE/build/documents.toml" || true) documents" >&2
