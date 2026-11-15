# Rework on real PRs

How much rework did merged PRs in the owner's repositories need, and do PRs
whose plans cite code with codecite need less of it (#179)? `rework.py` reads
every merged PR's blind peer review comments and counts two things: the
findings that are spec or convention violations, and the review rounds before
the PR passed. It splits the PRs by whether their plan carries codecite
citations.

This is observational and never a gate. The groups are not a controlled
experiment: which PRs had plans, and which plans cited code, follows from when
and how each was written, not from a random draw.

## What is counted

The repositories are the history benchmark's five, all public and the owner's
own.

- **Review comment:** a PR comment or review body with a `## Blind peer
  review` heading on a line of its own, from a bot or from the repository's
  owner, members or collaborators (anyone can comment on a public PR). The
  review posted when a PR opens starts with the heading; a re-review asked for
  with `@claude review this PR` is claude[bot]'s progress comment, edited in
  place, with the heading below its "Claude finished" line. A heading that
  gives no result (a review that didn't run) is counted on stderr, not as a
  round. Two formats exist. A *combined* comment covers every
  lens, with a lens table and one `Verdict:` line. A *per-lens* comment covers
  one lens (`## Blind peer review — <Lens>`), with its own verdict. Older
  comments use a few other layouts (a code-block listing, JSON, a bare
  `**PASS**`); `fixtures/` holds examples of each, copied from real comments, and
  the tests parse them all.
- **Round:** one combined comment, or one run of per-lens comments in time
  order, ending when a lens repeats. A round passes when its verdict, or every
  lens in it, is PASS.
- **Rounds to pass:** the 1-based index of the first passing round. A
  reviewed PR with no passing round is counted under *never passed* and left
  out of the mean: it was blocked and merged with no passing re-review on
  record.
- **Spec violation:** a MUST FIX or SHOULD FIX finding from the Acceptance
  Criteria lens. **Convention violation:** the same from the Policy &
  Provenance lens. NITPICKs never count. Where a comment gives only a lens's
  total number of findings, that total is used.
- **Plan:** a Markdown file the PR adds or changes under a `plans/` directory.
  A **cited plan** holds at least one codecite citation at the PR's head
  commit, in a form from `docs/usage.md` (`name@<sha>:path#L1-L9`,
  `name@sha256:<hex>#L1-L9` or `…#section-4.3`). Every PR is in one group:
  *cited plan*, *uncited plan* or *no plan*. A plan that can't be read at the
  head (a deleted branch, a force-pushed head) is counted under *unreadable
  plans*. Only a 404 counts: any other error from GitHub stops the run, so a
  rate limit never moves a PR between groups.

Each per-PR count is divided by the group's reviewed PRs, so a group's row
reads "findings per reviewed PR".

## Rerun

    python3 experiments/rework/rework.py            # all five repositories
    python3 experiments/rework/rework.py OWNER/REPO # any others
    python3 experiments/rework/rework.py --refresh  # refetch, ignoring the cache

It needs only `python3` and an authenticated `gh`. It pages each repository's
merged PRs through the GraphQL API, 25 at a time (larger pages time out on a
busy repository), and caches the raw responses and plan texts in
`$XDG_CACHE_HOME/codecite-rework/` (`~/.cache/codecite-rework/` by default;
`--cache` moves it), never in the repository. Each connection takes a PR's
first 100 files, comments and reviews; a PR with more is named on stderr. A
full fetch takes a few minutes; a rerun from the cache is instant.

Tests: `python3 -m unittest experiments/rework/test_rework.py`.

## Results

Run on 2026-10-30, every merged PR to that day.

| repository | group | PRs | reviewed | mean rounds to pass | never passed | spec / reviewed PR | convention / reviewed PR | all MUST+SHOULD / reviewed PR | unreadable plans |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| jamescrowley321/blind-peer-review | no plan | 134 | 0 | – | 0 | – | – | – | 0 |
| jamescrowley321/codecite | cited plan | 11 | 10 | 1.30 | 0 | 0.20 | 0.20 | 3.40 | 0 |
| jamescrowley321/codecite | uncited plan | 35 | 18 | 1.50 | 0 | 0.39 | 0.17 | 7.33 | 0 |
| jamescrowley321/codecite | no plan | 45 | 21 | 1.25 | 5 | 0.24 | 0.29 | 5.10 | 0 |
| jamescrowley321/identity-model | no plan | 452 | 52 | 1.00 | 33 | 0.10 | 1.29 | 2.27 | 0 |
| jamescrowley321/identity-stack | no plan | 206 | 4 | – | 4 | 0.25 | 1.00 | 6.75 | 0 |
| jamescrowley321/terraform-provider-descope | no plan | 121 | 3 | 1.00 | 2 | 0.00 | 0.67 | 1.00 | 0 |
| **all** | cited plan | 11 | 10 | 1.30 | 0 | 0.20 | 0.20 | 3.40 | 0 |
| **all** | uncited plan | 35 | 18 | 1.50 | 0 | 0.39 | 0.17 | 7.33 | 0 |
| **all** | no plan | 958 | 80 | 1.11 | 44 | 0.14 | 0.99 | 3.19 | 0 |

## Reading

Few PRs were reviewed at all: 108 of 1,004. blind-peer-review has no review
comments, identity-stack and terraform-provider-descope only a handful, and
only codecite has plans, so the cited and uncited groups are codecite alone,
with 10 and 18 reviewed PRs. Groups this small show a direction at most.

- Every reviewed codecite PR with a plan passed in the end. Cited-plan PRs took
  1.30 rounds on average against 1.50 for uncited plans; codecite PRs with no
  plan took 1.25, and 5 of their 21 never passed.
- Cited-plan PRs had fewer than half the findings of uncited ones: 3.40 MUST +
  SHOULD per reviewed PR against 7.33 (5.10 with no plan), and half the spec
  violations (0.20 against 0.39). Convention violations were the same (0.20
  against 0.17).
- Most of identity-model's 33 never-passed PRs were blocked once, mostly by
  the Policy & Provenance lens, and merged with no re-review on record. That
  is how the repository worked, not something the parser missed.
- The cited group overstates real citing. Several codecite plans (#45, #58,
  #59) are about the citation forms themselves and hold example citations with
  a placeholder hash, which the script counts as citations.
- Four PRs outside codecite have more than 100 changed files, and only their
  first 100 were read. None of those repositories has plans, so no PR's group
  changes.

Nothing here says codecite caused the difference. The table is a baseline to
rerun as more PRs are planned with codecite and reviewed.
