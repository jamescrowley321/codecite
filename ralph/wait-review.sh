#!/usr/bin/env bash
# Wait for the claude.yml blind peer review of a PR's head commit.
#
#   ralph/wait-review.sh <pr> <out file>
#
# claude.yml reviews a PR when it opens, and again when an owner or
# collaborator comments `@claude review this PR`. On open, the review is its
# own PR comment headed "## Blind peer review — PR #<n>". On a mention, it is
# claude[bot]'s progress comment ("Claude is working…", later "Claude
# finished…"), edited in place, with a "### Blind peer review — PR #<n>"
# heading inside. Either way it names the commit it reviewed (head `<sha>`)
# and gives a verdict, PASS or BLOCK.
#
# A review counts only when it was posted after the last request for the
# head commit, and names that commit. A request is the claude.yml run that
# opening the PR started on it (unless that run failed or was skipped), or a
# comment from request-review.sh naming it. A push doesn't request a review,
# so when the head has no request yet this script posts one. To re-review a
# head that already has a review (fixes that pushed nothing), run
# request-review.sh again. Commit timestamps are never used: the author sets
# them. A review of an older head, landing after a newer head's request, is
# waited past.
#
# Sleeps a minute first, then polls for up to 8 minutes, so a call fits inside
# one foreground Bash call (10-minute cap). The review phase calls it again on
# WAITING. Prints one line:
#   FOUND <PASS|BLOCK>          exit 0   (the review comment is written to <out file>)
#   REVIEW_FAILED <reason>      exit 3   (no verdict will come for this commit)
#   WAITING                     exit 4   (still running; call again)
set -euo pipefail
trap 'echo "REVIEW_FAILED a gh or jq call failed (line $LINENO)"; exit 3' ERR

pr="$1"
out="$2"
repo=jamescrowley321/codecite
here=$(cd "$(dirname "$0")" && pwd)
me=$(gh api user --jq .login)
sha=$(gh pr view "$pr" -R "$repo" --json headRefOid --jq .headRefOid)

comments() { gh api "repos/$repo/issues/$pr/comments" --paginate --jq '.[]' | jq -s .; }

# When this head's review was last asked for: the newest of the claude.yml
# run that opening the PR started on it, still running or successful, and
# our request comments naming it.
# Either lookup failing fails since(), so a failed run lookup never reads as
# "no request" and posts one that cancels the opening review mid-run.
since() {
  local runs requests
  runs=$(gh run list -R "$repo" --workflow claude.yml --event pull_request --commit "$sha" \
    --json createdAt,status,conclusion) || return 1
  requests=$(comments) || return 1
  {
    jq -r '.[] | select(.status != "completed" or .conclusion == "success") | .createdAt' <<< "$runs"
    jq -r --arg me "$me" --arg sha "$sha" '.[]
      | select(.user.login == $me and ((.body // "") | contains("@claude review this PR"))
               and ((.body // "") | contains($sha)))
      | .created_at' <<< "$requests"
  } | sort | tail -1
}

if ! asked=$(since); then
  echo "REVIEW_FAILED could not read the PR's runs or comments"
  exit 3
fi
if [[ -z "$asked" ]]; then
  "$here/request-review.sh" "$pr" "$sha" > /dev/null
  # The new comment can take a moment to show in the listing.
  for _ in 1 2 3 4 5; do
    asked=$(since) || asked=""
    [[ -n "$asked" ]] && break
    sleep "${WAIT_RETRY:-5}"
  done
  if [[ -z "$asked" ]]; then
    echo "REVIEW_FAILED posted a review request but can't find it (is gh logged in as $me?)"
    exit 3
  fi
fi

sleep "${WAIT_SLEEP:-60}"
trap - ERR

for _ in $(seq 1 8); do
  # A gh error mid-poll is retried on the next poll.
  if ! all=$(comments); then
    sleep "${WAIT_POLL:-60}"
    continue
  fi
  # Only claude[bot], a login no user can hold: anyone can post the heading.
  # The newest comment since the request that carries the heading and doesn't
  # name another head: an unrelated mention's progress comment, or an older
  # head's review landing late, never hides this head's review.
  review=$(jq --arg since "$asked" --arg sha "$sha" '
    [.[] | select(.user.login == "claude[bot]" and .user.type == "Bot"
                  and ((.body // "") | test("(^|\n)#{2,4} Blind peer review"; "i"))
                  and .created_at > $since)
         | select(([(.body // "") | capture("(?i)\\bhead\\b[^`]{0,12}`(?<h>[0-9a-f]{7,40})`").h] | .[0]) as $named
                  | $named == null or ($sha | startswith($named)))]
    | max_by(.created_at) // empty' <<< "$all")
  if [[ -n "$review" ]]; then
    body=$(jq -r .body <<< "$review")
    url=$(jq -r .html_url <<< "$review")
    # Every line that is a verdict, in any of the skill's spellings ("Verdict:
    # **BLOCK**", "### Verdict: **PASS**", "**Verdict: PASS**", "**Verdict:**
    # PASS"), must agree; a finding that quotes a verdict mid-line is not one.
    verdicts=$(grep -oiE '^[#[:space:]]*\**verdict\**:?\**[[:space:]]*\**(PASS|BLOCK)\b' <<< "$body" \
      | grep -oiE '(PASS|BLOCK)$' | tr a-z A-Z | sort -u || true)
    # The first commit named as the head: "head `<sha>`", "Head: `<sha>`".
    named=$(grep -oiE '\bhead\b[^`]{0,12}`[0-9a-f]{7,40}`' <<< "$body" | head -1 | grep -oE '[0-9a-f]{7,40}' | tail -1 || true)
    if [[ -z "$verdicts" ]]; then
      if grep -qE '^\*\*Claude (finished|encountered an error)' <<< "$body"; then
        echo "REVIEW_FAILED the review ended without a verdict ($url)"
        exit 3
      fi
      :  # still running
    elif ! grep -qiE "^#{2,4} Blind peer review\W+PR #$pr\b" <<< "$body"; then
      echo "REVIEW_FAILED the newest review is headed for another PR, or names none ($url)"
      exit 3
    elif [[ -z "$named" ]]; then
      echo "REVIEW_FAILED the newest review names no head commit ($url)"
      exit 3
    elif [[ "$verdicts" != PASS && "$verdicts" != BLOCK ]]; then
      echo "REVIEW_FAILED the review has both a PASS and a BLOCK verdict line ($url)"
      exit 3
    else
      printf '%s\n' "$body" > "$out"
      echo "FOUND $verdicts"
      exit 0
    fi
  fi
  sleep "${WAIT_POLL:-60}"
done

echo "WAITING"
exit 4
