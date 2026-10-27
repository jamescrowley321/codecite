#!/usr/bin/env bash
# Show where each open PR stands: merge state, the verdict of its latest blind
# peer review, the issue it closes, and whether the eval scorecard is posted.
#
#   scripts/cloud-status.sh            # every open PR against main
#   scripts/cloud-status.sh 131 137    # just these PRs
#
# Merge states: CLEAN is ready to merge, BEHIND needs `gh pr update-branch <n>`,
# DIRTY has conflicts, BLOCKED is waiting on a required check, UNSTABLE has a
# failing check that isn't required.
# Review: PASS or BLOCK from the latest "Blind peer review" comment that
# claude.yml posts, "stale" when commits landed after it, "none" before the
# first. A stale PR gets a fresh review from an `@claude review this PR` comment.
# Session links from scripts/cloud-launch.sh are in
# ~/.local/state/codecite/cloud-sessions.log.
set -euo pipefail

if [[ $# -gt 0 ]]; then
  prs=("$@")
else
  mapfile -t prs < <(gh pr list --base main --state open --json number -q '.[].number' | sort -n)
fi

printf '%-6s %-8s %-12s %-8s %-6s %s\n' PR STATE REVIEW ISSUE EVAL TITLE
for n in "${prs[@]}"; do
  gh pr view "$n" --json number,title,body,mergeStateStatus,commits,comments,labels | jq -r '
    # capture() emits nothing on a miss, so each match is collected in an
    # array and defaulted; any list gh returns can be null.
    ((.commits // []) | .[-1].committedDate // "") as $pushed
    | ([(.comments // [])[] | select((.body // "") | startswith("## Blind peer review"))] | .[-1]) as $rev
    | (if $rev == null then "none"
       else ([$rev.body | capture("Verdict: \\*\\*(?<v>[A-Z]+)\\*\\*").v] | .[0] // "?")
         + (if $rev.createdAt < $pushed then " stale" else "" end)
       end) as $review
    | ([(.body // "") | capture("(?i)(closes|part of) #(?<i>[0-9]+)").i] | .[0]
        | if . == null then "-" else "#" + . end) as $issue
    | (if any((.comments // [])[]; (.body // "") | contains("codecite-eval-scorecard")) then "posted"
       elif any((.labels // [])[]; .name == "eval") then "label" else "-" end) as $eval
    | "#\(.number)\t\(.mergeStateStatus)\t\($review)\t\($issue)\t\($eval)\t\(.title)"' \
    | awk -F'\t' '{ printf "%-6s %-8s %-12s %-8s %-6s %s\n", $1, $2, $3, $4, $5, $6 }'
done
