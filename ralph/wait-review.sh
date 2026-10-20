#!/usr/bin/env bash
# Wait for the CI blind peer review (claude.yml) to post on a PR.
#
#   ralph/wait-review.sh <pr> <since UTC, e.g. 2026-10-20T03:00:00Z> <out file>
#
# Sleeps a minute first, then polls once a minute for up to 20 minutes for a
# comment by `claude` created after <since> that starts with "## Blind peer
# review". Prints one line:
#   FOUND <PASS|BLOCK|UNKNOWN>   exit 0   (the comment body is written to <out file>)
#   REVIEW_FAILED <reason>      exit 3   (the review run ended without posting)
#   WAITING                     exit 4   (still running; call again)
set -euo pipefail

pr="$1"
since="$2"
out="$3"
repo=jamescrowley321/codecite

sleep 60

for _ in $(seq 1 20); do
  body=$(gh pr view "$pr" -R "$repo" --json comments --jq \
    "[.comments[] | select(.author.login == \"claude\" and .createdAt > \"$since\"
      and (.body | ascii_downcase | startswith(\"## blind peer review\")))] | last | .body // empty")
  if [[ -n "$body" ]]; then
    printf '%s\n' "$body" > "$out"
    if grep -qE '\bBLOCK\b' <<< "$body"; then verdict=BLOCK
    elif grep -qE '\bPASS\b' <<< "$body"; then verdict=PASS
    else verdict=UNKNOWN; fi
    echo "FOUND $verdict"
    exit 0
  fi

  # A review run created after <since> that finished without posting has failed.
  # Skipped runs are the workflow's no-op runs (bot comments, non-@claude
  # comments) and don't count. A green run is not a review either: the action's
  # workflow-validation self-skip and a session stopped by permission denials
  # both report success. So a successful run with no comment two polls later
  # counts as failed.
  run=$(gh run list -R "$repo" --workflow claude.yml --limit 20 \
    --json databaseId,status,conclusion,createdAt --jq \
    "[.[] | select(.createdAt > \"$since\" and .conclusion != \"skipped\")] | first // empty")
  if [[ -n "$run" ]]; then
    id=$(jq -r .databaseId <<< "$run")
    status=$(jq -r .status <<< "$run")
    conclusion=$(jq -r .conclusion <<< "$run")
    if [[ "$status" == completed && "$conclusion" != success ]]; then
      reason=$(gh run view "$id" -R "$repo" --log-failed 2>/dev/null \
        | grep -oE '##\[error\].*' | head -n 1 || true)
      echo "REVIEW_FAILED run $id $conclusion: ${reason:-no error line in the log}"
      exit 3
    fi
    if [[ "$status" == completed ]]; then
      quiet_polls=$(( ${quiet_polls:-0} + 1 ))
      if (( quiet_polls > 2 )); then
        echo "REVIEW_FAILED run $id succeeded but posted no review (self-skip or permission denials)"
        exit 3
      fi
    fi
  fi
  sleep 60
done

echo "WAITING"
exit 4
