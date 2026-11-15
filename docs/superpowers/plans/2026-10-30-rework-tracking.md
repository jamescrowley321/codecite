# Rework tracking from blind peer review findings (#179)

**Goal:** an observational table, never a gate, of the rework real PRs needed
in the owner's repositories: the blind peer review findings that are spec or
convention violations, and the review rounds before a PR passed. PRs whose
plans carry codecite citations are compared with PRs whose plans don't.

**Done when:** `experiments/rework/` holds a script that collects those counts
from the PRs' review comments, with the first table committed beside it.

**Architecture:** one stdlib-only Python script, `experiments/rework/rework.py`,
in three layers kept apart so the parsing can be tested without the network:

1. `fetch` — `gh pr list --state merged` per repository, cached as raw JSON in
   a cache directory (default `/var/tmp/codecite-rework/`), never committed.
2. `parse` — pure functions over comment bodies and plan texts.
3. `report` — pure aggregation into a Markdown table.

Tests are `experiments/rework/test_rework.py`, run with
`python3 -m unittest experiments/rework/test_rework.py`, over small fixture
bodies copied from real review comments in `experiments/rework/fixtures/`.

**Repositories:** the five history-benchmark repositories (blind-peer-review,
codecite, identity-model, identity-stack, terraform-provider-descope), all
public and the owner's own. No other repository is named or committed.

**Definitions** (DEC-012 in the loop's decision journal; restated in the
README so the table can be read alone):

- *Review comment:* a PR issue comment or review body whose first heading
  matches `## Blind peer review` (case-insensitive). Two formats exist:
  - *combined* — one comment for all lenses (`## Blind peer review — PR #N`,
    or `(local, …)`), a lens table and one `Verdict:` line;
  - *per-lens* — one comment per lens (`## Blind peer review — <Lens>` or
    `: <Lens>`), each with `**Verdict: X** (n finding(s), m MUST FIX)` or
    `**Verdict:** X (…)`.
- *Round:* one combined comment, or one run of per-lens comments, ordered by
  `createdAt`; a per-lens run ends when a lens repeats. A round passes when
  every lens in it is PASS (or its combined verdict is PASS).
- *Rounds to pass:* the 1-based index of the first passing round; a PR whose
  rounds never pass is counted as "never passed" and left out of the mean.
- *Spec violation:* a MUST FIX or SHOULD FIX finding from the Acceptance
  Criteria lens. *Convention violation:* the same from the Policy &
  Provenance lens. NITPICKs are never counted. Where a format gives only a
  lens's total finding count, that total is used and the README says so.
- *Plan:* a Markdown file the PR adds or modifies under a `plans/` directory.
- *Cited plan:* a plan whose text at the PR's head commit holds at least one
  codecite citation: `name@<hex 7–64>:path#L1-L9`,
  `name@sha256:<hex>#L1-L9` or `…#section-4.3` (forms in `docs/usage.md`).

Every PR falls in one group: *cited plan*, *uncited plan* or *no plan*.

---

## Task 1: parse review comments

**Files:** `experiments/rework/rework.py`, `experiments/rework/test_rework.py`,
`experiments/rework/fixtures/{combined.md,combined-local.md,per-lens-*.md}`.

1. Copy fixtures from real comments: codecite PR #197's combined comment, an
   identity-model `(local, codex)` comment, and per-lens comments in both the
   `— Lens` / `**Verdict: X**` and `: Lens` / `**Verdict:** X` spellings,
   including one with findings.
2. Write failing tests:
   - `is_review(body)` true for each fixture, false for an unrelated comment
     (a pre-release notice).
   - `parse_review(body)` returns a list of `LensResult(lens, verdict, must,
     should, nit, total)`: one per table row for combined comments (severity
     columns when the table has them, else `total` from `(N findings)`), one
     for a per-lens comment.
3. Implement with `re` only; `None` for a severity a format doesn't give.
4. `python3 -m unittest experiments/rework/test_rework.py` passes. Commit
   `feat(experiments): parse blind peer review comments (#179)`.

## Task 2: rounds and violation counts

1. Failing tests for `rounds(reviews)`, input `(createdAt, [LensResult])`
   pairs: a combined BLOCK then PASS is two rounds, pass at 2; eight per-lens
   comments then a repeat of Cold Read starts round 2; a PR with only BLOCK
   rounds gives `None`.
2. Failing tests for `violations(rounds)`: sums MUST+SHOULD (or `total` when
   both are `None`) for Acceptance Criteria (spec) and Policy & Provenance
   (convention) across all rounds, plus the same sum over every lens.
3. Implement; tests pass. Commit
   `feat(experiments): count review rounds and spec findings (#179)`.

## Task 3: plans and citations

1. Failing tests: `plan_paths(files)` keeps `docs/superpowers/plans/x.md` and
   `docs/plans/y.md`, drops `docs/design.md` and `plans/x.txt`;
   `has_citation(text)` is true for `codecite@56fc2dd:crates/a.rs#L10-L42`,
   `notes@sha256:1a2b3c4d5e6f#L3-L9` and
   `rfc9449@sha256:2d8657f07291#section-4.3`, false for an e-mail address,
   `user@host:path` and a bare `#L10`.
2. Implement. Plan text is fetched with
   `gh api repos/<owner>/<repo>/contents/<path>?ref=<headRefOid>` and the raw
   media type; a 404 (head gone) counts the plan as unreadable, reported in a
   column, not guessed.
3. Tests pass. Commit `feat(experiments): find codecite citations in plans (#179)`.

## Task 4: collect and report

1. `fetch(repo, cache)` runs `gh pr list --repo <repo> --state merged
   --limit 1000 --json number,title,headRefOid,files,comments,reviews` once
   per repository into the cache; `--refresh` refetches.
2. `report(rows)` writes one Markdown table: per repository and group, the
   PRs, the PRs with a review, mean rounds to pass, never passed, spec and
   convention findings per reviewed PR, and all MUST+SHOULD findings per
   reviewed PR; then a pooled row per group. A test over three synthetic
   rows checks the means and the pooled row.
3. `main()`: `python3 experiments/rework/rework.py [--refresh] [--cache DIR]
   [repo …]`, defaulting to the five repositories; prints the table to stdout.
4. Run it; spot-check two PRs by hand against their comments (one combined,
   one per-lens) and one cited plan. Try an adversarial path: an unknown
   repository name gives a clear error and no table.
5. Commit `feat(experiments): collect rework from merged PRs (#179)`.

## Task 5: first table and docs

1. Write `experiments/rework/README.md`: what it measures, the definitions
   above, that it is observational and never a gate, how to rerun, and the
   first table pasted under `## Results` with the run date, plus a short
   reading of it (small groups say so; no claim beyond the counts).
2. Check `docs/design.md` and `evals/README.md` for any text this
   contradicts or should point at (the "less rework" success measure), and
   add one sentence pointing at `experiments/rework/` if design.md has a
   place for it.
3. Run the gates (Rust gates unchanged by this, run anyway) and the
   unittest. Commit `docs: the first rework table (#179)`.

No history benchmark: this changes no indexing, search, ranking or CLI
output.
