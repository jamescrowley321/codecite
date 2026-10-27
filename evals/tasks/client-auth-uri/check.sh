#!/usr/bin/env bash
# Prove the task is well-formed. The token endpoint is the app's own, so no
# IdP is needed and planted.sh runs the hidden tests itself: the starting
# project fails them, the reference passes them and every check, and each
# planted bad solution fails.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$HERE/planted.sh"
