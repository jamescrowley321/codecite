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
# heading inside once it is done. Either way it names the commit it reviewed
# (head `<sha>`) and gives a verdict, PASS or BLOCK; a comment with no verdict
# line yet is still running.
#
# A review belongs to the head commit only when it names that commit, and it
# counts only when it was posted after the last request for that commit: the
# claude.yml run started by opening the PR, or a comment
# `@claude review this PR at head `<sha>``. A push doesn't request a review,
# so when the head has no request yet this script posts one. To re-review a
# head that already has a review (fixes that pushed nothing), post the request
# comment again. Commit timestamps are never used: the author sets them.
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
me=$(gh api user --jq .login)
sha=$(gh pr view "$pr" -R "$repo" --json headRefOid --jq .headRefOid)

comments() { gh api "repos/$repo/issues/$pr/comments" --paginate --jq '.[]' | jq -s .; }

# When this head's review was last asked for: the newest of the claude.yml
# run that opening the PR started on it, and our request comments naming it.
since() {
  {
    gh run list -R "$repo" --workflow claude.yml --event pull_request --commit "$sha" \
      --json createdAt --jq '.[].createdAt'
    comments | jq -r --arg me "$me" --arg sha "$sha" '.[]
      | select(.user.login == $me and ((.body // "") | contains("@claude review this PR"))
               and ((.body // "") | contains($sha)))
      | .created_at'
  } | sort | tail -1
}

asked=$(since)
if [[ -z "$asked" ]]; then
  gh pr comment "$pr" -R "$repo" --body "@claude review this PR at head \`$sha\`. Name that commit in the review as head \`$sha\`." > /dev/null
  asked=$(since)
fi

sleep "${WAIT_SLEEP:-60}"

for _ in $(seq 1 8); do
  # Only claude[bot], a login no user can hold: anyone can post the heading.
  # The newest comment since the request that is, or will be, a review: one
  # with the heading, or a progress comment.
  review=$(comments | jq --arg since "$asked" '
    [.[] | select(.user.login == "claude[bot]" and .user.type == "Bot"
                  and ((.body // "") | test("(^|\n)#{2,4} Blind peer review|^\\*\\*Claude (is working|finished)"))
                  and .created_at > $since)]
    | max_by(.created_at) // empty')
  if [[ -n "$review" ]]; then
    body=$(jq -r .body <<< "$review")
    url=$(jq -r .html_url <<< "$review")
    # Every line that is a verdict, in any of the skill's spellings ("Verdict:
    # **BLOCK**", "### Verdict: **PASS**", "**Verdict: PASS**", "**Verdict:**
    # PASS"), must agree; a finding that quotes a verdict mid-line is not one.
    verdicts=$(grep -oiE '^[#[:space:]]*\**verdict\**:?\**[[:space:]]*\**(PASS|BLOCK)\b' <<< "$body" \
      | grep -oiE '(PASS|BLOCK)$' | tr a-z A-Z | sort -u || true)
    if [[ -z "$verdicts" ]]; then
      sleep "${WAIT_POLL:-60}"   # still running
      continue
    fi
    if ! grep -qE "^#{2,4} Blind peer review — PR #$pr\b" <<< "$body"; then
      echo "REVIEW_FAILED the newest review is headed for another PR, or not headed ($url)"
      exit 3
    fi
    # The first commit named as the head: "head `<sha>`", "Head: `<sha>`".
    named=$(grep -oiE '\bhead\b[^`]{0,12}`[0-9a-f]{7,40}`' <<< "$body" | head -1 | grep -oE '[0-9a-f]{7,40}' | tail -1 || true)
    if [[ -z "$named" ]]; then
      echo "REVIEW_FAILED the newest review names no head commit ($url)"
      exit 3
    fi
    if [[ "$sha" != "$named"* ]]; then
      echo "REVIEW_FAILED the newest review is of $named, not the head $sha ($url)"
      exit 3
    fi
    if [[ "$verdicts" != PASS && "$verdicts" != BLOCK ]]; then
      echo "REVIEW_FAILED the review has both a PASS and a BLOCK verdict line ($url)"
      exit 3
    fi
    printf '%s\n' "$body" > "$out"
    echo "FOUND $verdicts"
    exit 0
  fi
  sleep "${WAIT_POLL:-60}"
done

echo "WAITING"
exit 4
