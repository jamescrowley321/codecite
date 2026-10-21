# A/B Eval Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `evals/`, a local harness that runs one coding task through Claude Code and pi in a baseline arm and a codecite-CLI arm, scores each run with hidden tests against a real IdP, and prints a no-loss scorecard.

**Architecture:** A Node CLI (`evals/run.mjs`) with three phases: `compose` freezes a plan, `call` runs agents and records raw output, and `score` runs the hidden tests and writes the scorecard. Each phase reads and writes a round directory outside any git repo (`$CODECITE_EVALS_DIR`, default `~/.local/state/codecite-evals/<round>/`), so an agent can't walk up into `evals/tasks/*/hidden`. A task is a directory. An agent is one adapter module. An arm is data in the plan. The IdP is a lean node-oidc-provider fixture, and the task's project and hidden tests are Python managed with `uv`.

**Tech Stack:** Node 26 (ESM `.mjs`, `node:test`, no npm deps in the harness itself); `oidc-provider` 9.7.1 for the IdP; Python ≥3.12 with `uv`, FastAPI, pytest and httpx for the task; the `codecite` release binary; `claude` 2.1.x and `pi` 1.2.0.

**Spec:** `docs/superpowers/specs/2026-10-20-ab-eval-harness-design.md`

## Global Constraints

- The arms differ only in `codecite` being on `PATH` with a prebuilt corpus, and the codecite note appended to the prompt. Everything else is byte-identical.
- The agent never sees: `hidden/`, `reference/`, the IdP fixture, or identity-model's `.git`.
- A number an agent didn't report is `null` (unknown), never `0`. Unknown never passes a measure.
- A loss is codecite's pass rate more than **5 percentage points** below baseline's, or its median total tokens or median wall time more than **5%** above baseline's, for any agent/model pair. Any loss gives the verdict `STOP`.
- Default runs: **20** per agent/model/arm. Runs are serial, with arm order shuffled by the plan's seed.
- `claude -p` authenticates with a subscription token from `claude setup-token`, stored by the owner as `CLAUDE_CODE_OAUTH_TOKEN` in the env file below.
- The OpenRouter key is read only from `~/.config/codecite/evals.env` (`OPENROUTER_EVAL_API_KEY`) and is never printed or written into results. The Claude Code token is read from the same file (`CLAUDE_CODE_OAUTH_TOKEN`). `ANTHROPIC_API_KEY` is never set.
- Web tools are off in both arms. Transcripts matching `github.com`, `git clone`, `curl `, `wget `, `pip install` or `uv add`, or touching `evals/tasks`, `test_hidden`, `/reference/` or `codecite-worktrees` (the hidden tests are reachable by absolute path without a sandbox), are tainted and left out of the verdict.
- Fatal provider statuses: 400, 401, 402 and 404 stop the round. The circuit breaker trips at ≥80% failed calls after ≥10 calls.
- Round directories live outside any git repo: `$CODECITE_EVALS_DIR`, default `~/.local/state/codecite-evals/`. Nothing in this repo contains a real secret, and the IdP client secret is a fixture value.
- identity-model is pinned at `9458d60f4f2f20227bddf2594fd7d10cba66ea0a`.

## Review Focus

1. **A run that never reaches the agent loop.** For example, a bad model slug: OpenRouter returns 404 on the first call, and pi exits non-zero with no usage. The expected behavior is a fatal stop, not 160 runs each scored as a failure. Pinned in Task 4 by `classifies a 404 as fatal`.
2. **An agent that edits outside its project dir** (for example the identity-model export). The export is shared across runs, so later runs would see the change. Expected: the export is read-only (`chmod -R a-w`), and the run records the write failure as the agent's problem. Pinned in Task 3 by `export is read-only`.
3. **A `claude -p` session that still loads the owner's config.** Expected: the run's init event lists no MCP servers and no plugins, and the harness refuses to score a run whose init event shows any. Pinned in Task 5 by `rejects a run with user MCP servers`.
4. **Uneven run counts after retries or taint.** Expected: the pair reports `INCOMPLETE`, not a verdict on the runs that are left. Pinned in Task 7 by `incomplete when an arm is short`.
5. **pi's JSONL stream with a partial last line** (the agent was killed at the timeout). Expected: the parser drops the partial line, keeps the usage it already saw, and marks the run `timeout`. Pinned in Task 6 by `tolerates a truncated last line`.

