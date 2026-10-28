**Claude finished @jamescrowley321's task in 1m 8s** —— [View job](https://github.com/jamescrowley321/codecite/actions/runs/40589620952)

---
### Blind peer review — PR #149

- [x] Gather context and read the diff
- [x] Run the blind peer review lenses (all 8, fresh parallel subagents, diff scoped against `origin/main`)
- [x] Post the per-lens table and verdict

| Lens | Result | MUST FIX | Summary |
| --- | --- | --- | --- |
| Cold Read | PASS | 0 | Adding the task to the group stops dispatches for different tasks from cancelling each other's pending runs. One SHOULD FIX, below. |
| Edge Cases | PASS | 0 | PR events fall back to `'pr'` and keep the grouping they had before. No new unhandled path. |
| Acceptance Criteria | PASS | 0 | The PR body has no acceptance criteria and no `Closes #n`, so there was nothing to check. |
| Security Review | PASS | 0 | No security findings. `inputs.task` is only used in the concurrency key. |
| Red Team | PASS | 0 | No exploitable path. The group key isn't run by a shell, and only users with write access can dispatch. |
| Policy & Provenance | PASS | 0 | The harness and model are named, an accountable human is named, the review box is ticked, and the diff adds no secrets. |
| OWASP Web Top 10 | PASS (self-skipped) | 0 | No web or HTTP surface. The diff only changes the concurrency key in the workflow. |
| OWASP LLM Top 10 | PASS (self-skipped) | 0 | No LLM or AI surface in this diff. |

**Verdict: PASS**

#### SHOULD FIX

- **Cold Read**, `.github/workflows/evals.yml:47`: the group key is now task + tier + arms. Two dispatches that match on those but have different `epochs` or `resume` (both are inputs, at lines 28 and 32) still share a group, so the second one cancels the first while it is still pending. For example, a fresh run queued behind a running one gets dropped when a `resume` dispatch for the same task comes in. This is the same lost-dispatch bug this PR fixes, triggered by other inputs. It was already there before this change and the PR doesn't make it worse, but the new comment still says "Dispatches queue rather than cancel", which is only true for the inputs in the key.
  *Recommendation:* add `-${{ inputs.epochs || 'default' }}-${{ inputs.resume || 'fresh' }}` to the group key, or reword the comment to say that a dispatch matching a pending one on task, tier and arms replaces it. [Fix this →](https://claude.ai/code?q=In%20.github%2Fworkflows%2Fevals.yml%20on%20branch%20fix%2Fevals-concurrency-per-task%2C%20the%20concurrency%20group%20(line%20~47)%20is%20keyed%20on%20task%2C%20tier%20and%20arms%20but%20not%20inputs.epochs%20or%20inputs.resume%2C%20so%20two%20dispatches%20differing%20only%20in%20those%20share%20a%20group%20and%20the%20later%20cancels%20the%20pending%20earlier%20one.%20Add%20%60-%24%7B%7B%20inputs.epochs%20%7C%7C%20'default'%20%7D%7D-%24%7B%7B%20inputs.resume%20%7C%7C%20'fresh'%20%7D%7D%60%20to%20the%20group%20key%20and%20update%20the%20comment%20above%20it.&repo=jamescrowley321/codecite)

> Reviewed in-host: the lenses ran on the same model family that wrote this diff.
> A same-family reviewer shares the author's blind spots. CI reviews with a
> different family — see `docs/model-selection.md`.
