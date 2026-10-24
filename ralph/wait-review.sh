#!/usr/bin/env bash
# Wait for the CI blind peer review (blind-peer-review.yml) on a PR's head commit.
#
#   ralph/wait-review.sh <pr> <out file>
#
# The lenses run on every push to a PR that targets main, and each posts a PR
# review. The workflow's Merge Gate job adjudicates them: it passes, or it
# fails on a MUST FIX. This script reads that verdict for the PR's current head
# commit, so a push re-reviews without any request.
#
# Sleeps a minute first, then polls for up to 8 minutes, so a call fits inside
# one foreground Bash call (10-minute cap). The review phase calls it again on
# WAITING. Prints one line:
#   FOUND <PASS|BLOCK>          exit 0   (the lens reviews are written to <out file>)
#   REVIEW_FAILED <reason>      exit 3   (no CI verdict will come for this commit)
#   WAITING                     exit 4   (still running; call again)
set -euo pipefail

pr="$1"
out="$2"
repo=jamescrowley321/codecite

read -r base sha < <(gh pr view "$pr" -R "$repo" --json baseRefName,headRefOid \
  --jq '"\(.baseRefName) \(.headRefOid)"')

# The workflow only runs on PRs to main; a stacked PR gets no CI review.
if [[ "$base" != main ]]; then
  echo "REVIEW_FAILED no CI review: the lenses run only on PRs to main, and this one targets $base"
  exit 3
fi

sleep "${WAIT_SLEEP:-60}"

for _ in $(seq 1 8); do
  # The newest lens run on this commit; a description edit starts another one
  # and cancels the old.
  run=$(gh run list -R "$repo" --workflow blind-peer-review.yml --commit "$sha" --limit 5 \
    --json databaseId,status,conclusion,createdAt --jq 'max_by(.createdAt) // empty')
  if [[ -n "$run" && $(jq -r .status <<< "$run") == completed ]]; then
    id=$(jq -r .databaseId <<< "$run")
    gate=$(gh run view "$id" -R "$repo" --json jobs --jq '.jobs[] | select(.name == "Merge Gate")')
    # Which gate step failed tells a BLOCK from a review that never finished.
    step() { jq -r --arg n "$1" '.steps[] | select(.name == $n) | .conclusion' <<< "$gate"; }
    if [[ $(jq -r .conclusion <<< "$run") == cancelled ]]; then
      :  # superseded; the newer run shows up on a later poll
    elif [[ -z "$gate" || $(jq -r .conclusion <<< "$gate") == skipped ]]; then
      echo "REVIEW_FAILED run $id ran no Merge Gate (lenses disabled by RUN_LENSES?)"
      exit 3
    elif [[ $(step "Fail closed when prerequisites did not succeed") == failure ]]; then
      echo "REVIEW_FAILED run $id: a lens or its preflight did not finish (see the run)"
      exit 3
    elif [[ $(step "Adjudicate lenses") == success ]]; then
      verdict=PASS
    elif [[ $(step "Adjudicate lenses") == failure ]]; then
      verdict=BLOCK
    else
      echo "REVIEW_FAILED run $id: the Merge Gate did not adjudicate (exempt PR?)"
      exit 3
    fi
    if [[ -n "${verdict:-}" ]]; then
      # Every lens review on this commit, as the review-fix phase reads them.
      gh api "repos/$repo/pulls/$pr/reviews" --paginate --jq \
        ".[] | select(.commit_id == \"$sha\" and ((.body // \"\") | contains(\"blind-peer-review:rendered-v1\")))
         | \"\(.body)\n\n---\n\"" \
        | python3 -I -c 'import html, sys; sys.stdout.write(html.unescape(sys.stdin.read()))' > "$out"
      echo "FOUND $verdict"
      exit 0
    fi
  fi
  sleep 60
done

echo "WAITING"
exit 4