---

Each task below is one GitHub issue in the "M4 Evaluation harness v1" milestone, and its own PR.

### Task 1: The node-oidc-provider fixture

**Files:**
- Create: `evals/fixtures/node-oidc/package.json`, `package-lock.json`, `provider.js`, `Dockerfile`, `compose.yaml`, `README.md`
- Create: `evals/fixtures/node-oidc/smoke.test.mjs`

**Interfaces:**
- Produces: an issuer at `http://127.0.0.1:9011` (env `PORT`, default `9011`, so it doesn't collide with identity-model's `9010`), discovery at `/.well-known/openid-configuration`.
- Clients (`token_endpoint_auth_method: client_secret_post`, grant `client_credentials`):
  - `eval-client` / `eval-secret`: access token TTL 600 s
  - `eval-expiring` / `eval-secret`: access token TTL 2 s
- Resources: `urn:codecite-eval:api` (the audience the app must enforce) and `urn:codecite-eval:other` (for the wrong-audience test). Both issue RS256 JWT access tokens. `defaultResource` is `urn:codecite-eval:api`. TTL comes from `getResourceServerInfo(ctx, resource, client)` using `client.clientId`.
- Start: `npm ci && node provider.js`, or `docker compose -f evals/fixtures/node-oidc/compose.yaml up -d`. Readiness: `GET /.well-known/openid-configuration` returns 200.

- [ ] **Step 1: Write `smoke.test.mjs`.** It starts `provider.js` on a free port as a child process, waits for discovery, and asserts:
  - `client_credentials` for `eval-client` with `resource=urn:codecite-eval:api` returns a JWT whose payload has `aud === "urn:codecite-eval:api"`, `client_id === "eval-client"`, and `exp - iat === 600`.
  - The same request with `resource=urn:codecite-eval:other` returns `aud === "urn:codecite-eval:other"`.
  - `eval-expiring` returns `exp - iat === 2`.
- [ ] **Step 2: Run `node --test evals/fixtures/node-oidc/smoke.test.mjs`.** Expected: FAIL (no provider).
- [ ] **Step 3: Write `provider.js` as a lean copy of identity-model's `infra/node-oidc-provider/provider.js`.** Keep only the config above: no vectors, no auth-code or PKCE clients, and no `/v/` routes. Use `package.json` with `"oidc-provider": "9.7.1"` exact, and generate `package-lock.json` with `npm install`. Add a Dockerfile (`node:26-alpine`, `npm ci`, `node provider.js`) and `compose.yaml` that binds `127.0.0.1:9011`.
- [ ] **Step 4: Rerun the test.** Expected: PASS. Then run `docker compose -f evals/fixtures/node-oidc/compose.yaml up -d && curl -sf http://127.0.0.1:9011/.well-known/openid-configuration` and expect JSON.
- [ ] **Step 5: Commit** `feat(evals): add a lean node-oidc-provider fixture`. The README covers what it serves, both start paths, and the env names `TEST_DISCO_ADDRESS`, `TEST_JWKS_ADDRESS` and `TEST_CLIENT_ID`.

### Task 2: The FastAPI task: starting project, prompt, hidden tests, reference solution

**Files:**
- Create: `evals/tasks/fastapi-middleware/task.json`
- Create: `evals/tasks/fastapi-middleware/prompt.md`
- Create: `evals/tasks/fastapi-middleware/project/` with `pyproject.toml`, `app/__init__.py`, `app/main.py`, `README.md`
- Create: `evals/tasks/fastapi-middleware/hidden/test_hidden.py`, `hidden/conftest.py`
- Create: `evals/tasks/fastapi-middleware/reference/app/main.py`
- Create: `evals/tasks/fastapi-middleware/check.sh`

**Interfaces:**
- Consumes: Task 1's issuer, clients and resources.
- Produces `task.json`:

