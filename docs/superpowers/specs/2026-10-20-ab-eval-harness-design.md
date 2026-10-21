# The A/B eval harness: does codecite make coding agents better?

## Intent

codecite exists to make coding agents better at real work, starting with the
owner's identity-model monorepo. Whether it does is still unmeasured. The owner's
rule is blunt: "There is no point working on this if it performs worse." Feature
work stops until this harness gives an answer, and it keeps running after that so
every later feature is judged by the same numbers.

The harness gives the same coding task to the same agent and model twice, in two
arms:

- **Baseline:** the agent's normal tools (shell, file read and write, grep, glob).
- **codecite:** the same tools, plus the `codecite` CLI on `PATH` with the task's
  corpus already indexed.

It scores each run with hidden tests and records tokens and wall-clock time. A
scorecard then says whether codecite lost on anything.

The first round is deliberately small: one task, a few agent/model pairs, many
runs. Its job is an early, trustworthy signal on tokens and time, and a harness
that later rounds can feed harder identity-model tasks into without rework.

## Goals

1. **A go/no-go verdict by the owner's rule.** codecite must not lose on
   correctness, tokens or time, for any agent/model pair (see *Scorecard*).
2. **Cheap to add an agent.** The first agents are Claude Code and pi. The owner
   will add more, so an agent is one adapter behind a small interface.
3. **Cheap to add a task.** A task is a directory: starting project, prompt,
   hidden tests and corpus definition. The harness core doesn't change.
4. **Arms are data.** An arm is a named set of tools and prompt additions in the
   plan. Round one has two; web tools and MCP tools come as more arms.
5. **Repeatable.** The plan (task, agents, models, arms, run count, commits) is
   frozen in a file before anything runs. Two runs of the same plan are comparable.
6. **Honest telemetry.** A number the agent didn't report is recorded as unknown,
   never as zero, and an unknown can't count as a win.

## Non-goals

- MCP tools. The codecite arm uses the CLI through the shell. The owner chose the
  CLI because it is ready now and quick to run. MCP `search` and `read` (#32) can
  become a third arm later; the harness doesn't depend on #32.
- CI. The first rounds run locally on the owner's machine. A manually dispatched
  CI job on `main` comes later (GitHub-hosted runners have Docker for the IdP).
- The retrieval benchmark (recall@10, MRR) in `docs/design.md`. That is separate
  work.
- Citation accuracy scoring. This task has no spec citations to check.

## Anti-goals

- **A rigged comparison.** If the arms differ in anything except codecite (prompt
  text beyond the codecite note, the source the agent can read, model settings,
  user config leaking in), the result is worthless even if it says "win".
- **Leaking the answer.** The agent never sees the hidden tests, the IdP fixture,
  or identity-model's git history and branches (fix branches exist that would
  contaminate later tasks).
- **Surprise spend.** No run spends past the eval key's cap. A misconfigured key or
  model fails fast and doesn't spend credits on retries.

## The first task: protect a FastAPI app

**Starting project:** a tiny FastAPI app written for the eval (the owner's
choice): three or four routes, a `/health` route, no auth. It depends on
`fastapi-identity-model` from the identity-model copy (below) so the package is
installed but unused. Its Python environment is built before the clock starts.

The hidden tests rely on a fixed contract that the starting project and prompt
set (our call):

- The app object is `app.main:app`.
- A `/me` route already exists and returns 501. The prompt asks the agent to make
  it return the current user as JSON with `sub` and `client_id`, taken from
  `CurrentUser`.
- The app reads `OIDC_DISCOVERY_URL` and `OIDC_AUDIENCE` from the environment at
  startup, not import. The prompt names both variables. The hidden tests set them
  before creating the test client.

