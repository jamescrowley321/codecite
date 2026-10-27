#!/usr/bin/env bash
# Show where each open PR stands: merge state, the Merge Gate on its head, the
# lenses still requesting changes there, and the issue it closes.
#
#   scripts/cloud-status.sh            # every open PR against main
#   scripts/cloud-status.sh 131 137    # just these PRs
#
# Merge states: CLEAN is ready to merge, BEHIND needs `gh pr update-branch <n>`,
# DIRTY has conflicts, BLOCKED is waiting on a required check.
# Session links from scripts/cloud-launch.sh are in
# ~/.local/state/codecite/cloud-sessions.log.
set -euo pipefail

if [[ $# -gt 0 ]]; then
  prs=("$@")
else
  mapfile -t prs < <(gh pr list --base main --state open --json number -q '.[].number' | sort -n)
fi

printf '%-6s %-8s %-10s %-8s %-6s %s\n' PR STATE GATE ISSUE EVAL "CHANGES REQUESTED / TITLE"
for n in "${prs[@]}"; do
  gh pr view "$n" --json number,title,body,mergeStateStatus,headRefOid,reviews,statusCheckRollup,comments,labels | jq -r '
    # capture() emits nothing on a miss, so each match is collected in an
    # array and defaulted; any list gh returns can be null.
    .headRefOid as $h
    | ([(.statusCheckRollup // [])[] | select(.name == "Merge Gate")] | sort_by(.startedAt // "") | last
        | if . == null then "pending" else (.conclusion // .status) end) as $gate
    | ([(.reviews // [])[] | select(.commit.oid == $h and .state == "CHANGES_REQUESTED")
        | ([(.body // "") | capture("## (?<l>[^\n]+)").l] | first // "?")] | unique | join(", ")) as $cr
    | ([(.body // "") | capture("(?i)(closes|part of) #(?<i>[0-9]+)").i] | first
        | if . == null then "-" else "#" + . end) as $issue
    | (if any((.comments // [])[]; (.body // "") | contains("codecite-eval-scorecard")) then "posted"
       elif any((.labels // [])[]; .name == "eval") then "label" else "-" end) as $eval
    | "#\(.number)\t\(.mergeStateStatus)\t\($gate)\t\($issue)\t\($eval)\t\(if $cr == "" then .title else "[" + $cr + "] " + .title end)"' \
    | awk -F'\t' '{ printf "%-6s %-8s %-10s %-8s %-6s %s\n", $1, $2, $3, $4, $5, $6 }'
done
