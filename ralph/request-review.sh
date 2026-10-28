#!/usr/bin/env bash
# Ask claude.yml for a blind peer review of a PR's head commit.
#
#   ralph/request-review.sh <pr> [sha]    # sha defaults to the PR's head
#
# Posts the one request text the loop uses, from wait-review.sh and from the
# review-fix phase alike. A mention gets none of the pull_request prompt, so
# the request itself pins what wait-review.sh reads: the heading, the head
# commit (which the reviewer reads from GitHub, not from this comment) and the
# verdict line.
set -euo pipefail

pr="$1"
repo=jamescrowley321/codecite
sha="${2:-$(gh pr view "$pr" -R "$repo" --json headRefOid --jq .headRefOid)}"

gh pr comment "$pr" -R "$repo" --body "@claude review this PR at head \`$sha\`. Head the review \`### Blind peer review — PR #$pr\`, put head \`<sha>\` on the line below with the sha from \`gh pr view $pr --json headRefOid\`, and give the verdict on its own line as \`Verdict: **PASS**\` or \`Verdict: **BLOCK**\`." > /dev/null
echo "$sha"