**Prompt (same in both arms):** protect the app with identity-model's
`TokenValidationMiddleware`, leave `/health` open, and expose the logged-in user
to route handlers through `CurrentUser`. The prompt names the discovery URL and
audience to configure through environment variables, without saying how the
middleware takes them. The middleware is
`py/packages/fastapi-identity-model/fastapi_identity_model/middleware.py` in
identity-model; its `__init__` takes `discovery_url`, `audience`, `excluded_paths`,
`custom_claims_validator`, `require_access_token_marker` and
`access_token_marker_claims`. Reference usage is in `py/examples/fastapi/app.py`
and `py/examples/descope/app.py`.

**Hidden tests (pytest, run after the agent exits):** against a real IdP, using
real tokens:

| Request | Expected |
| --- | --- |
| No token | 401 |
| A valid `client_credentials` token | 200, and the user exposed through `CurrentUser` carries the right `sub` / `client_id` |
| A tampered token | 401 |
| An expired token | 401 |
| A token minted for another audience | 401 |
| `/health` with no token | 200 |

A run passes only if every hidden test passes. The hidden tests exercise the app
through HTTP (or FastAPI's test client against the app object), so they don't
depend on how the agent structured its code.

**Corpus:** identity-model at a pinned commit, exported without `.git` to a fixed
path outside the run's project. The prompt in both arms names that path with the
same words. The codecite arm also has it indexed. That's a new eval corpus
manifest covering the export only (our call). The spec documents in
`corpora/identity.toml` stay out, because the baseline arm has no copy of them on
disk. Neither arm gets information the other can't reach.

## The test IdP: node-oidc-provider

The owner chose node-oidc-provider because it runs in Docker or npm and has a lean
config. identity-model already uses it
(`~/repos/auth/identity-model/infra/node-oidc-provider/`), but its `provider.js`
is 469 lines with spec-vector routes and auth-code and PKCE clients. The eval gets
a lean copy at `evals/fixtures/node-oidc/`:

- `oidc-provider` pinned to exactly `9.7.1` (identity-model's locked version), with
  a committed `package-lock.json`.
- One `client_credentials` client.
- `resourceIndicators` configured so access tokens are JWTs with an `aud`, and a
  second resource so the hidden tests can mint a wrong-audience token.
- A short access-token TTL, or another documented way to get an expired token.
- Runs with `docker compose` or `npm ci && node provider.js`, on `127.0.0.1` only.
- Env names follow identity-model's `.env.node-oidc`
  (`TEST_DISCO_ADDRESS`, `TEST_JWKS_ADDRESS`, `TEST_CLIENT_ID`). The client secret
  is a fixture value written for this repo, never copied from identity-model.

The IdP starts only for scoring. Agents don't need it running while they work.

## Agents and models (round one)

| Agent | Model | Billing |
| --- | --- | --- |
| Claude Code (`claude -p`) | Sonnet 5.5 | the owner's subscription; tokens still recorded |
| pi 1.2.0 via OpenRouter | `google/gemini-4-argon` | eval key |
| pi via OpenRouter | `openai/gpt-6.1-sol` | eval key |
| pi via OpenRouter | `deepseek/deepseek-v4.1-flash` | eval key |

The owner left the model choice to us and is open to the top class. The two top
models were picked from OpenRouter's live list on 2026-10-20. The cheap one shows
whether codecite helps a weaker model more. Slugs go in the frozen plan, so a
renamed model fails at plan validation, not mid-run.

**Run count:** 20 per agent/model/arm by default (160 runs in round one, 120 of
them on the eval key), set in the plan (see *Scorecard* for why 20). The owner wants many runs to beat noise, so the run count is a plan setting
and raising it needs no code change.

## Isolation

Each run gets:

- **A fresh copy of the starting project** in a scratch directory outside any git
  repo, with its own Python environment.
- **A clean agent home.** pi runs with a fresh `PI_CODING_AGENT_DIR` (the owner's
  `~/.pi/agent` has a custom key, `+codemode` and disabled MCP; none of it may leak
  in). Claude Code runs without the owner's user-level `CLAUDE.md`, memory,
  plugins, hooks, skills or MCP servers (codegraph, playwright and others are
  configured there). How to get that clean start is the builder's call. Either way,
  the run records Claude Code's init event (tools, MCP servers, plugins) to prove it.
- **Its own `XDG_DATA_HOME`** in the codecite arm, holding a copy of the
  prebuilt corpus `.db`. Building the index happens once per round, before any
  run, and doesn't count against the codecite arm's time. The baseline arm has no
  `codecite` on `PATH`.
- **The same prompt**, plus, in the codecite arm only, a short note naming the
  `codecite search` and `codecite read` commands and the corpus. The note's text is
  part of the plan and is printed on the scorecard.

- **No web tools.** Claude Code's WebFetch and WebSearch are off in both arms,
  and pi gets no web tool. The shell still has network access, because the
  agent needs its model provider. After each run the harness scans the transcript
  for `github.com`, `git clone`, `curl`, `wget`, `pip install` / `uv add`, and any
  path into codecite's own `evals/tasks` (reachable by absolute path), and
  marks a hit as **tainted**. Tainted runs are listed on the scorecard and left
  out of the verdict, and the round reports "incomplete" if that leaves too few
  runs (the owner's choice, over a network sandbox for round one).

Runs are serial by default (parallel runs would distort wall-clock time), and the
order of arms is shuffled with a seed in the plan so time-of-day drift hits both
arms alike.

## The harness

The harness lives under `evals/` and follows blind-peer-review's eval shape
(`~/repos/blind-peer-review/evals/`), which the owner already trusts:

1. **compose** (no secrets): validate and freeze the plan to `plan.json`: task,
   agents, models, arms, run count, seed, codecite commit, identity-model commit,
   prompt and codecite note. `--dry-run` stops here and prints what would run and
   a cost estimate.
2. **call** (the only phase that holds keys): run each agent invocation, capture
   its raw output, usage and wall-clock time.
3. **score:** start the IdP, run the hidden tests against each run's project, then
   build the scorecard.

Each phase writes to a round directory outside any git repo (default
`~/.local/state/codecite-evals/<round-id>/`), so an agent can't walk up into the
hidden tests, and a round can be
re-scored without re-running agents.

**Adapter interface (what each agent provides):** build the command and
environment for one run in either arm, and parse the agent's output into
`{input, output, cacheRead, cacheWrite, totalTokens, costUsd, turns, exitStatus}`,
leaving fields the agent didn't report as unknown.

- **Claude Code:** `claude -p` with JSON output (`usage`, `total_cost_usd`,
  `duration_ms`, `num_turns`). Authenticates with the owner's subscription. Never
  set `ANTHROPIC_API_KEY` alongside it, or it bills per token.
- **pi:** `pi --mode json --provider openrouter --model <slug>`. It emits JSONL;
  usage is on `message_update.usage`
  (`input, output, cacheRead, cacheWrite, totalTokens, cost.total`). Split on LF
  yourself; Node's readline mishandles it. Tools are set explicitly with `--tools`
  so both arms get the same built-ins. If pi can send an OpenRouter `X-Title`, set
  it per arm so OpenRouter's dashboard splits spend by arm.

**Keys:** the OpenRouter key is a dedicated `codecite-evals` key on the owner's
personal account, capped at $150 a month, minted by Terraform in the private
oss-admin repo (`terraform/blind-peer-review`, oss-admin#27). It lives locally in
`~/.config/codecite/evals.env` (mode 0600) as `OPENROUTER_EVAL_API_KEY`. The
harness reads only that file, passes the key to pi through the run's environment,
and never prints it. The owner's other OpenRouter key in `~/.pi/agent/auth.json`
is never used. When CI comes, the key goes to a codecite secret named
`OPENROUTER_EVAL_API_KEY`, not `OPENROUTER_API_KEY` (oss-admin's
`lens_review["codecite"]` owns that one).

**Spend and failure guards (from blind-peer-review):**

- Before calling, check the key's remaining limit and refuse a plan whose
  estimate exceeds it.
- HTTP 400, 401, 402 and 404 from the provider are fatal: stop the round.
- Circuit breaker: stop when at least 80% of calls have failed after 10 calls.
- A per-run timeout (a plan setting). A run that times out is a failed run for
  correctness, and its tokens and time count as recorded.
- An infrastructure failure before the agent did any work (provider 5xx on the
  first call, harness error) is retried, not scored as a failure. It is listed on
  the scorecard.

## Scorecard

Per agent/model pair, comparing the codecite arm to the baseline arm:

| Measure | Loss when |
| --- | --- |
| Correctness | codecite's pass rate is more than 5 percentage points below baseline's |
| Tokens | codecite's median total tokens are more than 5% above baseline's |
| Time | codecite's median wall-clock time is more than 5% above baseline's |

The owner set the 5% margin so a difference inside run-to-run noise doesn't stop
the project, and chose "many runs" over a statistical test to keep the noise
down. The rule is a plain comparison of the numbers (our reading of the owner's
answer). Any loss on any measure for any pair is a **stop**. If a measure is
unknown for a pair, the pair can't pass that measure, and the verdict says so.

Our calls on applying it:

- **Runs that count.** Token and time medians cover every scored run, failed and
  timed-out runs included, because a failed attempt still costs its tokens and
  time. The scorecard also shows medians over passing runs only.
- **Equal counts.** A verdict needs every pair to reach the planned run count in
  both arms. Retried infrastructure failures are rerun until it does, or the
  round reports "incomplete" rather than a verdict.
- **Run count vs margin.** With 10 runs, one extra failure is 10 points, so the
  correctness margin only does its job from 20 runs up. The plan defaults to 20
  runs per agent/model/arm. `--dry-run` prints the cost so the owner can lower it.

The scorecard also shows per pair: run count, pass rate, median and spread
(interquartile range) of tokens and time, dollar cost, tool calls, how often
the codecite arm actually called `codecite`, retried runs, and the exact plan.
It is written as Markdown so the owner can paste it into an issue.

Total tokens means everything the model processed: input, output, cache reads and
cache writes. Cost is shown beside it because cached tokens are cheap.

## Later rounds (out of scope here, shaping the design)

Harder identity-model tasks plug in as new task directories:

- Replay Rust port commits and score with each commit's own tests: `7c6d453`
  (RFC 7009), `70148c3` (RFC 8693), `493ff0c` (RFC 7662), `a2bae55` (DPoP). Take
  the scoring tests from the replayed commit; the test layout changed at
  `9458d60`.
- The 33 strict-xfail `_KNOWN_GAPS` in
  `py/src/tests/spec_vectors/test_spec_http_vectors.py`.

**Web tools as a measured condition.** The owner's next step after round one:
give the baseline its web tools back, because searching the internet has its own
token and time overhead and can pull in wrong or stale material, and codecite
has neither. That is a third arm (baseline plus web tools), run against the same
codecite arm, so the plan's arm list must not be fixed at two.

The replay tasks need a network sandbox (only the model provider reachable)
before they run, because a `git clone` there leaks the answer, not only a hint.

These need a starting project taken from a git commit, and hidden tests taken
from a later commit. The task-directory format should allow that without assuming
a hand-written app.

## Changes to `docs/design.md`

Replace the *Initial success criteria* under *Evaluation plan* with the owner's
rule (no loss on correctness, tokens or time, with the 5% margin, per
agent/model pair). Describe the arms as baseline tools versus the codecite CLI,
with MCP tools as a later arm. The retrieval benchmark stays as written.

## Left to the builder

- Harness language. Node `.mjs` matches blind-peer-review and the IdP fixture
  already needs Node; the starting project and hidden tests are Python (managed
  with `uv`).
- How Claude Code gets its clean start, as long as the init event proves it.
- The exact prompt wording and the codecite note, within the rules above.
- The cost estimator's method.
- Directory layout under `evals/`, as long as a task is one directory.
