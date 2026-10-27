# Spec-question task

Issue: #104. Builds the Inspect task and scorer on #130's questions
(`evals/tasks/spec-questions/questions.toml`).

## Goal

An A/B task where the agent answers one question about the identity specs,
asked in its own words, and cites the section. Both arms have the specs as
files; the codecite arm also has codecite. The scorer is deterministic and
runs in the sandbox after the agent exits.

## Decisions

- **One sample per question per arm.** Sample ids are `<arm>:<question id>`,
  and a sample's metadata holds only its arm and question id, so no answer
  is in the dataset. The scorer looks the question up in `questions.toml`
  at scoring time and writes its `answer` and `accepted` into the sandbox
  with `score.py`.
- **The answer file** is `/work/answer.txt`, exactly two lines, `answer:` and
  `section:`. A fixed format keeps the scorer a string match: answers ignore
  case, quotes, backticks and a final full stop; sections must name an
  accepted section, a step of it counting, and codecite's `@sha256:` pin is
  allowed so the codecite arm can paste a citation. A subsection doesn't
  count.
- **Specs on disk.** `prepare.sh` writes `specs.tsv` (name, sha256) for the
  current documents of the corpus manifest when `task.json` names a `specs`
  directory. The Dockerfile copies each from codecite's document cache after
  indexing, so the files are the bytes codecite indexed, and `sha256sum -c`
  checks them against the pin again. Other tasks name no `specs`, so their
  images are unchanged.
- **No project, no IdP.** `task.json`'s `project` is null: `prepare.sh`
  stages an empty one and the Dockerfile skips `uv sync`. The task uses
  `inspect/compose-offline.yaml`, the sandbox alone with `network_mode:
  none`; the agent bridge needs no network.
- **Same agent.** `spec_task.py` reuses `task.py`'s `pi()` and
  `arm_to_store()`, so `PI_TOOLS`, `PI_FLAGS`, the arm split, timeouts and
  provider errors are handled the same way. The two scorers share
  `why_stopped()`.
- **Report.** Tables are per task, headed `**Task: `<name>`**`; a directory
  gives one log per task and model. Planned runs per arm are the arm's sample
  ids times epochs.
- **Epochs.** `task.json` sets 1 epoch on the quick tier and 3 on the full
  (34 and 102 runs per arm); `panel.sh` reads them, and `EPOCHS` overrides.
- **CI.** The eval job is a matrix over tasks: a labelled PR runs
  `fastapi-middleware` and `spec-questions` on their own runners, and a
  dispatch runs its `task` input. Each job uploads its scorecard; the
  comment job joins them into the one PR comment. Logs go to
  `inspect/logs/<tier>/<task>/` and the `eval-logs-<tier>-<task>` artifact.
  `eval-task` runs `spec-questions/check.sh`.

## Steps

1. `score.py`, then `planted.py` and `check.sh`: every question's reference
   passes; a wrong answer, a wrong or sub-section, a missing file and
   malformed files fail.
2. `task.json`, `prompt.md`, `spec_task.py`, `compose-offline.yaml`, the
   registry entry; `tests/test_spec_task.py` on the dataset, prompts and
   scorer (a local sandbox runs `score.py`).
3. `prepare.sh` and the Dockerfile; build the image and smoke-run two
   samples with Inspect's `mockllm/model`.
4. `report.py` per task and per question; tests for task tables, questions ×
   epochs and a question's sandbox error.
5. `panel.sh` (`TASK_ID`, task epochs, per-task log directory) and
   `evals.yml` (matrix, scorecard artifacts), `ci.yml` (`eval-task`).
6. Docs: `evals/README.md`, the task's README, `docs/design.md`.

## Left for after the merge of #127's PR

The issue's comment adds: calibrate on the baseline arm only, land in the
40–80% band, freeze before the first A/B run. That needs #127's baseline-only
dispatch, so the first quick A/B run on the PR waits for it.
