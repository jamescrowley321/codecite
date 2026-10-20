#!/usr/bin/env bash
# Wait for the CI blind peer review (claude.yml) to post on a PR.
#
#   ralph/wait-review.sh <pr> <since UTC, e.g. 2026-10-20T03:00:00Z> <out file>
#
# Sleeps a minute first, then polls once a minute for up to 7 minutes, so a
# call fits inside one foreground Bash call (10-minute cap). The review phase
# calls it again on WAITING. Only
# signals from this PR count: its comments, and the pull_request runs on its
# head branch. Prints one line:
#   FOUND <PASS|BLOCK|UNKNOWN>   exit 0   (the comment body is written to <out file>)
#   REVIEW_FAILED <reason>      exit 3   (the review ended without a verdict)
#   WAITING                     exit 4   (still running; call again)
#
# Where the review lands depends on how it was started:
# - PR opened (agent mode): its own comment, starting "## Blind peer review".
# - "@claude review this PR" (tag mode): inside Claude's tracking comment,
#   which starts "**Claude finished ...**" and has the review heading below.
# Either way the verdict is read from the "Verdict" line only, because the
# rest of a review can mention PASS or BLOCK in passing.
set -euo pipefail

pr="$1"
since="$2"
out="$3"
repo=jamescrowley321/codecite

branch=$(gh pr view "$pr" -R "$repo" --json headRefName --jq .headRefName)

sleep 60

# Prints the verdict from a review body: the first word after "Verdict" on a
# heading or bold line ("### Verdict: **BLOCK**", "**Verdict: none. ..."),
# so a table header or a sentence that mentions PASS or BLOCK is never read.
verdict_of() {
  local line word
  line=$(grep -m 1 -iE '^ *(#+ *|\*\*)verdict' <<< "$1" \
    || grep -m 1 -iE 'verdict *:' <<< "$1" || true)
  word=$(grep -oiE 'verdict[^a-z]*[a-z]+' <<< "$line" | head -n 1 | grep -oiE '[a-z]+$' || true)
  case "${word^^}" in
    BLOCK) echo BLOCK ;;
    PASS) echo PASS ;;
    NONE) echo NONE ;;
    *) echo UNKNOWN ;;
  esac
}

quiet_polls=0
for _ in $(seq 1 7); do
  # This PR's comments from Claude since the request, newest last.
  comments=$(gh pr view "$pr" -R "$repo" --json comments --jq \
    "[.comments[] | select(.author.login == \"claude\" and .createdAt > \"$since\") | .body]")

  # A review heading at the start of any line, in either posting mode.
  review=$(jq -r '[.[] | select(test("(^|\n)#{2,3} *blind peer review"; "i"))] | last // empty' <<< "$comments")
  if [[ -n "$review" ]]; then
    verdict=$(verdict_of "$review")
    if [[ "$verdict" == NONE ]]; then
      echo "REVIEW_FAILED the review posted but did not run (see the PR comment)"
      exit 3
    fi
    # A tag-mode tracking comment is edited in place while Claude works. Only
    # accept it once it says it finished.
    if ! grep -qE '^\*\*Claude (is working|is thinking)' <<< "$review"; then
      printf '%s\n' "$review" > "$out"
      echo "FOUND $verdict"
      exit 0
    fi
  fi

  # A tag-mode run that finished or errored without a review heading.
  done_no_review=$(jq -r '[.[] | select(test("^\\*\\*Claude (finished|encountered an error)"))] | last // empty' <<< "$comments")
  if [[ -n "$done_no_review" && -z "$review" ]]; then
    reason=$(grep -m 1 -iE 'error|denied|not run' <<< "$done_no_review" || echo "no review in the reply")
    echo "REVIEW_FAILED tag-mode run finished without a review: $reason"
    exit 3
  fi

  # The PR-open run (agent mode) for this PR's branch. A green run is not a
  # review: the workflow-validation self-skip and permission denials both
  # report success, so success with no comment two polls later is a failure.
  run=$(gh run list -R "$repo" --workflow claude.yml --branch "$branch" --event pull_request --limit 5 \
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
      quiet_polls=$((quiet_polls + 1))
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
