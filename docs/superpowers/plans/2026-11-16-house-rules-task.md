# house-rules task: implementation plan (#160)

**Goal:** an eval task over knowledge the model can't have, identity-stack's
house rules, at three corpus sizes, so the scorecard draws a curve per model
of where codecite starts to pay off.

**Architecture:** a new task kind, `corpus` (a size-dial task), alongside the
code tasks and the replay tasks. Its project is a pinned public repository,
its corpus a manifest per size, and its scorer runs the hidden tests in the
project's own environment and the checks against the project as the image
built it. Everything else (arms, agents, scorecard, gate, calibration,
freezing) is the existing harness.

## Decisions

- **Pin.** identity-stack at `efa14d27`, the merge of identity-stack PR #578
  (identity-stack#513, the functional domain core), which comes after PR #520
  (identity-stack#512, the rule layer, merged at `c9b5e226`). The issue's
  comment asks for the pin after both merge, and for the feature to need a
  decision in the roles and permissions context.
- **Feature.** Import a tenant's roles from a partner catalogue over HTTP
  (`POST /api/roles/import`), and make role names lowercase slugs everywhere.
  The import exercises configuration, outbound HTTP, logging, tenancy,
  authorization, results and spans; the slug rule is the domain decision. The
  hidden test that calls the domain directly targets `access.role_name`, the
  smart constructor the starting tree already has, so it needs no name the
  agent invents and passes only when the rule is decided there (conventions
  §25), not in the request model or the service.
- **Hidden tests** drive the real app over a fresh SQLite file per test, with
  a stub catalogue on loopback and the environment set before `app` is
  imported, so a value read at import time and one read at call time both
  see it. They run from `backend/` with the project's `.venv`, offline.
- **Checks** run identity-stack's own enforcement (Ruff's bans, import-linter,
  its house-rule tests) from the start's copies laid over the agent's tree,
  so loosening the enforcement doesn't help, plus reuse and routing checks
  of their own. Control rules (best practice) and generic best-practice
  checks are reported apart and never fail a run.
- **Corpus sizes.** `s` identity-stack; `m` plus identity-model and the
  identity spec documents; `l` plus five public FastAPI and auth
  repositories pinned by full SHA. The image indexes the manifest itself, at
  the pinned commits, so every rule's citation names the pin; the build fails
  if one doesn't resolve. The same commits are staged as files for both arms.
- **Probe.** `probe.toml` holds the three questions and their contamination
  patterns. A runner is out of scope here.

## Steps

1. Reference solution on identity-stack at the pin; hidden tests written
   against it, failing on the start (20 of 26) and passing with it.
2. `checks.py`; passes the start and the reference with every control.
3. `planted.py`/`planted.sh`: twenty-one rule breaks and hand-rolled helpers,
   each caught by its rule's check or by the hidden tests.
4. `rules.toml`: the rule layer with kind, checks and citation;
   `check.sh` proves the task, manifests and rules consistent.
5. Harness: `prepare.sh` stages a size (`CORPUS`), the Dockerfile indexes it
   and resolves the citations, `task.py` takes `-T corpus` and scores the
   kind, `panel.sh` and the Evals workflow pass the size, the report names
   each size's table, CI's `eval-task` runs `check.sh`.
6. Unit tests for the harness (`evals/tests/test_corpus_task.py`).
7. Verify locally: `check.sh`; `codecite read` of every rule citation after
   indexing each size; the sandbox image built at `s`. No agent runs locally.
8. PR, then calibration dispatches on the baseline arm at `s`, `m` and `l`.
