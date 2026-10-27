#!/usr/bin/env bash
# Wait for the claude.yml blind peer review of a PR's head commit.
#
#   ralph/wait-review.sh <pr> <out file>
#
# claude.yml reviews a PR when it opens, and again when an owner or
# collaborator comments `@claude review this PR`. Each review posts one PR
# comment headed "## Blind peer review" with a verdict line, PASS or BLOCK.
# This script takes the newest such comment from claude[bot] posted after the
# head commit, so a review of an older commit is never read. A review that
# names its head (head `<sha>`) must name this one.
#
# Sleeps a minute first, then polls for up to 8 minutes, so a call fits inside
# one foreground Bash call (10-minute cap). The review phase calls it again on
# WAITING. Prints one line:
#   FOUND <PASS|BLOCK>          exit 0   (the review comment is written to <out file>)
#   REVIEW_FAILED <reason>      exit 3   (no verdict will come for this commit)
#   WAITING                     exit 4   (still running; call again)
set -euo pipefail

pr="$1"
out="$2"
repo=jamescrowley321/codecite

read -r sha committed < <(gh pr view "$pr" -R "$repo" --json headRefOid,commits \
  --jq '"\(.headRefOid) \(.commits[-1].committedDate)"')

sleep "${WAIT_SLEEP:-60}"

for _ in $(seq 1 8); do
  # Only claude[bot]: anyone can post a comment with the heading.
  review=$(gh api "repos/$repo/issues/$pr/comments" --paginate --jq "
    .[] | select(.user.login == \"claude[bot]\" and .user.type == \"Bot\"
                 and ((.body // \"\") | startswith(\"## Blind peer review\"))
                 and .created_at > \"$committed\")" | jq -s 'max_by(.created_at) // empty')
  if [[ -n "$review" ]]; then
    body=$(jq -r .body <<< "$review")
    named=$(grep -oE 'head `[0-9a-f]{7,40}`' <<< "$body" | head -1 | tr -d '`' | cut -d' ' -f2 || true)
    if [[ -z "$named" || "$sha" == "$named"* ]]; then
      # The first line that is a verdict, in any of the skill's spellings
      # ("Verdict: **BLOCK**", "### Verdict: **PASS**", "**Verdict:** PASS").
      # A finding that quotes a verdict mid-line is not one.
      verdict=$(grep -m1 -oiE '^[#[:space:]]*\**verdict\**:?\**[[:space:]]*\**(PASS|BLOCK)\b' <<< "$body" | grep -oiE '(PASS|BLOCK)$' | tr a-z A-Z || true)
      if [[ -z "$verdict" ]]; then
        echo "REVIEW_FAILED the review comment has no Verdict: line ($(jq -r .html_url <<< "$review"))"
        exit 3
      fi
      printf '%s\n' "$body" > "$out"
      echo "FOUND $verdict"
      exit 0
    fi
  fi
  sleep 60
done

echo "WAITING"
exit 4
