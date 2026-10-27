#!/usr/bin/env bash
# Prove the task is well-formed: score.py passes each question's reference
# answer file and fails a wrong answer, a wrong section, a missing file and
# malformed output. It reads questions.toml only, so it needs neither an IdP
# nor identity-model.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 -I "$HERE/planted.py"
