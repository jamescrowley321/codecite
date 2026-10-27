#!/usr/bin/env bash
# Tests for ralph/wait-review.sh against a stub `gh`, using real review
# comments as fixtures: #145's review on open (review-145.md, which reviewed
# head 91d7628) and #149's review on a mention, an edited progress comment
# that names no head (review-149.md).
#
#   ralph/tests/wait-review.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
script="$here/../wait-review.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin"

# The stub answers the gh calls wait-review.sh makes from files in $work:
# head (the PR's head sha), runs (claude.yml run start times) and comments.json
# (the PR's comments). `gh pr comment` appends the new comment to
# comments.json and logs it to posted.
cat > "$work/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
  "api user") echo james ;;
  "pr view") cat "$STUB/head" ;;
  "run list") cat "$STUB/runs" ;;
  "pr comment")
    body=${*##*--body }
    echo "$body" >> "$STUB/posted"
    jq --arg b "$body" '. + [{user: {login: "james", type: "User"}, body: $b, created_at: "2026-10-27T05:00:00Z", html_url: "x"}]' \
      "$STUB/comments.json" > "$STUB/c.tmp" && mv "$STUB/c.tmp" "$STUB/comments.json" ;;
  api\ repos/*) jq -c '.[]' "$STUB/comments.json" ;;
  *) echo "stub gh: unexpected: $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$work/bin/gh"
export STUB="$work" PATH="$work/bin:$PATH" WAIT_SLEEP=0 WAIT_POLL=0

sha=91d7628aaaabbbbccccddddeeeeffff000011112
review=$(cat "$here/review-145.md")

# A comment: login, type, body, created_at.
comment() { jq -n --arg l "$1" --arg t "$2" --arg b "$3" --arg c "$4" '{user: {login: $l, type: $t}, body: $b, created_at: $c, html_url: "x"}'; }

setup() {
  echo "$sha" > "$work/head"
  printf '%s\n' "$@" | jq -s . > "$work/comments.json"
  : > "$work/runs" ; : > "$work/posted"
}

failed=0
check() {  # name, expected first line of output, expected exit code
  local got code
  set +e; got=$(timeout 5 bash "$script" 145 "$work/out.md" | head -1); code=${PIPESTATUS[0]}; set -e
  if [[ "$got" == "$2"* && "$code" == "$3" ]]; then
    echo "ok   $1"
  else
    echo "FAIL $1: got '$got' (exit $code), want '$2' (exit $3)"
    failed=1
  fi
}

# The review on open: the run started before the review was posted.
setup "$(comment 'claude[bot]' Bot "$review" 2026-10-27T03:48:53Z)"
echo 2026-10-27T03:40:00Z > "$work/runs"
check "a review after the opening run is found" "FOUND BLOCK" 0
grep -q '^## Blind peer review — PR #145' "$work/out.md" && echo "ok   the review is written to the out file" || { echo "FAIL out file"; failed=1; }
[[ ! -s "$work/posted" ]] && echo "ok   no request is posted when one exists" || { echo "FAIL posted a request"; failed=1; }

# A review of an older head: posted after this head's request, but it names 91d7628.
setup "$(comment james User "@claude review this PR at head \`deadbee000\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "$review" 2026-10-27T03:48:53Z)"
echo deadbee000 > "$work/head"
check "a review that names another head is refused" "REVIEW_FAILED the newest review is of 91d7628" 3

# No request yet for this head (a ci-phase push): the script asks for one.
setup "$(comment 'claude[bot]' Bot "$review" 2026-10-27T03:48:53Z)"
check "a head with no request waits for a new review" "WAITING" 4
grep -q "@claude review this PR at head \`$sha\`" "$work/posted" && echo "ok   it posts the request" || { echo "FAIL no request posted"; failed=1; }

# A re-request for the same head: last round's review no longer counts.
setup "$(comment 'claude[bot]' Bot "$review" 2026-10-27T03:48:53Z)" \
      "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T04:00:00Z)"
check "a review older than the latest request is ignored" "WAITING" 4

# Anyone can post the heading; only claude[bot] counts.
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment claude User "${review//BLOCK/PASS}" 2026-10-27T03:50:00Z)"
check "a review from a user named claude is ignored" "WAITING" 4

# The verdict spellings, and a finding that quotes one.
for spelling in 'Verdict: **PASS**' '### Verdict: **PASS**' '**Verdict:** PASS'; do
  body=$(printf '## Blind peer review — PR #145\n\nthrough head `91d7628`\n\n- a finding quoting "Verdict: **BLOCK**" mid-line\n\n%s\n' "$spelling")
  setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
        "$(comment 'claude[bot]' Bot "$body" 2026-10-27T03:50:00Z)"
  check "verdict spelled '$spelling'" "FOUND PASS" 0
done

# Lines that disagree, and no head named.
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "$(printf '## Blind peer review — PR #145\nhead `91d7628`\nVerdict: **PASS**\nVerdict: **BLOCK**\n')" 2026-10-27T03:50:00Z)"
check "PASS and BLOCK lines together are refused" "REVIEW_FAILED the review has both" 3
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "$(printf '## Blind peer review — PR #145\nVerdict: **PASS**\n')" 2026-10-27T03:50:00Z)"
check "a review that names no head is refused" "REVIEW_FAILED the newest review names no head" 3
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "${review/PR #145/PR #146}" 2026-10-27T03:50:00Z)"
check "a review headed for another PR is refused" "REVIEW_FAILED the newest review is headed for another PR" 3

# The head spelled the other way the skill writes it.
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "$(printf '## Blind peer review — PR #145\n\nHead: `%s`, compared with `origin/main`.\n\nVerdict: **PASS**\n' "$sha")" 2026-10-27T03:50:00Z)"
check "a head spelled 'Head: \`<sha>\`' is read" "FOUND PASS" 0

# A review on a mention: claude[bot]'s progress comment, edited in place.
progress="**Claude is working…** —— [View job](x)"
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "$progress" 2026-10-27T03:00:05Z)"
check "a progress comment with no verdict yet is still running" "WAITING" 4
mention=$(sed 's/PR #149/PR #145/' "$here/review-149.md")
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "$mention" 2026-10-27T03:00:05Z)"
check "a finished mention review that names no head is refused" "REVIEW_FAILED the newest review names no head" 3
setup "$(comment james User "@claude review this PR at head \`$sha\`" 2026-10-27T03:00:00Z)" \
      "$(comment 'claude[bot]' Bot "$(printf '%s\n\nhead `%s`\n' "$mention" "${sha:0:7}")" 2026-10-27T03:00:05Z)"
check "a finished mention review that names the head is found" "FOUND PASS" 0

exit "$failed"
