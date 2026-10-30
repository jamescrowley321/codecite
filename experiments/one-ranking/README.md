# One ranking: sweeping the shared weights

Once code, prose and specs share one score order (#183), two weights apply to
every chunk: the `Docs` role weight (how much a match in prose counts against
one in code) and `NAME_BOOST` (the factor on a chunk whose symbol is named by
the query). `sweep.sh` moves one knob at a time from today's values (Docs 1.0,
NAME_BOOST 3.0), rebuilds release, indexes every history repository afresh and
runs the codecite arm of the history benchmark. Each run's pooled `all` rows
are in `results/`.

A value is adopted only if no repository is lower on Recall@1, Recall@10 or
MRR and at least one is higher.

Rerun with `experiments/one-ranking/sweep.sh` (all five values) or
`experiments/one-ranking/sweep.sh docs=0.7 name-boost=4.0` (any values). It
needs the history clones `evals/history/run.sh` makes.

## Results

Recall@1 / Recall@10 / MRR, pooled over each repository's queries. A value
below the base is in bold.

| Run | blind-peer-review | codecite | identity-model | identity-stack | terraform-provider-descope | Adopt |
| --- | --- | --- | --- | --- | --- | --- |
| base (Docs 1.0, NAME_BOOST 3.0) | .33 / .89 / .46 | .18 / .57 / .27 | .19 / .73 / .36 | .22 / .61 / .35 | .38 / .84 / .55 | |
| Docs 0.6 | **.31** / **.86** / .47 | .27 / .64 / .37 | .29 / .75 / .42 | .26 / .65 / .37 | .47 / **.81** / .61 | no |
| Docs 0.8 | .36 / .89 / .50 | .25 / .57 / .33 | .27 / .77 / .41 | .26 / .63 / .37 | .41 / .84 / .58 | **yes** |
| Docs 1.2 | **.31** / **.78** / **.43** | **.16** / **.50** / **.25** | **.15** / **.65** / **.31** | .22 / .61 / **.34** | **.34** / .84 / **.51** | no |
| NAME_BOOST 2.0 | .33 / .89 / .47 | **.11** / .57 / **.24** | .19 / .73 / .37 | .22 / .61 / .35 | .41 / .84 / .57 | no |
| NAME_BOOST 5.0 | **.31** / .89 / **.44** | .23 / .61 / .34 | .19 / .73 / .36 | .24 / .63 / .36 | **.34** / .84 / **.53** | no |

## Reading

Prose ranked level with code crowds code out: with one score order, a README
or design doc that repeats a query's words outscores the file a change
touched. Weighting prose down helps every repository up to a point. At 0.8 no
repository loses on any metric and all five gain on Recall@1 and MRR. At 0.6
four repositories gain more, but blind-peer-review, whose changes often land
in prose (skills, prompts), loses, as does terraform-provider-descope's
Recall@10. Raising prose to 1.2 loses almost everywhere.

`NAME_BOOST` is near its best at 3.0: lower loses the codecite repository's
Recall@1, higher loses blind-peer-review and terraform-provider-descope.

Adopted: `Docs` weight 0.8. `NAME_BOOST` stays 3.0. The values in between
(0.7, 0.9) and the two knobs together weren't swept; the bound on this sweep
is deliberate (#183).
