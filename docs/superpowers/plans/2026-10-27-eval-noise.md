# Measure eval noise with an A/A round and time runs from the transcript (#90)

Wall time is too noisy to call a time loss at 10 runs per arm. Build an A/A
round, read time and providers from each sample's transcript, run the round
in CI, and write down what it shows. The change to the verdict's measure is
James's call in review.

## Steps

1. **Tests first** (`evals/tests/`, `uv run --locked pytest` from `evals/`):
   - `test_task.py`: `fastapi_middleware(arms=...)` gives baseline and
     codecite by default, two baseline arms labelled `baseline-a` and
     `baseline-b` with identical prompts for `aa`, and refuses anything else.
     `pi()` reads the arm's condition, not its label.
   - `test_report.py`: logs built with model events, as the agent bridge
     writes them. An A/A log is headed A/A, keeps its labels, gets no
     verdict and names the measures past the margin. The transcript table
     shows median turns, output tokens, model time, other time and decode
     rate per arm; failed (retried) calls count as neither turns nor time.
     Providers come from the logged OpenRouter responses, counted per arm,
     made safe for a table cell. Every measure gets an interval. Logs
     without model events leave those measures blank, and the measures
     survive a write to and read from a log file.
2. **`config.py`:** the arm settings, each two arms of (label, condition),
   shared by the task and the report.
3. **`task.py`:** an `arms` task argument (`ab` default, `aa`). The sample
   metadata keeps the label as `arm` and adds `condition`, which the agent
   reads to set the prompt and sandbox.
4. **`report.py`:** pair the arms from the log's `arms` task argument. Read
   turns, model time and providers from the sample's model events (a call's
   time is its output's generation time, else Inspect's working time, else
   its start to its end), output tokens from its model usage. Add the
   transcript table and the interval table. The verdict's measures and
   rule stay as they are.
5. **`panel.sh` and `evals.yml`:** `ARMS=aa` runs the round into
   `inspect/logs/<tier>-aa/`; the workflow takes `arms` and `epochs` on
   dispatch, and a quick tier at more epochs gets the full tier's limits.
6. **Run** the A/A round on the quick panel at 10 epochs per arm from this
   branch, and link the run from the PR.
7. **Finding:** `docs/superpowers/specs/2026-10-27-eval-noise-design.md`
   with the run's table, per measure whether the A/A interval fits inside
   the 5% margin, and the time measure the verdict should use.
8. **Docs:** `evals/README.md` (A/A rounds, the scorecard's tables, the
   dispatch inputs) and `docs/design.md` (Arms, Measures). The verdict's
   measure in `docs/design.md` and `report.py` changes only once James
   approves the finding.

## Default a reviewer can change

An interval "fits" the margin when it lies inside 5% (5 points for pass
rate) on both sides of 0, since an A/A difference has no favourable side.
