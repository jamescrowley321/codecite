#!/usr/bin/env bash
# Wait for the CI run (ci.yml) on one commit and report how it went.
#
#   ralph/wait-ci.sh <branch> [sha]     # sha defaults to HEAD
#
# Sleeps a minute first so GitHub has registered the push, then waits for the
# run for exactly this commit, so a stale result from an earlier push is never
# read. Prints one line, then the failed jobs' logs on failure:
#   PASS <run id>   exit 0
#   FAIL <run id>   exit 1
#   NO_RUN          exit 2   (no run appeared within 3 minutes of the sleep)
#
# Run it in the foreground; with CI's few-minute jobs it fits inside one Bash
# call (10-minute cap).
set -euo pipefail

branch="$1"
sha="${2:-$(git rev-parse HEAD)}"

sleep 60

deadline=$((SECONDS + 180))
while :; do
  id=$(gh run list --workflow ci.yml --branch "$branch" --commit "$sha" --limit 1 \
    --json databaseId --jq '.[0].databaseId // empty')
  [[ -n "$id" ]] && break
  if (( SECONDS >= deadline )); then
    echo "NO_RUN"
    exit 2
  fi
  sleep 10
done

if gh run watch "$id" --exit-status --interval 15 > /dev/null 2>&1; then
  echo "PASS $id"
else
  echo "FAIL $id"
  gh run view "$id" --log-failed | tail -n 300
  exit 1
fi
