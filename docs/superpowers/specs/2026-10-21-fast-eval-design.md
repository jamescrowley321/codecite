# Fast A/B eval design

This replaces how round one ran (`2026-10-20-ab-eval-harness-design.md`). The
harness, sandbox, taint scan and bootstrap verdict stay. What changes is how
much runs, how fast, and what "the task is done" means.

## Why

Round one was too slow to be useful: 680 runs, one at a time, up to 20
minutes each, with a whole round pausing when one model hit a rate limit. It
would have taken more than a day. Its correctness measure was also too weak.
The hidden tests check behaviour only, so an agent could ignore the
instructions ("use identity-model's `TokenValidationMiddleware`") and still
pass. Almost every run passed in both arms, so correctness couldn't tell the
arms apart.

James wants an eval that:

- finishes in **under 20 minutes**, from `compose` to the scorecard;
- runs **fewer tests**: a small fixed panel of models, not every model on
  OpenRouter;
- checks the output **deterministically**: did the agent do what the task
  asked, the way it asked;
- possibly adds an **LLM judge** for what a script can't check.

## Goals

- A full eval (`compose` → `call` → `score` → scorecard) finishes in under 20
  minutes of wall-clock time on this machine (22 cores, 62 GB).
- Correctness means the hidden tests pass **and** the deterministic checks
  pass.
- An LLM judge grades each solution, blind to its arm, and its scores appear
  in the scorecard.
- One model's rate limit slows only that model, never the whole eval.

## Non-goals

- A second, harder task. It is worth doing, since the current task is too
  easy to separate the arms on correctness, but it gets its own issue.
- Ranking models against each other. The eval compares codecite against
  baseline within each model.
- Removing the slow, full-size round. It stays available as a plan file for
  release decisions. It just isn't the default.

## Anti-goals

- Getting fast by giving up isolation: every run still runs in its own
  sandbox, with no access to the key, the home directory or the eval files.
- Letting the judge see the arm. That includes the transcript, which shows
  codecite calls. The judge sees the task and the diff only.
- Gating on the judge before anyone has shown it agrees with the
  deterministic checks.

## The decisions

### 1. The panel: six models

| Agent | Model | Why it's in |
|---|---|---|
| claude | claude-opus-5-5 | The strongest result in round one (tokens −12%) |
| claude | claude-sonnet-5-5 | The default most people run |
| pi | openai/gpt-6.1-sol | Fastest pi model (0.7 min per run) |
| pi | google/gemini-4-argon | Google, 1.5 min per run, never failed |
| pi | deepseek/deepseek-v4.1-flash | A cheap open-weights model, 1.8 min |
| pi | mistralai/devstral-2512 | A coding-tuned open model, 2.2 min |

Dropped: haiku (it ignored codecite in 80% of runs), and every model whose
runs averaged more than 2.5 minutes, or that hit rate limits (kimi-k3,
kimi-k2.7-code, mistral-large, mistral-medium, qwen3.8-27b, gpt-6.1-sol-pro,
gemini-3.8-flash). glm-5.3, gpt-6-luna-pro and deepseek-v4-pro ran fine
but are cut to keep the panel at six, one model per vendor where possible.
The panel is a plan file
(`evals/plans/quick.json`). Changing it is a one-line edit.

### 2. Runs: 10 per arm

6 models × 2 arms × 10 runs = **120 runs**. Round one's averages put that at
about 150 run-minutes.

The cost is precision. With 10 runs per arm, the bootstrap interval on tokens
roughly doubles in width compared with 20. The quick eval will catch large
losses, such as codecite costing 30% more tokens, but not small ones. That is
the trade James asked for. The full plan (20 per arm) stays for when a small
difference matters.

The INCOMPLETE threshold follows the plan's `runs`, not a hard-coded 20: a
pair needs `runs` usable runs per arm.

### 3. Concurrency

`call` gains `--concurrency N`, default 12. It runs up to N runs at once,
each in its own sandbox, as now.

- **Order:** the runs are still drawn in the plan's seeded shuffle, but
  runs whose model is slow on average start first, so the slowest work
  isn't left for the end.
- **Per-model limit:** at most 4 runs of one model at once
  (`maxPerModel`, a plan field), to stay under provider rate limits.
- **Rate limits:** a 429 makes that run back off with jitter (5 s, 20 s, 60 s),
  then retry. After 3 tries the run ends `infra_error` and is redone at
  the end of the call. Other runs keep going. The round-wide rate-limit stop
  goes.
- **Wall time** is still measured per run. Concurrent runs share the machine,
  but both arms are interleaved by the shuffle, so the load is the same for
  both arms on average.

