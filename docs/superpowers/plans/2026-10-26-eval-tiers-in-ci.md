# Eval tiers in CI

Issue #111: run the A/B eval in GitHub Actions as a quick tier on PRs labelled
`eval` and a full tier on dispatch, sharing one task and one scorer.

## Decisions

- **Tier is a word.** `evals/inspect/panel.sh quick|full` holds each tier's
  panel and epochs. `EPOCHS` and `MAX_SANDBOXES` still override; arguments
  after the tier go to `inspect eval`.
- **Quick panel:** claude-haiku-5.5, claude-sonnet-5.5 and openai/gpt-6.1-sol.
  No full Inspect run's medians are committed yet, so the third model is the
  fastest pi model by round one's medians (0.7 min per run,
  `docs/superpowers/specs/2026-10-21-fast-eval-design.md`). gemini-3.8-flash,
  the slowest in the first Inspect run, stays out. A reviewer can swap it.
- **One process per tier.** All models go to one `inspect eval` as a
  comma-separated `--model`, with `--max-tasks` set to the panel size so they
  run side by side, `--max-samples 10` per model, and `--max-sandboxes`
  bounding the live sandboxes, and with them the Docker networks.
- **Network pools.** Locally `MAX_SANDBOXES` defaults to 14, under the
  default pools' ~31 networks. CI gives the daemon /24 pools out of
  `10.200.0.0/16` (256 networks) and runs 8 sandboxes, Inspect's default for
  4 CPUs. The first CI run at 20 had samples finishing in waves, with ~314 s
  of wall time against ~70 s of working time: Inspect caps its subprocesses
  at the CPU count and docker CLI calls at twice it, and waiting on those
  caps counts in wall time. `panel.sh` raises both caps to twice
  `MAX_SANDBOXES`.
- **Logs by tier:** `inspect/logs/quick/` and `inspect/logs/full/`.
  `report.py --tier` heads the scorecard with the tier; the quick heading says
  it is directional and that a loss means run the full tier. `panel.sh` writes
  the scorecard to `scorecard.md` in the tier's log directory, with the
  tier's wall time, even when a model's run failed.
- **Resume:** `RETRY=1 panel.sh <tier>` runs `inspect eval-retry` over the
  tier's started, errored and cancelled logs. In CI, a dispatch's `resume`
  input names an earlier run whose `eval-logs-<tier>` artifact to finish.
- **Image cache:** after `prepare.sh`, the key is a hash of every file in the
  two images' build contexts (`inspect/build/`, `compose.yaml`,
  `fixtures/node-oidc/`), which covers the codecite binary and the
  identity-model commit. A hit loads the images with `docker load`; a miss
  builds them with `docker compose build` and saves them. Inspect then runs
  with `--sandbox-prebuilt`, so it verifies the images instead of rebuilding.
- **Two jobs.** `eval` holds the eval key and only `contents: read` (plus
  `actions: read` to fetch logs to resume). `comment` holds only
  `pull-requests: write` and posts the scorecard from the eval job's output,
  updating one comment found by a marker.
- **Concurrency:** one group per PR and tier, cancel-in-progress on PRs.
  Events that run nothing (another label, a push to an unlabelled PR) get a
  group of their own so they cancel nothing. Dispatches don't cancel.
- **Timeouts:** the job 45 min (quick) or 360 (full), and the eval step 35 or
  345, so the scorecard and logs still upload when the step times out.

## Steps

1. `panel.sh quick|full`: tiers, one Inspect process, bounded sandboxes,
   `RETRY`, the scorecard file.
2. `report.py --tier`: argparse, tier heading, tier log directory default.
   Check against synthetic logs written with `inspect_ai.log`.
3. `.github/workflows/evals.yml` as above; zizmor clean.
4. Docs: `evals/README.md` (both tiers, the label, CI, local runs and pools),
   `docs/design.md` (Evaluation plan, Tiers), `AGENTS.md` (issue flow step 6).
5. PR, then add the `eval` label to it for the first quick run, and dispatch
   one full run for its wall time.
