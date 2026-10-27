#!/usr/bin/env bash
# Prove the hidden tests and checks are sound. The reference solutions (one
# calling identity-model, one hand-written) pass both; the unchanged start and
# planted bad solutions fail exactly the tests or checks named for them;
# planted good variants fail nothing. Needs uv and an identity-model checkout
# (IDENTITY_MODEL_REPO); no model key, no IdP.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IDENTITY_MODEL_REPO="${IDENTITY_MODEL_REPO:-$HOME/repos/auth/identity-model}"
COMMIT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["corpus"]["commit"])' "$HERE/task.json")"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
export PYTHONDONTWRITEBYTECODE=1

# Export identity-model at the pinned commit, without .git.
mkdir -p "$WORK/identity-model"
git -C "$IDENTITY_MODEL_REPO" archive "$COMMIT" | tar -x -C "$WORK/identity-model"

# The score command, exactly as task.json has it, with {hidden} set to this
# task's hidden tests and a JUnit report so failures are read by test id.
mapfile -t SCORE_CMD < <(python3 -c '
import json, sys
for a in json.load(open(sys.argv[1]))["score"]:
    print(a.replace("{hidden}", sys.argv[2]))
' "$HERE/task.json" "$HERE/hidden")

R="test_rejected_with_400_and_session_kept"
START_FAILS="$R[events_missing],$R[events_not_object],$R[events_without_logout_member],$R[iat_missing],$R[jti_missing],$R[no_sub_or_sid],$R[nonce],$R[wrong_iss]"

# plant <name> <base> <failing tests, or "skip"> <failing checks> <edit run in the copy>
# base: start (the project as given), library (reference/app) or handwritten
# (reference/handwritten/app). Expected failures are comma-separated, sorted.
plant() {
  local name="$1" base="$2" want_tests="$3" want_checks="$4" edit="$5" dest="$WORK/$1"
  cp -r "$HERE/project" "$dest"
  case "$base" in
    library) cp "$HERE"/reference/app/*.py "$dest/app/" ;;
    handwritten) cp "$HERE"/reference/handwritten/app/*.py "$dest/app/" ;;
  esac
  # `set -e` is off inside a function called from `||`: check the edit here.
  (cd "$dest" && eval "$edit") || { echo "planted: $name: the edit did not apply" >&2; return 1; }
  local got_checks got_tests ok=1
  got_checks="$(python3 -I "$HERE/checks.py" "$HERE/project" "$dest" | python3 -c 'import json,sys; print(",".join(c["name"] for c in json.load(sys.stdin)["checks"] if not c["passed"]))')"
  [[ "$got_checks" == "$want_checks" ]] || { echo "planted: $name: checks failed [$got_checks], want [$want_checks]" >&2; ok=0; }
  if [[ "$want_tests" != skip ]]; then
    sed -i "s|@IDENTITY_MODEL@|$WORK/identity-model|g" "$dest/pyproject.toml"
    (cd "$dest" && uv sync --quiet && "${SCORE_CMD[@]}" --junitxml="$dest/junit.xml" >"$dest/score.log" 2>&1) || true
    got_tests="$(python3 - "$dest/junit.xml" <<'PY'
import sys, xml.etree.ElementTree as ET
try:
    root = ET.parse(sys.argv[1]).getroot()
except (OSError, ET.ParseError):
    print("no-report"); sys.exit()
cases = list(root.iter("testcase"))
bad = sorted(c.get("name") for c in cases if c.find("failure") is not None or c.find("error") is not None)
# 13 tests: anything fewer means collection broke.
print(",".join(bad) if len(cases) == 13 else f"collected-{len(cases)}")
PY
)"
    if [[ "$got_tests" != "$want_tests" ]]; then
      echo "planted: $name: tests failed [$got_tests], want [$want_tests]" >&2
      tail -30 "$dest/score.log" >&2
      ok=0
    fi
  fi
  if (( ok )); then echo "planted: $name: ok" >&2; else return 1; fi
}

status=0
# References pass the hidden tests and every check.
plant reference library "" "" ":" || status=1
plant handwritten handwritten "" "" ":" || status=1
# The unchanged start: signature and audience only.
plant start start "$START_FAILS" "" ":" || status=1
# Bad solutions that miss one spec rule each.
plant no-nonce-check handwritten "$R[nonce]" "" \
  "sed -i '/if \"nonce\" in claims:/,+1d' app/logout.py && ! grep -q nonce app/logout.py" || status=1
plant no-sub-sid-check handwritten "$R[no_sub_or_sid]" "" \
  "sed -i '/if \"sub\" not in claims and \"sid\" not in claims:/,+1d' app/logout.py && ! grep -q 'neither sub' app/logout.py" || status=1
# Bad solutions each check must catch.
plant hard-coded-jwks handwritten skip provider-from-env \
  "sed -i 's|_signing_keys(disco\[\"jwks_uri\"\])|_signing_keys(\"http://127.0.0.1:9000/jwks\")|' app/logout.py && grep -q 9000 app/logout.py" || status=1
plant hard-coded-issuer handwritten skip provider-from-env \
  "sed -i 's|issuer=disco\[\"issuer\"\]|issuer=\"https://op.example\"|' app/logout.py && grep -q op.example app/logout.py" || status=1
plant hard-coded-discovery library skip provider-from-env \
  "sed -i 's|os.environ\[\"OIDC_DISCOVERY_URL\"\]|\"http://127.0.0.1:9000/.well-known/openid-configuration\"|' app/logout.py && grep -q 9000 app/logout.py" || status=1
plant new-dependency library skip deps-unchanged \
  "sed -i 's/^dependencies = \[/dependencies = [\n    \"joserfc>=1\",/' pyproject.toml" || status=1
plant added-file library skip scope "touch run.sh" || status=1
plant added-conftest library skip scope "touch conftest.py" || status=1
plant added-pytest-ini library skip scope "printf '[pytest]\\naddopts = -p no:warnings\\n' > pytest.ini" || status=1
# Good variants the checks must accept.
plant own-tests-and-docs library "" "" "mkdir -p tests && touch tests/test_logout.py && echo note >> README.md && echo x > NOTES.md" || status=1
plant from-os-environ library skip "" "sed -i 's/^import os$/from os import environ, getenv/; s/os.environ\[\"OIDC_CLIENT_ID\"\]/getenv(\"OIDC_CLIENT_ID\")/; s/os.environ/environ/g' app/logout.py" || status=1
plant logic-moved-to-main library skip "" "cat app/logout.py >> app/main.py && rm app/logout.py" || status=1
exit "$status"
