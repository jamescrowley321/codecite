# Phase: review-fix

`cd <worktree>`. Read `.claude/review/round-<review_round>.md`.

1. Every MUST FIX finding: fix it, or rule against it with a concrete reason the code stands. Fix test-first: write the test that reproduces it, watch it fail, fix, watch it pass. SHOULD FIX findings: fix the ones that are cheap and clearly right; leave the rest. Nitpicks: leave.
2. Run the router's gates. Commit with `fix: address review findings (round <review_round>)` and push.
3. Post one PR comment headed `## Review round <review_round>: response`, with one line per MUST FIX and SHOULD FIX finding: fixed in `<sha>`, or the ruling and its reason, or left for the owner. Never write `@claude` in this comment: any mention starts a separate Claude Code run.
4. If `review_round` is below 3: increment `review_round`, reset `review_waits` and `ci_waits`, set `phase: ci`. End. The push itself re-runs the CI review on the new commit; nothing needs requesting.
5. If `review_round` is 3 (two re-reviews already ran): post a PR comment saying the review still says BLOCK after two re-reviews and the remaining findings are left for the owner. Record `verdict: BLOCK` and set `phase: complete`. End. The PR stays open in the stack; never park an issue whose PR is open.