```json
{
  "id": "fastapi-middleware",
  "project": "project",
  "prompt": "prompt.md",
  "hidden": "hidden",
  "corpus": { "repo": "identity-model", "commit": "9458d60f4f2f20227bddf2594fd7d10cba66ea0a" },
  "idp": "node-oidc",
  "setup": ["uv", "sync"],
  "score": ["uv", "run", "--with", "pytest", "--with", "httpx", "pytest", "-q", "{hidden}"]
}
```

- `project/pyproject.toml` depends on `fastapi`, `uvicorn` and `fastapi-identity-model`. `[tool.uv.sources]` points `fastapi-identity-model` at `@IDENTITY_MODEL@/py/packages/fastapi-identity-model` and `py-identity-model` at `@IDENTITY_MODEL@/py`. Task 3 replaces `@IDENTITY_MODEL@` with the export path.
- `project/app/main.py` has `app = FastAPI()` and the routes `/health` → `{"status":"ok"}`, `/items` → a fixed list, `/items/{id}` → one item, and `/me` → HTTP 501. There's no auth.
- `prompt.md` asks the agent to:
  - protect every route with identity-model's `TokenValidationMiddleware`
  - leave `/health` open
  - make `/me` return `{"sub": ..., "client_id": ...}` for the current user, using `CurrentUser`
  - read `OIDC_DISCOVERY_URL` and `OIDC_AUDIENCE` from the environment when the app starts, not at import
  - treat the identity-model source as available at `{identity_model}`
  - not use the network beyond what's already installed
  - stop when done

- [ ] **Step 1: Write `hidden/conftest.py` and `hidden/test_hidden.py`.** The conftest takes the IdP base from `TEST_IDP` (default `http://127.0.0.1:9011`), sets `OIDC_DISCOVERY_URL` and `OIDC_AUDIENCE=urn:codecite-eval:api` before importing `app.main`, and yields `TestClient(app)` inside a `with` block so startup runs. A `token(client="eval-client", resource="urn:codecite-eval:api")` helper calls the token endpoint. Tests:
  - `test_no_token_401`: `GET /items` returns 401.
  - `test_valid_token_200`: `GET /items` with an `eval-client` token returns 200.
  - `test_me_returns_identity`: `GET /me` returns 200 with `client_id == "eval-client"` and `sub` equal to the token's `sub`.
  - `test_tampered_401`: flip one character in the signature segment, expect 401.
  - `test_expired_401`: take an `eval-expiring` token, sleep 4 s, expect 401.
  - `test_wrong_audience_401`: `resource=urn:codecite-eval:other`, expect 401.
  - `test_health_open`: `GET /health` with no token returns 200.
- [ ] **Step 2: Write `check.sh`.** It copies `project/` to a temp dir, substitutes `@IDENTITY_MODEL@` with `git -C ~/repos/auth/identity-model archive 9458d60f… | tar -x` into a temp export, runs `uv sync`, starts the Task 1 IdP, and runs the score command twice. The first run uses the starting project and expects **≥5 failures**; the second overlays `reference/app/main.py` and expects **0 failures**.
- [ ] **Step 3: Run `evals/tasks/fastapi-middleware/check.sh`.** Expected: FAIL, because there's no reference yet.
- [ ] **Step 4: Write `reference/app/main.py`, the intended solution.** It uses `TokenValidationMiddleware(app, discovery_url=..., audience=..., excluded_paths=["/health"])`, added during startup or app creation from the env vars, and `/me` uses `CurrentUser`. Use `py/examples/fastapi/app.py` in identity-model as the guide.
- [ ] **Step 5: Rerun `check.sh`.** Expected: starting project fails ≥5 tests, reference passes all 7.
- [ ] **Step 6: Commit** `feat(evals): add the FastAPI middleware task with hidden tests`.

### Task 3: Harness core: plan, round prep, workspace

