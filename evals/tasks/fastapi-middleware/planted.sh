#!/usr/bin/env bash
# Prove each deterministic check can fail: one planted bad solution per check,
# built from the reference, must fail exactly that check. Planted good
# variants (check "") must fail none.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
REF="$HERE/reference/app/main.py"

plant() { # plant <name> <check> <edit command run in the copy>
  local name="$1" check="$2" edit="$3" dest="$WORK/$1"
  cp -r "$HERE/project" "$dest"; cp "$REF" "$dest/app/main.py"
  (cd "$dest" && eval "$edit")
  local failed
  failed="$(python3 -I "$HERE/checks.py" "$HERE/project" "$dest" | python3 -c 'import json,sys; print(",".join(c["name"] for c in json.load(sys.stdin)["checks"] if not c["passed"]))')"
  if [[ "$failed" == "$check" ]]; then echo "planted: $name fails $check: ok" >&2
  else echo "planted: $name should fail only $check, failed [$failed]" >&2; return 1; fi
}

status=0
plant hand-rolled uses-middleware "sed -i 's/^app.add_middleware(/_unused = dict(/; s/^    TokenValidationMiddleware,$/    m=TokenValidationMiddleware,/' app/main.py" || status=1
plant no-current-user uses-current-user "sed -i 's/async def me(user: CurrentUser):/async def me(user=None):/' app/main.py" || status=1
plant hard-coded-url reads-env "sed -i 's|os.environ\[\"OIDC_DISCOVERY_URL\"\]|\"http://localhost:9000/.well-known/openid-configuration\"|' app/main.py" || status=1
plant new-dependency deps-unchanged "sed -i 's/^dependencies = \[/dependencies = [\n    \"pyjwt>=2\",/' pyproject.toml" || status=1
plant deleted-file scope "rm README.md && touch NOTES.txt && rm app/__init__.py" || status=1
plant added-file scope "touch run.sh" || status=1
# Valid spellings the checks must accept: every check passes.
plant keyword-args "" "sed -i 's/^    TokenValidationMiddleware,$/    middleware_class=TokenValidationMiddleware,/; s/@app.get(\"\\/me\")/@app.get(path=\"\\/me\")/' app/main.py" || status=1
plant from-os-environ "" "sed -i 's/^import os$/from os import environ/; s/os.environ/environ/g' app/main.py" || status=1
plant current-user-default "" "sed -i 's/async def me(user: CurrentUser):/async def me(user=CurrentUser):/' app/main.py" || status=1
exit "$status"