### 4. Shorter timeout

`timeoutSec` drops from 1200 to **360** (6 minutes). The slowest panel
run in round one took 6.1 minutes, and the medians are under 2.5. A
timeout still counts as a failed run, as now.

### 5. Deterministic checks

Each task gains a `checks` script beside its hidden tests:
`evals/tasks/<id>/checks.py`. Scoring runs it inside the sandbox after
pytest, using the project's Python, with the start project and the
agent's final project as arguments. It prints one JSON object:

```json
{"checks": [{"name": "uses-middleware", "passed": true, "detail": "..."}]}
```

For `fastapi-middleware` the checks are:

| Check | Passes when |
|---|---|
| `uses-middleware` | `app/main.py` imports `TokenValidationMiddleware` from `fastapi_identity_model` and passes it to `app.add_middleware` (read with `ast`, not a regex) |
| `uses-current-user` | The `/me` handler takes a parameter annotated with, or defaulting to, identity-model's `CurrentUser` |
| `reads-env` | `OIDC_DISCOVERY_URL` and `OIDC_AUDIENCE` are read from `os.environ` or `os.getenv`, and no `http://` or `https://` literal appears in `app/` |
| `deps-unchanged` | `pyproject.toml` and `uv.lock` match the start project byte for byte |
| `scope` | Nothing outside `app/` was added, removed or changed (ignoring `.venv`, caches, and `__pycache__`) |

A run **passes** when every hidden test passes and every check passes.
`score.json` records each check, and the scorecard shows a pass rate for each
check per arm, so a fall in correctness can be traced to a cause.

`check.sh` (the task's self-test) grows: the reference solution must pass
every check, and one planted bad solution per check must fail that check
(for example a hand-written JWT check that passes the hidden tests but
fails `uses-middleware`).

### 6. LLM judge

After scoring, `judge` grades each scored run.

- **What it sees:** the task prompt, the unified diff from the start project
  to the agent's project (only `app/`, at most 400 lines), and the rubric.
  No transcript, no arm, no model name, no run id.
- **Model:** `anthropic/claude-opus-5-5` called straight through OpenRouter's
  chat API with `temperature: 0` and a JSON schema for the answer. It's not
  an agent and has no tools. One call per run, 16 at a time.
- **Rubric:** three scores from 1 to 5, each with a one-sentence reason.
  - *Follows the instructions:* all five requirements, done the way the
    prompt says.
  - *Uses the library as intended:* identity-model's middleware and
    dependency, no reimplemented token checks.
  - *Change hygiene:* small and focused, no dead code, no debugging prints,
    no hard-coded settings.
- **Caching:** results are stored per diff hash in `judge.json`, so
  re-scoring doesn't re-pay.
- **In the scorecard:** the median of each score per arm, with the same
  bootstrap interval. It's **reported, not gating**. A later change can make
  it gate once a round shows the judge agrees with the deterministic checks.
  The scorecard shows that agreement: how often a run failing a check got
  "follows the instructions" ≤ 2.
- **Bias:** the judge is a Claude model grading Claude models too. That bias
  hits both arms of a pair equally, and the eval only compares arms within a
  pair, so it cancels out.

### 7. Time budget

| Step | Estimate |
|---|---|
| compose (export, uv sync, IdP freeze) | 1 min |
| call: 150 run-minutes ÷ 12 at once, plus a tail up to one 6-minute timeout | 13–15 min |
| score: 120 pytest + checks runs, 8 at once, one shared IdP | 1–2 min |
| judge: 120 calls, 16 at once | 1–2 min |
| **total** | **16–20 min** |

The scorecard records each step's wall time. If the total goes over 20
minutes, it says so at the top.

### 8. One command

`node evals/run.mjs quick` runs compose, call, score, judge and the
scorecard in order, on `evals/plans/quick.json`, and prints the scorecard
path. `run-round.sh` and the 15-minute sleeps go away.

## Constraints

- No npm dependencies (as now). The judge uses `fetch`.
- The OpenRouter key stays outside the sandbox. The judge runs outside it
  (it reads diffs, not code), so the key never enters one.
- The pi adapter's provider allow-list on the account doesn't change. Every
  panel model is served by an allowed provider (checked in round one).
- The `check.sh` self-test must pass before a round can compose (as now).

## Left to the builder

- How the scheduler is built (a worker pool in `callRound` is fine).
- Exact backoff numbers and the per-model limit's default, within the numbers
  above.
- The judge prompt's wording, as long as it holds the rubric above and
  leaves the arm out.
- Where per-check results go in the scorecard layout.
