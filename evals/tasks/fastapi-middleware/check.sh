#!/usr/bin/env bash
# Prove the task is well-formed: the starting project fails the hidden tests
# and the reference solution passes them, against real tokens from the IdP.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
IDP_DIR="$REPO_ROOT/evals/fixtures/node-oidc"
IDENTITY_MODEL_REPO="${IDENTITY_MODEL_REPO:-$HOME/repos/auth/identity-model}"
COMMIT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["corpus"]["commit"])' "$HERE/task.json")"

WORK="$(mktemp -d)"
IDP_PID=""
cleanup() {
  if [[ -n "$IDP_PID" ]]; then
    kill "$IDP_PID" 2>/dev/null || true
    wait "$IDP_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# Export identity-model at the pinned commit, without .git.
mkdir -p "$WORK/identity-model"
git -C "$IDENTITY_MODEL_REPO" archive "$COMMIT" | tar -x -C "$WORK/identity-model"

# Start the IdP on a free loopback port.
[[ -d "$IDP_DIR/node_modules" ]] || (cd "$IDP_DIR" && npm ci --silent)
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
PORT="$PORT" node "$IDP_DIR/provider.js" >"$WORK/idp.log" 2>&1 &
IDP_PID=$!
export PYTHONDONTWRITEBYTECODE=1
export TEST_IDP="http://127.0.0.1:$PORT"
for _ in $(seq 1 50); do
  if curl -fsS "$TEST_IDP/.well-known/openid-configuration" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$IDP_PID" 2>/dev/null; then
    echo "check: IdP exited early" >&2
    cat "$WORK/idp.log" >&2
    exit 1
  fi
  sleep 0.2
done
curl -fsS "$TEST_IDP/.well-known/openid-configuration" >/dev/null \
  || { echo "check: IdP not ready at $TEST_IDP" >&2; cat "$WORK/idp.log" >&2; exit 1; }

# Copy the project, point it at the export, and install it.
prepare() {
  local dest="$1"
  cp -r "$HERE/project" "$dest"
  rm -rf "$dest/.venv"
  sed -i "s|@IDENTITY_MODEL@|$WORK/identity-model|g" "$dest/pyproject.toml"
  (cd "$dest" && uv sync --quiet)
}

TOTAL_TESTS=7

# The score command, exactly as task.json has it (the harness's score phase
# runs the same array), with {hidden} set to this task's hidden tests.
mapfile -t SCORE_CMD < <(python3 -c '
import json, sys
for a in json.load(open(sys.argv[1]))["score"]:
    print(a.replace("{hidden}", sys.argv[2]))
' "$HERE/task.json" "$HERE/hidden")

# Run the task's score command; print the number of tests that did not pass
# (failures, errors, and tests that never ran because collection broke).
score() {
  local dest="$1" out
  set +e
  out="$(cd "$dest" && "${SCORE_CMD[@]}" 2>&1)"
  set -e
  printf '%s\n' "$out" >&2
  local passed
  passed="$(printf '%s\n' "$out" | tail -1 | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' || true)"
  echo $(( TOTAL_TESTS - ${passed:-0} ))
}

status=0

echo "== starting project (expect >= 5 failures)" >&2
prepare "$WORK/start"
start_failed="$(score "$WORK/start")"
if (( start_failed >= 5 )); then
  echo "check: starting project fails $start_failed hidden tests: ok" >&2
else
  echo "check: starting project fails only $start_failed hidden tests (want >= 5)" >&2
  status=1
fi

echo "== reference solution (expect 0 failures)" >&2
prepare "$WORK/reference"
if [[ ! -f "$HERE/reference/app/main.py" ]]; then
  echo "check: no reference/app/main.py" >&2
  exit 1
fi
cp "$HERE/reference/app/main.py" "$WORK/reference/app/main.py"
ref_failed="$(score "$WORK/reference")"
if (( ref_failed == 0 )); then
  echo "check: reference passes every hidden test: ok" >&2
else
  echo "check: reference fails $ref_failed hidden tests (want 0)" >&2
  status=1
fi

# A correct solution that also turns on identity-model's documented
# hardening (an access-token marker claim is required) must pass too.
echo "== hardened reference, require_access_token_marker=True (expect 0 failures)" >&2
prepare "$WORK/hardened"
sed 's|excluded_paths=\["/health"\],|excluded_paths=["/health"],\n    require_access_token_marker=True,|' \
  "$HERE/reference/app/main.py" >"$WORK/hardened/app/main.py"
grep -q 'require_access_token_marker=True' "$WORK/hardened/app/main.py" \
  || { echo "check: could not make the hardened reference" >&2; exit 1; }
hard_failed="$(score "$WORK/hardened")"
if (( hard_failed == 0 )); then
  echo "check: hardened reference passes every hidden test: ok" >&2
else
  echo "check: hardened reference fails $hard_failed hidden tests (want 0)" >&2
  status=1
fi

echo "== planted bad solutions (each must fail its check)" >&2
"$HERE/planted.sh" || status=1

exit "$status"
