#!/usr/bin/env bash
# Start one Claude Code cloud session per issue and print each session's link.
#
#   scripts/cloud-launch.sh <issue> [<issue> ...]
#   scripts/cloud-launch.sh 111 117 105
#
# Each session works its issue per AGENTS.md "Working an issue end to end".
# The sessions run in the cloud: once this script returns, nothing stays
# running locally, and the machine can go offline.
#
# `claude --cloud` refuses to run without a terminal, so each launch runs
# under `script`, which gives it one. Every launch is also appended to
# $CODECITE_CLOUD_LOG (default ~/.local/state/codecite/cloud-sessions.log)
# as "<UTC time> #<issue> <link>", so the links survive the terminal.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: $0 <issue> [<issue> ...]" >&2
  exit 2
fi

LOG="${CODECITE_CLOUD_LOG:-$HOME/.local/state/codecite/cloud-sessions.log}"
mkdir -p "$(dirname "$LOG")"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

status=0
for n in "$@"; do
  if [[ ! "$n" =~ ^[0-9]+$ ]]; then
    echo "#$n: not an issue number" >&2
    status=1
    continue
  fi
  prompt="Work codecite#$n per AGENTS.md 'Working an issue end to end'"
  # `script` records the session's output, terminal escapes and all.
  timeout 120 script -qfc "claude --cloud \"$prompt\"" "$TMP/$n.log" < /dev/null > /dev/null 2>&1 || true
  url="$(sed 's/\x1b\[[0-9;?]*[a-zA-Z]//g' "$TMP/$n.log" | tr -d '\r' \
    | grep -o 'https://claude.ai/code/session_[A-Za-z0-9]*' | head -1 || true)"
  if [[ -z "$url" ]]; then
    echo "#$n: no session created; output follows" >&2
    # An empty log matches nothing; that must not stop the remaining launches.
    sed 's/\x1b\[[0-9;?]*[a-zA-Z]//g' "$TMP/$n.log" 2> /dev/null | tr -d '\r' | grep -v '^\s*$' | tail -5 >&2 || true
    status=1
    continue
  fi
  echo "#$n: $url"
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) #$n $url" >> "$LOG"
done
exit "$status"
