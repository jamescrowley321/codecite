#!/usr/bin/env bash
# Stage the sandbox image's build context in build/: the task's starting
# project (empty when task.json's project is null), identity-model at the
# task's pinned commit, the document sources of the task's corpus (task.json
# corpus.documents, a manifest), and a codecite binary. When task.json names
# a `specs` directory, specs.tsv lists the current documents to copy there as
# files, `<name> <sha256>` a line. The image holds one task: run this for the
# task you are about to run, then build.
#
# A replay task (task.json kind "replay") stages its repository instead:
# `git archive` of its base commit from REPLAY_REPO (default the history
# benchmark's clone, ~/.cache/codecite/history/<repo>) as the project, an
# empty identity-model, corpus.toml naming the project as the `eval` corpus,
# warm.sh to install its dependencies and build its tests, and the task's
# language as the Dockerfile's LANGUAGE default, which picks the toolchain.
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
KIND="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("kind") or "")' "$TASK/task.json")"

rm -rf "$HERE/build"; mkdir -p "$HERE/build/identity-model"
cp "$CODECITE" "$HERE/build/codecite"
cp "$HERE/pi-models.json" "$HERE/build/"
if [[ "$KIND" == replay ]]; then
  read -r NAME BASE LANGUAGE < <(python3 -c 'import json,sys; t=json.load(open(sys.argv[1])); print(t["repo"].split("/")[1], t["base"], t["language"])' "$TASK/task.json")
  REPLAY_REPO="${REPLAY_REPO:-$HOME/.cache/codecite/history/$NAME}"
  mkdir "$HERE/build/project"
  git -C "$REPLAY_REPO" archive "$BASE" | tar -x -C "$HERE/build/project"
  : >"$HERE/build/documents.toml"; : >"$HERE/build/specs.tsv"
  echo "$BASE" >"$HERE/build/COMMIT"
  # warm.sh runs in the image while it still has the network, so scoring
  # can run offline: the task's setup, then a build of its tests that runs
  # none. corpus.toml's @COMMIT@ becomes the project's one commit there.
  python3 - "$TASK/task.json" "$REPO_ROOT/evals/history/$NAME/manifest.toml" "$HERE/build" <<'PY'
import json, shlex, sys, tomllib
from pathlib import Path
task, history, build = json.load(open(sys.argv[1])), Path(sys.argv[2]), Path(sys.argv[3])
warm = {
    "python": [["uv", "run", "--with", "pytest==9.1.1", "pytest", "--version"]],
    "go": [["go", "test", "-count=1", "-run", "^$", "./..."]],
    "rust": [["cargo", "test", "--locked", "--no-run"]],
    "node": [],
}[task["language"]]
steps = ([task["setup"]] if task["setup"] else []) + warm
(build / "warm.sh").write_text("\n".join(["set -eux", "cd " + shlex.quote(task["workdir"]), *map(shlex.join, steps)]) + "\n")
license = ""
if history.exists():
    source = tomllib.loads(history.read_text())["source"][0]
    if "license" in source:
        license = f"license = {json.dumps(source['license'])}\n"
name = task["repo"].split("/")[1]
(build / "corpus.toml").write_text(
    f'name = "eval"\n\n[[source]]\nkind = "git"\nname = "{name}"\nurl = "/work/project"\n'
    f'commit = "@COMMIT@"\n{license}use = "adapt"\n'
)
PY
  sed "s/^ARG LANGUAGE=none$/ARG LANGUAGE=$LANGUAGE/" "$HERE/Dockerfile" >"$HERE/build/Dockerfile"
  grep -q "^ARG LANGUAGE=$LANGUAGE$" "$HERE/build/Dockerfile" || { echo "prepare: the Dockerfile has no ARG LANGUAGE=none line" >&2; exit 1; }
  echo "staged build/ for $TASK_ID: $NAME at $BASE, $LANGUAGE" >&2
  exit 0
fi
: >"$HERE/build/warm.sh"; : >"$HERE/build/corpus.toml"
cp "$HERE/Dockerfile" "$HERE/build/"
COMMIT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["corpus"]["commit"])' "$TASK/task.json")"
git -C "$IDENTITY_MODEL_REPO" archive "$COMMIT" | tar -x -C "$HERE/build/identity-model"
PROJECT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("project") or "")' "$TASK/task.json")"
if [[ -n "$PROJECT" ]]; then cp -r "$TASK/$PROJECT" "$HERE/build/project"; else mkdir "$HERE/build/project"; fi
# The [[source]] tables of kind "document" from the corpus.documents manifest,
# appended to the image's corpus manifest; empty when the task names none.
python3 - "$TASK/task.json" "$REPO_ROOT" "$HERE/build/specs.tsv" >"$HERE/build/documents.toml" <<'PY'
import json, re, sys, tomllib
from pathlib import Path
task = json.load(open(sys.argv[1]))
manifest = task["corpus"].get("documents")
docs, specs = [], []
if manifest:
    text = (Path(sys.argv[2]) / manifest).read_text()
    blocks = [b for b in re.split(r"(?m)^(?=\[\[source\]\])", text) if b.startswith("[[source]]")]
    for b in blocks:
        b = "\n".join(line for line in b.splitlines() if line.strip() and not line.startswith("#"))
        source = tomllib.loads(b)["source"][0]
        if source["kind"] == "document":
            docs.append(b)
            if task.get("specs") and source.get("current", True):
                specs.append(f"{source['name']} {source['sha256']}\n")
    print("\n\n".join(docs))
Path(sys.argv[3]).write_text("".join(specs))
PY
echo "$COMMIT" > "$HERE/build/COMMIT"
echo "staged build/ for $TASK_ID at identity-model $COMMIT, $(grep -c '^\[\[source\]\]' "$HERE/build/documents.toml" || true) documents, $(wc -l < "$HERE/build/specs.tsv") of them as spec files" >&2
