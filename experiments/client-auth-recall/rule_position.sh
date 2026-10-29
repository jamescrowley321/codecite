#!/bin/sh
# Where RFC 6749 §3.2's repeated-parameter rule lands in the `requirements`
# JSON for the sections agents asked about on client-auth-uri (#170).
# Usage: rule_position.sh <codecite binary> <corpus>
set -eu
for citation in "RFC 6749 §2.3" "RFC 6749 §2.3.1"; do
  "$1" requirements "$2" "$citation" --json | python3 -I -c '
import json, sys
body = sys.stdin.read()
at = body.find("MUST NOT be included more than once")
groups = [s["citation"].rsplit("#section-", 1)[1] for s in json.loads(body)["sections"]]
print(f"{sys.argv[1]}: char {at} of {len(body)} ({at * 100 // len(body)}%), groups {groups}")
' "$citation"
done
