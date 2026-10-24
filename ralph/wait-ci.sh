#!/usr/bin/env bash
# Wait for every CI workflow on one commit and report how it went.
#
#   ralph/wait-ci.sh <branch> [sha]     # sha defaults to HEAD
#
# Covers every workflow that runs on the commit (CI, Security, CodeQL,
# OSV-Scanner, Dependency Review, ...) except the review workflows, which are
# the review phase's concern. main's required checks live across all of them.
#
# Sleeps a minute first so GitHub has registered the push, then waits for the
# runs for exactly this commit, so a stale result from an earlier push is never
# read. Polls for up to 8 minutes so a call fits inside one foreground Bash
# call (10-minute cap); CodeQL can take longer, so the ci phase calls again on
# WAITING. Prints one line per workflow, then one verdict line, then the failed
# runs' logs on failure:
#   PASS            exit 0
#   FAIL            exit 1
#   NO_RUN          exit 2   (no CI run appeared within 3 minutes of the sleep)
#   WAITING         exit 4   (still running; call again)
set -euo pipefail

branch="$1"
sha="${2:-$(git rev-parse HEAD)}"
repo=jamescrowley321/codecite
# Advisory review workflows: never a CI failure.
skip='["Blind Peer Review", "Claude Code"]'

sleep "${WAIT_SLEEP:-60}"

# The newest run of each workflow on this commit, review workflows left out.
runs_json() {
  gh run list -R "$repo" --branch "$branch" --commit "$sha" --limit 50 \
    --json databaseId,workflowName,status,conclusion,createdAt --jq "
      [.[] | select(.workflowName as \$w | $skip | index(\$w) | not)]
      | group_by(.workflowName) | map(max_by(.createdAt))"
}

deadline=$((SECONDS + 180))
while :; do
  runs=$(runs_json)
  jq -e 'any(.[]; .workflowName == "CI")' <<< "$runs" > /dev/null && break
  if (( SECONDS >= deadline )); then
    echo "NO_RUN"
    exit 2
  fi
  sleep 10
done

deadline=$((SECONDS + 420))
while :; do
  runs=$(runs_json)
  failed=$(jq -r '.[] | select(.status == "completed" and
    (.conclusion | IN("success", "neutral", "skipped") | not)) | .databaseId' <<< "$runs")
  pending=$(jq '[.[] | select(.status != "completed")] | length' <<< "$runs")
  if [[ -n "$failed" || "$pending" == 0 ]] || (( SECONDS >= deadline )); then
    break
  fi
  sleep 15
done

jq -r '.[] | "  \(.workflowName): \(if .status == "completed" then .conclusion else .status end) (\(.databaseId))"' <<< "$runs"

if [[ -n "$failed" ]]; then
  echo "FAIL"
  for id in $failed; do
    echo "--- failed run $id"
    # Drop the job/step/timestamp prefix and the post-step artifact upload,
    # which would otherwise fill the tail and hide the finding.
    gh run view "$id" -R "$repo" --log-failed \
      | sed -E 's/^[^\t]*\t[^\t]*\t[^ ]* //' \
      | { grep -vE 'artifact|Uploaded bytes|blob storage|SHA256 digest' || true; } \
      | tail -n 150
  done
  exit 1
fi
if [[ "$pending" != 0 ]]; then
  echo "WAITING"
  exit 4
fi
echo "PASS"