**Files:**
- Create: `evals/run.mjs` (CLI: `compose`, `call`, `score`), `evals/lib/plan.mjs`, `evals/lib/rng.mjs`, `evals/lib/workspace.mjs`, `evals/lib/telemetry.mjs`, `evals/lib/secrets.mjs`
- Create: `evals/plans/round-1.json`, `evals/README.md`
- Create: `evals/lib/*.test.mjs`
- Modify: `.gitignore` (add `evals/**/node_modules/`)

**Interfaces:**
- Produces:
  - `loadPlan(path: string) -> PlanSpec`: validates and throws with a message naming the bad field.
  - `freezePlan(spec, {codeciteCommit, roundId}) -> Plan`: writes `<roundsDir>/<roundId>/plan.json`; `roundsDir()` is `$CODECITE_EVALS_DIR` or `~/.local/state/codecite-evals`, and it throws when that path is inside a git work tree (`git rev-parse --is-inside-work-tree` succeeds).
  - `expandRuns(plan) -> Run[]`, where `Run = {id, agent, model, arm, rep, seq}`. `id = "<agent>-<model slug with / → _>-<arm>-<rep>"`, and the order is a seeded Fisher–Yates shuffle with `mulberry32(plan.seed)` from `rng.mjs`.
  - `prepareRound(plan, roundDir) -> Round`:
    - exports identity-model at `task.corpus.commit` to `roundDir/identity-model` with `git archive` (so no `.git`), then `chmod -R a-w` it
    - writes `roundDir/corpus.toml` (`name = "eval"`, one `local` source at the export)
    - indexes it with `XDG_DATA_HOME=roundDir/xdg codecite index eval --manifest roundDir/corpus.toml`
    - builds `target/release/codecite` and copies it to `roundDir/bin-codecite/codecite`
  - `prepareRun(run, round) -> RunDirs`, where `RunDirs = {project, home, xdg, bin, out}`:
    - copies the task's `project/` into `out/project` and substitutes `@IDENTITY_MODEL@`
    - runs the task's `setup` before the clock starts
    - codecite arm only: copies `round/xdg` into the run's `xdg` and puts `bin-codecite` on `PATH`
  - `Usage = {input, output, cacheRead, cacheWrite, totalTokens, costUsd}`, with every field `number | null`. `emptyUsage()` returns all `null`. `addUsage(a, b)` is `null`-propagating, so `null + n = null` but `null + null` starting from `emptyUsage()` stays `null`.
  - `readSecrets() -> {openrouter, claudeOauth}` reads `~/.config/codecite/evals.env`, throws naming the missing variable (never its value), and refuses a file readable by group or others.
- `round-1.json` holds:
  - `task: "fastapi-middleware"`, `runs: 20`, `seed: 20261020`, `timeoutSec: 1200`
  - `agents`: `[{agent:"claude", model:"claude-sonnet-5-5"}, {agent:"pi", model:"google/gemini-4-argon"}, {agent:"pi", model:"openai/gpt-6.1-sol"}, {agent:"pi", model:"deepseek/deepseek-v4.1-flash"}]`
  - `arms`: `[{name:"baseline"}, {name:"codecite", codecite:true, note:"<note text>"}]`
  - The note names `codecite search eval "<query>"` and `codecite read eval "<citation>"`, says the corpus `eval` indexes the identity-model source at `{identity_model}`, and is otherwise neutral.

- [ ] **Step 1: Write tests.**
  - `plan.test.mjs`:
    - `rejects a plan without a baseline arm`
    - `expandRuns gives runs × agents × arms runs` (a 20-run, 4-agent, 2-arm plan gives 160 runs)
    - `same seed gives the same order`
    - `ids are unique`
  - `telemetry.test.mjs`:
    - `unknown stays unknown`: `addUsage(emptyUsage(), {input: 5, …null})` gives `input === 5` and `output === null`
  - `secrets.test.mjs`:
    - `refuses a world-readable env file`
    - `error names the variable, not the value`
  - `workspace.test.mjs`, using a temp git repo as a fake identity-model:
    - `export has no .git`
    - `export is read-only`
    - `baseline PATH has no codecite`
    - `placeholder replaced`, `round dir inside a git repo is refused`
