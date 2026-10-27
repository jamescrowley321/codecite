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
# claude.yml posts, its own on open or its edited progress comment on a
# mention (claude[bot]'s only: anyone can post the heading),
# "stale" when it doesn't name the head commit (head `<sha>`), "none" before
# the first. A stale PR gets a fresh review from an `@claude review this PR` comment.
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
  # The REST API names the bot claude[bot], a login no user can hold; gh pr
  # view drops the suffix. Only the newest review headed for this PR is kept,
  # so the jq argument stays small however many rounds the PR had.
  reviews=$(gh api "repos/{owner}/{repo}/issues/$n/comments" --paginate --jq '
    .[] | select(.user.login == "claude[bot]" and .user.type == "Bot"
                 and ((.body // "") | test("(^|\n)#{2,4} Blind peer review — PR #'"$n"'\\b"; "i")))
    | {body, createdAt: .created_at}' | jq -s 'max_by(.createdAt) // null')
  gh pr view "$n" --json number,title,body,mergeStateStatus,headRefOid,comments,labels | jq -r --argjson reviews "$reviews" '
    # capture() emits nothing on a miss, so each match is collected in an
    # array and defaulted; any list gh returns can be null.
    (.headRefOid // "") as $head
    | $reviews as $rev
    # Every line that is a verdict ("Verdict: **BLOCK**", "### Verdict:
    # **PASS**", "**Verdict:** PASS"), not a finding that quotes one;
    # "running" while a progress comment has none yet, "?" when there is none
    # or they disagree.
    | (if $rev == null then "none"
       else ([$rev.body | capture("(?im)^[#\\s]*\\**verdict\\**:?\\**\\s*\\**(?<v>PASS|BLOCK)\\b"; "g").v | ascii_upcase] | unique
             | if length == 1 then .[0]
               elif length == 0 and ($rev.body | test("^\\*\\*Claude is working")) then "running"
               else "?" end)
         # The author sets a commit date; the reviewed sha says what was read.
         + (([$rev.body | capture("(?i)\\bhead\\b[^`]{0,12}`(?<h>[0-9a-f]{7,40})`").h] | .[0]) as $named
            | if $named != null and ($head | startswith($named)) then "" else " stale" end)
       end) as $review
    | ([(.body // "") | capture("(?i)(closes|part of) #(?<i>[0-9]+)").i] | .[0]
        | if . == null then "-" else "#" + . end) as $issue
    | (if any((.comments // [])[]; (.body // "") | contains("codecite-eval-scorecard")) then "posted"
       elif any((.labels // [])[]; .name == "eval") then "label" else "-" end) as $eval
    | "#\(.number)\t\(.mergeStateStatus)\t\($review)\t\($issue)\t\($eval)\t\(.title)"' \
    | awk -F'\t' '{ printf "%-6s %-8s %-12s %-8s %-6s %s\n", $1, $2, $3, $4, $5, $6 }'
done
