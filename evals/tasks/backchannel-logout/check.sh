#!/usr/bin/env bash
# Prove the task is well-formed, with no model key and no IdP: task.json names
# files that exist, the prompt names no identity-model symbol and no spec
# section, the corpus is pinned, and planted.sh shows the hidden tests and
# checks pass the references and fail the planted bad solutions.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
IDENTITY_MODEL_REPO="${IDENTITY_MODEL_REPO:-$HOME/repos/auth/identity-model}"
export IDENTITY_MODEL_REPO

echo "== task.json, prompt and corpus" >&2
python3 -I - "$HERE" "$REPO_ROOT" "$IDENTITY_MODEL_REPO" <<'PY'
import ast, json, re, subprocess, sys, tomllib
from pathlib import Path

here, root, im = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
task = json.loads((here / "task.json").read_text())
problems = []

for key in ("project", "prompt", "hidden", "checks"):
    if not (here / task[key]).exists():
        problems.append(f"task.json {key}: {task[key]} is missing")
for path in ("reference/app/logout.py", "reference/handwritten/app/logout.py", "planted.sh"):
    if not (here / path).exists():
        problems.append(f"{path} is missing")

# The prompt states behaviour only: no identity-model symbol, no spec section.
prompt = (here / task["prompt"]).read_text()
commit = task["corpus"]["commit"]
init = subprocess.run(["git", "-C", im, "show", f"{commit}:py/src/py_identity_model/__init__.py"],
                      check=True, capture_output=True, text=True).stdout
exported = next(
    (ast.literal_eval(n.value) for n in ast.parse(init).body
     if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "__all__" for t in n.targets)),
    [],
)
if not exported:
    problems.append("could not read py_identity_model.__all__")
symbols = set(exported) | {"py_identity_model", "py-identity-model", "logout_logic", "validate_logout_token_claims"}
named = sorted(s for s in symbols if re.search(rf"(?<![\w-]){re.escape(s)}(?![\w-])", prompt))
if named:
    problems.append(f"prompt names identity-model symbols: {', '.join(named)}")
for pattern in (r"§", r"\bsection\b", r"\b\d+\.\d+\b", r"back-?channel", r"\bopenid\b", r"\brfc\s*\d+"):
    if m := re.search(pattern, prompt, re.IGNORECASE):
        problems.append(f"prompt names the spec: {m.group(0)!r}")

# The corpus: identity-model at the pinned commit plus the spec documents of
# the retrieval benchmark's manifest, each pinned by sha256.
manifest = tomllib.loads((root / task["corpus"]["documents"]).read_text())
sources = manifest.get("source", [])
repo = [s for s in sources if s.get("kind") == "git" and s.get("name") == task["corpus"]["repo"]]
if not repo or repo[0].get("commit") != commit:
    problems.append(f"{task['corpus']['documents']} pins {task['corpus']['repo']} at "
                    f"{repo[0].get('commit') if repo else 'nothing'}, task.json at {commit}")
docs = [s for s in sources if s.get("kind") == "document"]
unpinned = [s["name"] for s in docs if not re.fullmatch(r"[0-9a-f]{64}", s.get("sha256", ""))]
if unpinned:
    problems.append(f"documents without a sha256: {', '.join(unpinned)}")
if not any(s["name"] == "openid-connect-backchannel-1_0" and s.get("current") for s in docs):
    problems.append("the corpus lacks the current Back-Channel Logout spec")

for p in problems:
    print(f"check: {p}", file=sys.stderr)
if problems:
    sys.exit(1)
print(f"check: task.json, prompt and corpus ({len(docs)} documents): ok", file=sys.stderr)
PY

echo "== references and planted solutions" >&2
"$HERE/planted.sh"
