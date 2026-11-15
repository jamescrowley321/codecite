#!/usr/bin/env bash
# Prove a replay task is well-formed: its hidden tests fail on the project at
# the base and pass once the reference fix is applied. REPLAY_REPO is a clone
# holding both of the task's commits; the language's toolchain must be on
# PATH. The replay miner copies this template into every task it writes.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${REPLAY_REPO:?REPLAY_REPO must name a clone holding both commits of the task}"

# task.json's base commit, project directory, setup command and test
# command, NUL-separated so arguments keep their spaces. `{hidden}` in the
# test expands to the hidden paths, named from the project directory.
field() {
  python3 -I - "$HERE/task.json" "$1" <<'PY'
import json, posixpath, sys

task = json.load(open(sys.argv[1]))
value = task[sys.argv[2]]
if sys.argv[2] == "test":
    hidden = [posixpath.relpath(h, task["workdir"]) for h in task["hidden"]]
    value = [h for arg in value for h in (hidden if arg == "{hidden}" else [arg])]
for arg in value if isinstance(value, list) else [value]:
    sys.stdout.write(arg + "\0")
PY
}

mapfile -d '' -t base < <(field base)
mapfile -d '' -t workdir < <(field workdir)
mapfile -d '' -t setup < <(field setup)
mapfile -d '' -t test < <(field test)

# A fix the same size as the line it replaces, applied within a second of
# the first run, would leave Python's cached bytecode looking fresh.
export PYTHONDONTWRITEBYTECODE=1

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
git -C "$REPLAY_REPO" archive "${base[0]}" | tar -x -C "$WORK"
cd "$WORK/${workdir[0]}"
if ((${#setup[@]})); then
  echo "== setup: ${setup[*]}" >&2
  "${setup[@]}" >&2
fi
cp -R "$HERE/hidden/." "$WORK/"

echo "== hidden tests at the base" >&2
if "${test[@]}" >&2; then
  echo "check.sh: the hidden tests pass at the base, so they can't tell a fix" >&2
  exit 1
fi

echo "== hidden tests with the reference fix" >&2
git -C "$WORK" apply "$HERE/reference/fix.patch"
if ! "${test[@]}" >&2; then
  echo "check.sh: the hidden tests fail with the reference fix" >&2
  exit 1
fi
echo "check.sh: the hidden tests need the fix" >&2