- [ ] **Step 2: Run `node --test evals/lib/`.** Expected: FAIL.
- [ ] **Step 3: Implement the interfaces above.** `run.mjs compose --plan evals/plans/round-1.json [--round <id>] [--dry-run]` freezes the plan and prepares the round. `--dry-run` prints the run count per pair and the cost estimate from Task 4, and spends nothing.
- [ ] **Step 4: Run the tests.** Expected: PASS. Then run `node evals/run.mjs compose --plan evals/plans/round-1.json --dry-run` and expect it to print 160 runs.
- [ ] **Step 5: Commit** `feat(evals): add the harness core with frozen plans and isolated runs`.

### Task 4: The call loop with spend and failure guards

**Files:**
- Create: `evals/lib/call.mjs`, `evals/lib/guards.mjs`, `evals/lib/taint.mjs`, `evals/lib/openrouter.mjs`, and tests
- Modify: `evals/run.mjs` (the `call` phase)

**Interfaces:**
- Consumes: `Plan`, `Run`, `prepareRun` and `readSecrets` from Task 3, plus the adapter interface:
  - `adapter.command(run, dirs, ctx) -> {cmd, args, env, cwd}`
  - `adapter.parse(stdout: string, stderr: string) -> AgentResult`
  - `AgentResult = {usage: Usage, turns, toolCalls, codeciteCalls, transcript: string, providerStatus: number|null, init: object|null}`
- Produces:
  - `<round>/runs/<run.id>/` holds `stdout.jsonl`, `stderr.txt`, `result.json`, and `project/`.
  - `result.json` is `{run, status: "ok"|"timeout"|"agent_error"|"infra_error", wallMs, ...AgentResult minus transcript, tainted: string[]}`.
  - `classify(result) -> "fatal"|"infra"|"agent"`:
    - fatal: `providerStatus` is 400, 401, 402 or 404
    - infra: provider status ≥500 with `turns === 0`, or the harness failed to spawn
    - agent: everything else
  - `new CircuitBreaker({min: 10, ratio: 0.8})`, with `.record(ok: boolean)` and `.tripped`.
  - `taintHits(transcript) -> string[]` checks the Global Constraints patterns, looking only at shell commands the agent ran.
  - `keyRemainingUsd(key) -> number|null` calls `GET https://openrouter.ai/api/v1/key` and returns `limit - usage`.
  - `estimateUsd(plan) -> number`: runs × a per-model token guess (300k input, 20k output) × the live OpenRouter price from `/api/v1/models`, with Claude counted as 0.

- [ ] **Step 1: Write tests.**
  - `guards.test.mjs`:
    - `classifies a 404 as fatal`
    - `5xx before any turn is infra`
    - `breaker trips at 8 of 10`
    - `breaker ignores the first 9`
  - `taint.test.mjs`:
    - `flags git clone in a bash call`
    - `ignores github.com inside a file the agent read`
    - `flags a find that reaches evals/tasks`
  - `call.test.mjs`: use a fake adapter whose command is `node fake-agent.mjs` with a mode argument.
    - `timeout kills the agent and marks timeout`
    - `fatal stops the round`
    - `infra failure is retried up to 3 times then recorded`
    - `refuses when the estimate exceeds the key's remaining limit`, with `keyRemainingUsd` stubbed
    - `result.json never contains the key`: grep every written file for the fake key
- [ ] **Step 2: Run the tests.** Expected: FAIL.
- [ ] **Step 3: Implement.** Runs go serially in `seq` order. A run whose `result.json` already has status `ok` or `timeout` is skipped, so `call` resumes. Wall time is from spawn to exit, and the timeout uses `SIGTERM`, then `SIGKILL` 10 s later. The key is passed through `env` only.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit** `feat(evals): run agents with spend, failure and taint guards`.

### Task 5: The Claude Code adapter

**Files:**
- Create: `evals/agents/claude.mjs`, `evals/agents/claude.test.mjs`, `evals/agents/fixtures/claude-stream.jsonl` (recorded from a real one-line run, with secrets scrubbed)

