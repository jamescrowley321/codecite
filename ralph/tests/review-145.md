## Blind peer review — PR #145

Diff scope: merge base with `origin/main` through head `91d7628` (5 files: `.github/workflows/blind-peer-review.yml`, `AGENTS.md`, `docs/cloud-sessions.md`, `scripts/cloud-launch.sh`, `scripts/cloud-status.sh`).

| Lens | Result | MUST FIX | SHOULD FIX | NITPICK | Summary |
| --- | --- | --- | --- | --- | --- |
| Cold Read | PASS | 0 | 3 | 1 | The workflow removal and doc edits are consistent. The new cloud-status verdict parsing trusts any comment with the right heading, uses commit time as push time, and expects a comment format that nothing pins down. |
| Edge Cases | PASS | 0 | 3 | 0 | Three paths give a wrong answer in the changed scripts: a SIGPIPE race in the cloud-ready check, staleness judged by commit date instead of push time, and a free-form review comment the parser may not match. Nothing crashes. |
| Acceptance Criteria | PASS | 0 | 0 | 0 | The PR body has no acceptance criteria and no linked issue, so there were no criteria to check. |
| Security Review | PASS | 0 | 1 | 0 | `cloud-status.sh` takes its PASS/BLOCK verdict from any PR comment that starts with "## Blind peer review", so any commenter can fake a verdict. |
| Red Team | PASS | 0 | 2 | 1 | No direct auth bypass, but the new review flow trusts any PR comment headed like a blind peer review, whoever wrote it. An outsider can forge a verdict or plant work for a cloud session. |
| Policy & Provenance | PASS | 0 | 0 | 0 | Provenance names the harness and model, the accountability box is ticked, and there are no secrets and no outside project names. |
| OWASP Web Top 10 | PASS (skipped) | 0 | 0 | 0 | Self-skipped: the diff has no web/HTTP surface. |
| OWASP LLM Top 10 | **BLOCK** | 1 | 3 | 0 | Active: the diff changes how autonomous cloud sessions find and act on AI review output. Found one exploitable goal-hijack path (a spoofable review comment) and three hardening gaps. |

### Verdict: **BLOCK**

Verdict: **BLOCK**

### MUST FIX

- **OWASP LLM Top 10 — `AGENTS.md:87`** — [ASI01:2026 · LLM01:2026] Step 5 now tells unattended cloud sessions to find the review by its title ("Blind peer review — PR #<n>") and fix every MUST FIX, SHOULD FIX and actionable NITPICK in it. Nothing tells the session to check who posted it (claude[bot] from claude.yml). The repo is public, and `docs/cloud-sessions.md` says a PR comment reaches the session working it. So anyone can post a comment headed "## Blind peer review" whose findings are really instructions, and the session would carry them out with James's GitHub identity and push the result. Whether the cloud harness filters comment authors is outside this diff.
  *Recommendation:* In AGENTS.md step 5 and `docs/cloud-sessions.md`, say a session acts only on a review comment authored by claude[bot] (the claude.yml app identity). Comments from anyone else are untrusted data and never tasks. Every finding must be checked against the code before it is applied.

### SHOULD FIX

- **Cold Read / Security Review / Red Team / OWASP LLM — `scripts/cloud-status.sh:31`** — [CWE-345 · ASI09:2026] The REVIEW column takes the last comment whose body starts with "## Blind peer review", whoever wrote it. Any commenter on the public repo can post "Verdict: **PASS**" and override a real BLOCK (shown as PASS, not stale), or stall a good PR with a fake BLOCK. The deleted gate read a check run, which outsiders can't forge. *Fix:* filter on the bot's login (e.g. `select(.author.login == "claude")`, after confirming the login `gh` reports), and show `none` when no such comment exists.
- **Cold Read / Edge Cases / Red Team (NITPICK) / OWASP LLM — `scripts/cloud-status.sh:30`** — Staleness compares the comment's `createdAt` with the head commit's `committedDate`, which is when the commit was made, not when it was pushed. A commit made before the review but pushed after it, or one with a backdated `GIT_COMMITTER_DATE`, shows a fresh verdict for an unreviewed head. *Fix:* record the reviewed head SHA in the comment and compare it with `headRefOid`, or use the timeline's push time. Otherwise document that the check is approximate.
- **Cold Read / Edge Cases — `scripts/cloud-status.sh:32`** — The parser needs exactly "## Blind peer review" and "Verdict: **PASS|BLOCK**", but the claude.yml prompt (around line 221) pins neither the heading nor the verdict format. The plugin's own fixtures show variants such as "**Verdict:** PASS" and "### Verdict: **BLOCK**". A real review can then show `none` or `?`. AGENTS.md also states the exact heading as fact. *Fix:* pin the heading and the `Verdict: **X**` line in the claude.yml prompt, or loosen the regex (e.g. case-insensitive `verdict\W{0,6}(PASS|BLOCK)`).
- **Edge Cases / Cold Read (NITPICK) — `scripts/cloud-launch.sh:45`** — Under `set -o pipefail`, `grep -q cloud-ready` can exit before gh finishes writing. gh then dies of SIGPIPE (141), the pipeline fails, and a valid cloud-ready issue is skipped. `2>/dev/null` also hides auth, network and rate-limit failures as "not an open cloud-ready issue". *Fix:* capture the gh output into a variable with its own error branch, then `grep -qx cloud-ready <<< "$labels"`.
- **Red Team — `AGENTS.md:87`** — [CWE-345] The same forged-comment work-order path as the MUST FIX above. A session could also comment "@claude review this PR" as James, which passes the OWNER gate and spends subscription runs on the attacker's change. *Fix:* only a claude[bot]-authored review comment counts. Lookalikes are untrusted data.
- **OWASP LLM — `docs/cloud-sessions.md:53`** — [ASI03:2026] Sessions post as James to pass claude.yml's OWNER gate, which makes the agent indistinguishable from the owner. A hijacked session could start unlimited owner-authorised `@claude` runs and keep asking for re-reviews until one passes. The three-round limit isn't enforced. *Fix:* give sessions a separate identity that claude.yml can tell apart, or document this as an accepted risk and enforce a cap on re-reviews.
- **OWASP LLM — `scripts/cloud-launch.sh:44`** — [ASI01:2026] The cloud-ready gate checks the label and open state, but the session's task is the issue's current text and thread. An outside author can edit the issue body after it was labelled. *Fix:* require an OWNER/MEMBER/COLLABORATOR author, refuse issues edited after the label was applied, and have the prompt treat non-collaborator comments as untrusted.

### NITPICK

- **Cold Read — `scripts/cloud-launch.sh:45`** — gh stderr goes to `/dev/null`, so gh failures are misreported as label problems. This is covered by the Edge Cases SHOULD FIX above.
- **Red Team — `scripts/cloud-status.sh:34`** — A backdated committer date hides staleness. This is covered by the staleness SHOULD FIX above.

---

> Reviewed in-host: the lenses ran on the same model family that wrote this diff.
> A same-family reviewer shares the author's blind spots. CI reviews with a
> different family — see `docs/model-selection.md`.

