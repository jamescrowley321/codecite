#!/usr/bin/env bash
# Prove house-rules well-formed, with no model key, no sandbox and no codecite:
# task.json names files that exist, the prompt names no identity-stack helper,
# every corpus manifest pins its git sources by full commit and starts with the
# same identity-stack pin, whole, the start the image builds from that pin has
# task.json's start_sha256 (the digest the scorer checks the sandbox's start
# against), rules.toml agrees with checks.py, the hidden tests
# fail on identity-stack as the agent gets it and pass with the reference
# solution, checks.py passes both trees, and planted.sh shows each planted
# rule break or hand-rolled helper caught.
#
#   evals/tasks/house-rules/check.sh
#
# Needs git, uv and network for identity-stack's dependencies.
# IDENTITY_STACK_REPO names a local clone holding the pinned commit; without
# one it is fetched from GitHub. (That `codecite read` resolves every rule's
# citation is checked where the corpus is indexed: the sandbox image's build.)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "== task.json, prompt, manifests and rules" >&2
SHA="$(python3 -I - "$HERE" <<'PY'
import json, re, sys, tomllib
from pathlib import Path

here = Path(sys.argv[1])
task = json.loads((here / "task.json").read_text())
problems = []
for key in ("prompt", "hidden", "checks", "rules", "probe"):
    if not (here / task[key]).exists():
        problems.append(f"task.json {key}: {task[key]} is missing")
for path in ("reference/solution.patch", "planted.sh", "planted.py"):
    if not (here / path).exists():
        problems.append(f"{path} is missing")

# The prompt names no helper the rule layer provides and no rule.
prompt = (here / task["prompt"]).read_text()
for word in ("knob", "outbound", "logbook", "result_to_response", "require_role", "get_tenant_id",
             "conventions", "ADR", "domain", "Result", "MUST"):
    if re.search(rf"\b{re.escape(word)}\b", prompt):
        problems.append(f"prompt.md names {word!r}")

# Every size pins the same project first, and every git source by full SHA.
pins = set()
for size, rel in task["corpus"]["sizes"].items():
    manifest = tomllib.loads((here / rel).read_text())
    if manifest.get("name") != "eval":
        problems.append(f"{rel}: the corpus must be named eval")
    sources = manifest["source"]
    if sources[0].get("name") != task["corpus"]["project"]:
        problems.append(f"{rel}: the first source must be the project, {task['corpus']['project']}")
    elif sources[0].get("include"):
        problems.append(f"{rel}: the project is staged whole, so its start is start_sha256's; drop its include")
    for s in sources:
        if s.get("kind", "git") == "git":
            if not re.fullmatch(r"[0-9a-f]{40}", s.get("commit", "")):
                problems.append(f"{rel}: {s.get('name')} is not pinned by a full commit SHA")
            for pattern in s.get("include", []):
                if not pattern.endswith("/**"):
                    problems.append(f"{rel}: {s['name']} include {pattern!r} must be <dir>/**, which prepare.sh can stage")
        elif s.get("kind") == "document" and not re.fullmatch(r"[0-9a-f]{64}", s.get("sha256", "")):
            problems.append(f"{rel}: {s.get('name')} is not pinned by sha256")
    pins.add(sources[0].get("commit"))
if task["corpus"]["default"] not in task["corpus"]["sizes"]:
    problems.append("corpus.default is not a size")
if len(pins) != 1:
    problems.append(f"the sizes pin the project at different commits: {sorted(pins)}")

# rules.toml's control rules are checks.py's, and its citations name the pin.
rules = tomllib.loads((here / task["rules"]).read_text())["rule"]
controls = {r["where"].split("§")[1] for r in rules if r["kind"] == "control"}
m = re.search(r"CONTROL_RULES = \{(.*?)\}", (here / task["checks"]).read_text())
if m is None or {x.strip().strip('"§') for x in m.group(1).split(",")} != controls:
    problems.append(f"checks.py CONTROL_RULES must be rules.toml's control rules: {sorted(controls, key=int)}")