**Interfaces:**
- Consumes: the adapter interface from Task 4.
- The command is:

  ```
  claude -p <prompt> --model <model> --output-format stream-json --verbose
         --strict-mcp-config --mcp-config '{"mcpServers":{}}'
         --setting-sources "" --disallowedTools WebFetch,WebSearch
         --dangerously-skip-permissions --no-session-persistence
  ```

  - `env`: `HOME` and `CLAUDE_CONFIG_DIR` both set to `dirs.home` (empty), `CLAUDE_CODE_OAUTH_TOKEN` from `readSecrets()`, and `PATH` from `dirs.bin` plus the system path. `ANTHROPIC_API_KEY` is explicitly deleted.
  - `cwd`: `dirs.project`.
- Parse:
  - the `system`/`init` event goes to `init`
  - the final `result` event gives `usage` (`input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`), `total_cost_usd` and `num_turns`
  - `tool_use` blocks give `toolCalls`
  - Bash `tool_use` whose command starts with `codecite ` counts toward `codeciteCalls`
  - `totalTokens` is the sum of the four token fields, or `null` if any is missing

- [ ] **Step 1: Record the fixture.** Run the command once with prompt `Reply with ok` in an empty dir, then scrub it.
- [ ] **Step 2: Write tests.**
  - `parses usage from the result event`, with exact numbers from the fixture
  - `missing result event gives null usage` (truncated fixture)
  - `rejects a run with user MCP servers`: when the init event has a non-empty `mcp_servers` or `plugins`, `parse` sets `status` to `infra_error` with reason `"config leaked"`
  - `command never sets ANTHROPIC_API_KEY`
  - `baseline PATH lacks codecite`
- [ ] **Step 3: Run the tests.** Expected: FAIL. **Step 4:** Implement. **Step 5:** Run the tests. Expected: PASS.
- [ ] **Step 6: Live check.** Run `node evals/run.mjs call` on a one-run plan with Claude in each arm. Expected: `result.json` has non-null `totalTokens`, and the init event shows no MCP servers or plugins.
- [ ] **Step 7: Commit** `feat(evals): add the Claude Code adapter`.

### Task 6: The pi adapter

**Files:**
- Create: `evals/agents/pi.mjs`, `evals/agents/pi.test.mjs`, `evals/agents/fixtures/pi-stream.jsonl`

**Interfaces:**
- Consumes: the adapter interface from Task 4.
- The command is:

  ```
  pi -p <prompt> --mode json --provider openrouter --model <slug>
     --no-session --no-mcp --no-extensions --no-skills
     --tools read,bash,edit,write,grep,find,ls
  ```

  Confirm the tool names against `pi --help` when recording the fixture, and use the same list in both arms.
  - `env`: `PI_CODING_AGENT_DIR` set to `dirs.home/pi` (empty), `OPENROUTER_API_KEY` from `readSecrets().openrouter`, `PATH` as in Task 5, and `HOME` set to `dirs.home`.
  - If pi's `models.json` supports per-provider `headers`, write one in the run's agent dir with `X-Title: codecite-evals-<arm>`. Otherwise skip it and note that in the README.
- Parse:
  - Split stdout on `\n`, and drop a final line that doesn't parse.
  - `usage` is the sum of `message_end.message.usage` over assistant messages, with the fields `input`, `output`, `cacheRead`, `cacheWrite`, `totalTokens` and `cost.total`.
  - `toolCalls` counts `tool_execution_start` events. `codeciteCalls` counts the ones with `toolName === "bash"` and `args.command` starting with `codecite `.
  - `providerStatus` is the HTTP status from an error `message_end` (`stopReason: "error"`) when present.

- [ ] **Step 1: Record the fixture** from a real `Reply with ok` run on `deepseek/deepseek-v4.1-flash`. It costs a fraction of a cent.
- [ ] **Step 2: Write tests.**
  - `sums usage across assistant messages`
  - `tolerates a truncated last line`
  - `extracts a 404 as providerStatus` (hand-made fixture line)
  - `agent dir is empty except models.json`
  - `the owner's ~/.pi is never referenced`: no env value or arg contains `/.pi/agent`
