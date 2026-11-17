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
# A task with a size dial (task.json kind "corpus", house-rules) stages the
# manifest of one size, CORPUS (s, m or l; default the task's own): each git
# source as files at its pinned commit (the project, task.json
# corpus.project, as the project; every other under corpus/<name>, cut to its
# `include` directories), each current document's name and hash in specs.tsv,
# the manifest itself as corpus.toml (the image indexes it at the pinned
# commits, so its citations name them), the size in corpus/.size, and the
# citations of the task's rules (rules.toml) in citations.txt, which the image
# build resolves with `codecite read`. Git sources are fetched once into
# CORPUS_CACHE (default ~/.cache/codecite/eval-corpus), or read from a local
# clone named by <NAME>_REPO (IDENTITY_STACK_REPO, IDENTITY_MODEL_REPO, ...).
#
#   inspect/prepare.sh [task-id]    (default fastapi-middleware)
#   CORPUS=s inspect/prepare.sh house-rules
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
TASK_ID="${1:-fastapi-middleware}"
TASK="$REPO_ROOT/evals/tasks/$TASK_ID"
[[ -f "$TASK/task.json" ]] || { echo "prepare: no task $TASK_ID at $TASK" >&2; exit 1; }
IDENTITY_MODEL_REPO="${IDENTITY_MODEL_REPO:-$HOME/repos/auth/identity-model}"
CODECITE="${CODECITE:-$REPO_ROOT/target/release/codecite}"
KIND="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("kind") or "")' "$TASK/task.json")"

rm -rf "$HERE/build"; mkdir -p "$HERE/build/identity-model" "$HERE/build/corpus"
: >"$HERE/build/citations.txt"
cp "$CODECITE" "$HERE/build/codecite"
cp "$HERE/pi-models.json" "$HERE/build/"
if [[ "$KIND" == corpus ]]; then
  python3 - "$TASK" "$HERE/build" "${CORPUS:-}" "${CORPUS_CACHE:-$HOME/.cache/codecite/eval-corpus}" <<'PY'
import json, os, re, shlex, subprocess, sys, tomllib
from pathlib import Path

task_dir, build, size, cache = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4])
task = json.loads((task_dir / "task.json").read_text())
size = size or task["corpus"]["default"]
if size not in task["corpus"]["sizes"]:
    sys.exit(f"prepare: CORPUS must be one of {', '.join(task['corpus']['sizes'])}, not {size!r}")
manifest_text = (task_dir / task["corpus"]["sizes"][size]).read_text()
manifest = tomllib.loads(manifest_text)


def repo_for(source):
    """A git directory holding the source's pinned commit: a local clone named
    by <NAME>_REPO when it has the commit, else a fetch into the cache."""
    name, commit = source["name"], source["commit"]
    local = os.environ.get(re.sub(r"\W", "_", name).upper() + "_REPO")
    if local and subprocess.run(["git", "-C", local, "cat-file", "-e", f"{commit}^{{commit}}"], capture_output=True).returncode == 0:
        return local
    repo = cache / name
    if not (repo / ".git").exists():
        repo.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
    if subprocess.run(["git", "-C", str(repo), "cat-file", "-e", f"{commit}^{{commit}}"], capture_output=True).returncode:
        subprocess.run(["git", "-C", str(repo), "fetch", "-q", "--depth", "1", source["url"], commit], check=True)
    return str(repo)


specs = []
for source in manifest["source"]:
    kind = source.get("kind", "git")
    if kind == "document":
        if source.get("current", True):
            specs.append(f"{source['name']} {source['sha256']}\n")
        continue
    if kind != "git":
        sys.exit(f"prepare: {source.get('name')}: a size-dial corpus holds git and document sources only")
    dest = build / "project" if source["name"] == task["corpus"]["project"] else build / "corpus" / source["name"]
    dest.mkdir(parents=True)
    paths = [p.removesuffix("/**") for p in source.get("include", [])]
    archive = subprocess.Popen(["git", "-C", repo_for(source), "archive", source["commit"], *paths], stdout=subprocess.PIPE)
    subprocess.run(["tar", "-x", "-C", str(dest)], stdin=archive.stdout, check=True)
    if archive.wait():
        sys.exit(f"prepare: git archive of {source['name']} at {source['commit']} failed")
if not (build / "project").exists():
    sys.exit(f"prepare: the {size} manifest has no source named {task['corpus']['project']}")
(build / "corpus" / ".size").write_text(size + "\n")
(build / "corpus.toml").write_text(manifest_text)
(build / "specs.tsv").write_text("".join(specs))
(build / "documents.toml").write_text("")
(build / "COMMIT").write_text(manifest["source"][0]["commit"] + "\n")
steps = [task["setup"]] if task.get("setup") else []
(build / "warm.sh").write_text("\n".join(["set -eux", "cd " + shlex.quote(task["workdir"]), *map(shlex.join, steps)]) + "\n")
rules = tomllib.loads((task_dir / task["rules"]).read_text())["rule"] if task.get("rules") else []
(build / "citations.txt").write_text("".join(r["citation"] + "\n" for r in rules))
print(f"staged build/ for {task['id']} at corpus size {size}: {len(manifest['source'])} sources, "
      f"{len(specs)} spec files, {len(rules)} rule citations", file=sys.stderr)
PY
  LANGUAGE="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["language"])' "$TASK/task.json")"
  sed "s/^ARG LANGUAGE=none$/ARG LANGUAGE=$LANGUAGE/" "$HERE/Dockerfile" >"$HERE/build/Dockerfile"
  grep -q "^ARG LANGUAGE=$LANGUAGE$" "$HERE/build/Dockerfile" || { echo "prepare: the Dockerfile has no ARG LANGUAGE=none line" >&2; exit 1; }
  exit 0
fi
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
