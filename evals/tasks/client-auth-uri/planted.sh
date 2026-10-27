#!/usr/bin/env bash
# Prove the task and its checks are sound: the reference passes the hidden
# tests and every check, and each planted solution, built from the reference
# (or the starting project), fails exactly the tests and checks it should.
# The token endpoint is the app's own, so no IdP is needed.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
REF="$HERE/reference/app/main.py"
export PYTHONDONTWRITEBYTECODE=1

# One installed project; each variant's app/ is swapped into it to run the
# hidden tests.
cp -r "$HERE/project" "$WORK/run"
rm -rf "$WORK/run/.venv"
(cd "$WORK/run" && uv sync --quiet)

# The score command, exactly as task.json has it, plus -rfE so failed tests
# are named.
mapfile -t SCORE_CMD < <(python3 -c '
import json, sys
for a in json.load(open(sys.argv[1]))["score"]:
    print(a.replace("{hidden}", sys.argv[2]))
' "$HERE/task.json" "$HERE/hidden")
SCORE_CMD+=(-rfE)

# Comma-separated names of the hidden tests that did not pass, sorted.
failed_tests() {
  local out
  set +e
  out="$(cd "$WORK/run" && "${SCORE_CMD[@]}" 2>&1)"
  set -e
  if ! printf '%s\n' "$out" | tail -1 | grep -qE '[0-9]+ (passed|failed)'; then
    printf '%s\n' "$out" >&2
    echo "COLLECTION-ERROR"
    return
  fi
  printf '%s\n' "$out" | grep -E '^(FAILED|ERROR) ' | grep -oE '::test_[a-z_]+' | cut -c3- | sort -u | paste -sd, - || true
}

plant() { # plant <name> <failing tests> <failing checks> <edit command run in the copy>
  local name="$1" want_tests="$2" want_checks="$3" edit="$4" dest="$WORK/$1"
  cp -r "$HERE/project" "$dest"; rm -rf "$dest/.venv"; cp "$REF" "$dest/app/main.py"
  (cd "$dest" && eval "$edit")
  local checks tests
  checks="$(python3 -I "$HERE/checks.py" "$HERE/project" "$dest" | python3 -c 'import json,sys; print(",".join(c["name"] for c in json.load(sys.stdin)["checks"] if not c["passed"]))')"
  rm -rf "$WORK/run/app"; cp -r "$dest/app" "$WORK/run/app"
  tests="$(failed_tests)"
  if [[ "$tests" == "$want_tests" && "$checks" == "$want_checks" ]]; then
    echo "planted: $name fails tests [$tests] and checks [$checks]: ok" >&2
  else
    echo "planted: $name should fail tests [$want_tests] and checks [$want_checks]; failed tests [$tests] and checks [$checks]" >&2
    return 1
  fi
}

QUERY_TESTS="test_query_credentials_rejected,test_query_secret_rejected"
status=0
plant reference "" "" ":" || status=1
plant unchanged-start "$QUERY_TESTS" cites-section "cp '$HERE/project/app/main.py' app/main.py" || status=1
# Overcorrections: §2.3.1 allows the form body and requires HTTP Basic.
plant drops-form-body test_form_body_gets_token "" "sed -i 's/= form.get(\"client_id\"), form.get(\"client_secret\")$/= None, None/' app/main.py" || status=1
plant drops-basic test_basic_gets_token "" "sed -i 's/creds = basic_credentials(request)$/creds = None/' app/main.py" || status=1
# A correct fix whose comment cites the wrong section, or the right section of the wrong RFC.
plant wrong-section "" cites-section "sed -i 's/RFC 6749 §2.3.1:/RFC 6749 §2.3:/' app/main.py" || status=1
plant wrong-subsection "" cites-section "sed -i 's/RFC 6749 §2.3.1:/RFC 6749 §2.3.10:/' app/main.py" || status=1
plant wrong-subsubsection "" cites-section "sed -i 's/RFC 6749 §2.3.1:/RFC 6749 §2.3.1.2:/' app/main.py" || status=1
plant wrong-rfc "" cites-section "sed -i 's/RFC 6749 §2.3.1:/RFC 6750 §2.3.1:/' app/main.py" || status=1
plant citation-in-code "" cites-section "sed -i 's/RFC 6749 §2.3.1:/See the spec:/; s/^app = FastAPI(title=\"Token Service\")$/app = FastAPI(title=\"RFC 6749 §2.3.1\")/' app/main.py" || status=1
plant new-dependency "" deps-unchanged "sed -i 's/^dependencies = \[/dependencies = [\n    \"authlib>=1\",/' pyproject.toml" || status=1
plant deleted-file "" scope "rm README.md && rm app/__init__.py" || status=1
plant added-file "" scope "touch run.sh" || status=1
# Valid citation forms the check must accept: every check passes.
plant codecite-citation "" "" "sed -i 's/RFC 6749 §2.3.1:/rfc6749@sha256:535362fa3b4c#section-2.3.1:/' app/main.py" || status=1
plant spec-url "" "" "sed -i 's|RFC 6749 §2.3.1:|https://www.rfc-editor.org/rfc/rfc6749.html#section-2.3.1|' app/main.py" || status=1
plant section-of-rfc "" "" "sed -i 's/RFC 6749 §2.3.1:/Section 2.3.1 of RFC 6749:/' app/main.py" || status=1
plant rfc-comma-section "" "" "sed -i 's/RFC 6749 §2.3.1:/RFC6749, Section 2.3.1:/' app/main.py" || status=1
plant wrapped-citation "" "" "sed -i 's/# RFC 6749 §2.3.1: client/# Per RFC 6749\n    # §2.3.1, client/' app/main.py" || status=1
plant docstring-citation "" "" "sed -i 's/RFC 6749 §2.3.1:/See below:/; s|^    form = await request.form()$|    \"\"\"Issue a token. Client credentials per RFC 6749 §2.3.1.\"\"\"\n    form = await request.form()|' app/main.py" || status=1
# Ignoring URI credentials, rather than rejecting the request outright, also fails authentication.
plant ignores-query "" "" "sed -i '/^    if \"client_id\" in request.query_params/,/^        return invalid_client()$/d' app/main.py && ! grep -q query_params app/main.py" || status=1
plant docs-changed "" "" "echo 'Credentials never go in the URL.' >> README.md && echo notes > NOTES.md" || status=1
# Refusing URI credentials as a malformed request is §5.2's other reading.
plant uri-invalid-request "" "" "sed -i '/^    if \"client_id\" in request.query_params/,/^        return invalid_client()$/s/return invalid_client()/return error(\"invalid_request\", 400)/' app/main.py && grep -q '\"invalid_request\", 400' app/main.py" || status=1
# Only for URI credentials: a wrong secret is still invalid_client.
plant wrong-secret-invalid-request test_unknown_client_rejected,test_wrong_basic_secret_rejected,test_wrong_form_secret_rejected "" "sed -i 's/return error(\"invalid_client\", 401/return error(\"invalid_request\", 400/' app/main.py" || status=1
# A list of sections that includes 2.3.1 cites it; one that doesn't, doesn't.
plant section-list "" "" "sed -i 's|RFC 6749 §2.3.1:|RFC 6749 section 2.3 / 2.3.1 / 3.2:|' app/main.py" || status=1
plant section-list-without "" cites-section "sed -i 's|RFC 6749 §2.3.1:|RFC 6749 sections 2.3, 2.3.10 and 3.2:|' app/main.py" || status=1
plant section-signs-list "" "" "sed -i 's|RFC 6749 §2.3.1:|RFC 6749 §2.3, §2.3.1:|' app/main.py" || status=1
plant sections-of-rfc "" "" "sed -i 's|RFC 6749 §2.3.1:|Sections 2.3.1 and 3.2 of RFC 6749:|' app/main.py" || status=1
plant list-of-other-rfc "" cites-section "sed -i 's|RFC 6749 §2.3.1:|RFC 6749 §3.2 and 2.3.1 of RFC 7523:|' app/main.py" || status=1
# The agent's own tests are not out of scope; other new files still are.
plant added-tests "" "" "mkdir -p tests && echo 'def test_x(): pass' > tests/test_token.py && echo 'def test_y(): pass' > test_app.py && echo '' > tests/conftest.py" || status=1
plant added-non-test-under-tests "" scope "mkdir -p tests && echo 'echo hi' > tests/run.sh" || status=1
exit "$status"