- [ ] **Step 3: Run the tests.** Expected: FAIL. **Step 4:** Implement. **Step 5:** Run the tests. Expected: PASS.
- [ ] **Step 6: Live check.** Run a one-run plan per arm on `deepseek/deepseek-v4.1-flash`. Expected: non-null `totalTokens` and `costUsd`, and the codecite key's usage rises.
- [ ] **Step 7: Commit** `feat(evals): add the pi adapter`.

### Task 7: Scoring and the scorecard

**Files:**
- Create: `evals/lib/score.mjs`, `evals/lib/scorecard.mjs`, and tests
- Modify: `evals/run.mjs` (the `score` phase)

**Interfaces:**
- Consumes: `result.json` files (Task 4), `task.json` (Task 2), and the IdP (Task 1).
- Produces:
  - `scoreRun(runDir, task, idpUrl) -> {passed: boolean, failures: string[]}`. It runs the task's `score` command in `runDir/project` with `TEST_IDP=idpUrl` and `{hidden}` set to the task's `hidden/` path, then writes `score.json`.
  - `comparePair(baseline: RunRecord[], codecite: RunRecord[], plannedRuns) -> PairResult`, where `PairResult = {verdict: "PASS"|"STOP"|"INCOMPLETE", correctness, tokens, time}` and each measure is `{baseline, codecite, deltaPct|deltaPts, loss: boolean|null}`.
  - `renderScorecard(round) -> string` (Markdown), written to `<round>/scorecard.md`.
- Rules:
  - The usable runs are every scored run with `status` `ok` or `timeout`, not tainted.
  - If either arm has fewer usable runs than `plannedRuns`, the result is `INCOMPLETE`.
  - Correctness compares pass rates: a loss when codecite is more than 5 points below baseline.
  - Tokens and time compare medians over all usable runs (failed runs included): a loss when codecite's is more than 5% above baseline's.
  - Any `null` median means `loss: null`, and the verdict can't be `PASS`.
  - The round's verdict is `STOP` if any pair is `STOP`, `INCOMPLETE` if any is incomplete, and `PASS` otherwise.
  - The scorecard also shows the IQR, medians over passing runs only, cost, tool calls, codecite-call rate, the retried/tainted run lists, the codecite note text, and the frozen plan.

- [ ] **Step 1: Write tests in `scorecard.test.mjs`**, with synthetic records:
  - `pass when equal`
  - `stop on 6% more tokens`
  - `pass on 4% more time`
  - `stop on 10 points lower pass rate`
  - `incomplete when an arm is short`
  - `null tokens never pass`
  - `tainted runs excluded`
  - `round STOP if any pair STOPs`
- [ ] **Step 2: Write `score.test.mjs`.** Run `scoreRun` against Task 2's reference and starting project with the IdP up. Expected: `passed: true` for the reference, `false` for the starting project. Skip the test when Docker or Node can't start the IdP.
- [ ] **Step 3: Run the tests.** Expected: FAIL. **Step 4:** Implement. **Step 5:** Run the tests. Expected: PASS.
- [ ] **Step 6: Commit** `feat(evals): score runs with hidden tests and render the no-loss scorecard`.

### Task 8: Round one

**Files:**
- Create: `evals/README.md` section "Round one" (how to run, cost seen, link to the issue with the scorecard)

**Interfaces:**
- Consumes everything above.

- [ ] **Step 1: Dry run.** `node evals/run.mjs compose --plan evals/plans/round-1.json --round r1 --dry-run`. Expected: 160 runs and an estimate under the key's remaining limit.
- [ ] **Step 2: Compose, then call in the background.** Run `node evals/run.mjs call --round r1 > ~/.local/state/codecite-evals/r1/call.log 2>&1 &`. It takes hours. Resume with the same command if it stops.
- [ ] **Step 3: Score.** Run `node evals/run.mjs score --round r1`. Expected: `~/.local/state/codecite-evals/r1/scorecard.md` with a verdict.
- [ ] **Step 4: Post `scorecard.md`** as a comment on this issue. If the verdict is `STOP`, say which pair and measure, plainly.
- [ ] **Step 5: Commit** `docs(evals): record how round one ran`, including only the README section, not results.
