# Calibrate new eval tasks on the baseline arm and freeze them (#127)

A new task has to be hard enough that the arms can differ on correctness, and
tuning it while watching codecite's arm could tune it until codecite wins. So
calibrate on the baseline arm only, and freeze a task once its first A/B run
starts.

## Steps

1. **Tests first** (`evals/tests/`, `uv run --locked pytest` from `evals/`):
   - `test_task.py`: `ab(task_id, arms)` gives both arms' samples by default
     and only the baseline's with `arms="baseline"`, and refuses any other
     value.
   - `test_report.py`: logs whose task arguments say `arms=baseline` get a
     calibration table (pass, fails, timeouts, sandbox errors, median turns,
     tokens and time per model), the pooled pass rate and the band (in band,
     too hard, too easy, a model passing every run, incomplete), and no
     verdict for any pass count. An A/B log that lost its codecite samples
     stays an incomplete A/B row, and mixing the two kinds of log is refused.
   - `test_frozen.py`, on a throwaway git repo: a task matching its
     `frozen_at` passes; a changed, added or untracked file fails; a changed
     `task.json` fails unless only `frozen_at` differs; moving `frozen_at`
     fails against the base; an unfrozen task may change; an unknown commit
     fails; an unresolvable base fails. `fastapi-middleware` is frozen at
     a304f6a.
2. **`task.py`:** `ab_task(task_id, arms)` keeps only the baseline sample
   for `arms="baseline"`. A generic `ab(task_id, arms)` Inspect task runs any
   task by id, and each named task (`fastapi_middleware`, `client_auth_uri`)
   takes `arms` too.
3. **`report.py`:** tell a calibration from an A/B run by the log's `arms`
   task argument, not by which samples scored, and print the calibration
   table for it. The band, 40–80% pooled with no model at 100%, is one
   constant.
4. **`frozen.py`** (`codecite-eval-frozen [--base REF]`): diff each frozen
   task's directory against its `frozen_at` commit, ignoring `frozen_at` in
   `task.json`; with a base ref, refuse a moved `frozen_at`. CI's
   `evals-test` job runs it with the full history and the PR's base branch.
5. **`panel.sh`:** `TASK_ID` picks the task (`prepare.sh <task-id>` stages
   its image); `ARMS=baseline` runs the quick tier on the baseline arm alone
   into `inspect/logs/calibration/`.
6. **`evals.yml`:** dispatch inputs `task` and `arms`; check them before
   building; read the identity-model commit from the chosen task; name the
   log directory and artifact `calibration` for a calibration.
7. **Freeze `fastapi-middleware`** at a304f6a (#101), the commit of its first
   A/B run; its files are unchanged since.
8. **Docs:** the band, the freezing rule and `fastapi-middleware`'s role as a
   regression check in `docs/design.md` (Agent benchmark); how to calibrate
   in `evals/README.md`.
