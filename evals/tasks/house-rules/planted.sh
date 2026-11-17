#!/usr/bin/env bash
# Show that each planted rule break or hand-rolled helper in planted.py fails
# the hidden tests or the check that decides its rule (check.sh runs this).
#
#   planted.sh <reference tree> <venv bin> <start tree> [variant ...]
#
# The reference tree is identity-stack at the pin with reference/solution.patch
# applied, the venv the start's backend/.venv/bin, the start tree identity-stack
# at the pin as the agent gets it.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
(($# >= 3)) || { echo "usage: $0 <reference tree> <venv bin> <start tree> [variant ...]" >&2; exit 2; }
REFERENCE="$1" VENV="$2" START="$3"
shift 3
exec python3 -I "$HERE/planted.py" "$REFERENCE" "$VENV" "$HERE/hidden" "$HERE/checks.py" "$START" "$@"