for r in rules:
    if r["kind"] not in ("house", "habit", "control"):
        problems.append(f"rule {r['id']}: kind {r['kind']!r}")
    if not re.fullmatch(rf"{task['corpus']['project']}@[0-9a-f]{{7,40}}:[^#]+#L\d+-L\d+", r["citation"]):
        problems.append(f"rule {r['id']}: citation {r['citation']!r} is not a line-range citation into the project")
    elif not next(iter(pins), "").startswith(r["citation"].split("@")[1].split(":")[0]):
        problems.append(f"rule {r['id']}: citation is not at the pinned commit")

if problems:
    sys.exit("\n".join(problems))
print(next(iter(pins)))
PY
)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
REPO="${IDENTITY_STACK_REPO:-}"
if [[ -z "$REPO" ]] || ! git -C "$REPO" cat-file -e "$SHA^{commit}" 2>/dev/null; then
  echo "== fetching identity-stack at $SHA" >&2
  REPO="$WORK/repo"
  git init -q "$REPO"
  git -C "$REPO" fetch -q --depth 1 https://github.com/jamescrowley321/identity-stack "$SHA"
fi
mkdir "$WORK/start" "$WORK/reference"
git -C "$REPO" archive "$SHA" | tar -x -C "$WORK/start"
git -C "$REPO" archive "$SHA" | tar -x -C "$WORK/reference"

echo "== the image's start has task.json's start_sha256" >&2
# As the Dockerfile builds /opt/project-start.tar: the staged files committed
# afresh (so .gitignore applies) and archived.
mkdir "$WORK/image"
git -C "$REPO" archive "$SHA" | tar -x -C "$WORK/image"
(cd "$WORK/image" && git init -q . && git add -A \
  && git -c user.name=eval -c user.email=eval@localhost commit -qm export && git archive -o "$WORK/project-start.tar" HEAD)
DIGEST="$(uv run -q --project "$HERE/../.." python -I -c \
  'import sys; from codecite_evals.task import tree_digest; print(tree_digest(open(sys.argv[1], "rb").read()))' \
  "$WORK/project-start.tar")"
PINNED="$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["corpus"]["start_sha256"])' "$HERE/task.json")"
if [[ "$DIGEST" != "$PINNED" ]]; then
  echo "check: the start's digest is $DIGEST, task.json's start_sha256 $PINNED" >&2
  exit 1
fi
git -C "$WORK/reference" apply "$HERE/reference/solution.patch" 2>/dev/null \
  || (cd "$WORK/reference" && git apply "$HERE/reference/solution.patch")

echo "== identity-stack's environment, as the scorer builds it (task.json score_setup)" >&2
(cd "$WORK/start/backend" && uv sync --locked --no-install-project -q)
VENV="$WORK/start/backend/.venv/bin"
hidden() { (cd "$1/backend" && PYTHONDONTWRITEBYTECODE=1 "$VENV/python" -m pytest -q -p no:cacheprovider "$HERE/hidden" "${@:2}"); }

echo "== the hidden tests fail on the start" >&2
if hidden "$WORK/start" >/dev/null 2>&1; then
  echo "check: the hidden tests pass on identity-stack as the agent gets it" >&2
  exit 1
fi
echo "== the hidden tests pass with the reference" >&2
hidden "$WORK/reference" | tail -1

echo "== checks.py passes the start and the reference, controls too" >&2
for tree in start reference; do
  HOUSE_RULES_VENV="$VENV" python3 -I "$HERE/checks.py" "$WORK/start" "$WORK/$tree" >"$WORK/$tree.json"
  python3 -I - "$WORK/$tree.json" "$tree" <<'PY'
import json, sys
checks = json.load(open(sys.argv[1]))["checks"]
failed = [f"{c['name']}: {c['detail'][:300]}" for c in checks if not c["passed"]]
if failed:
    sys.exit(f"check: checks.py fails the {sys.argv[2]}:\n" + "\n".join(failed))
print(f"{sys.argv[2]}: {len(checks)} checks passed", file=sys.stderr)
PY
done

echo "== planted rule breaks fail" >&2
"$HERE/planted.sh" "$WORK/reference" "$VENV" "$WORK/start"
