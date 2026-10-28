# evals

Two benchmarks decide whether codecite earns its place (`docs/design.md`,
Evaluation plan):

- **A/B agent eval** (`inspect/`): the same coding agent does the same task
  twice, in a **baseline** arm and in a **codecite** arm with codecite's
  `search` and `read` as native tools, the `codecite` CLI on `PATH` and a
  prebuilt corpus. Hidden tests and deterministic checks
  score both. Runs on [Inspect AI](https://inspect.aisi.org.uk/).
- **Retrieval benchmark** (`retrieval/`): labelled queries run through
  `codecite-bench`; CI runs it on every PR.

## Layout

| Path | What |
| --- | --- |
| `src/codecite_evals/` | The `codecite-evals` package that `uv sync` installs: `task.py` (the code tasks: one sample per arm, pi in a Docker sandbox, the scorer), `spec_task.py` (the spec-question task: one sample per question per arm, on the same pi), `report.py` (the scorecard, or the calibration table), `frozen.py` (the frozen-task check) and `config.py` (where every file below is found) |
| `tests/` | Unit tests for the task's agent, the scorecard, the frozen-task check and the config; no model key or Docker needed |
| `inspect/prepare.sh` | Stage the sandbox image's build context in `inspect/build/` for one task (`prepare.sh <task-id>`, default `fastapi-middleware`) |
| `inspect/Dockerfile`, `inspect/compose.yaml` | The sandbox image and the IdP beside it, on a network with no route out |
| `inspect/compose-offline.yaml` | The sandbox alone with no network, for a task that needs no IdP (spec-questions) |
| `inspect/pi-models.json` | Points pi at Inspect's agent bridge |
| `inspect/panel.sh` | Run one tier (`quick` or `full`): every model in one Inspect process, then the report |
| `tasks/<id>/` | A task: `task.json` (with `frozen_at` once frozen), `prompt.md`, starting `project/`, `hidden/` tests, `checks.py`, `reference/`, and `check.sh`/`planted.sh` to prove the task and checks are sound |
| `tasks/spec-questions/` | The spec-question task: `questions.toml` (labelled questions with closed answers, checked against the index by `src/codecite_evals/spec_questions.py`), `prompt.md`, the in-sandbox scorer `score.py`, and `check.sh`/`planted.py` to prove the scorer sound |
| `fixtures/node-oidc/` | The IdP the fastapi-middleware hidden tests take tokens from |
| `retrieval/` | The retrieval benchmark's queries and corpus manifest |

## Tasks

- **`fastapi-middleware`**: protect a FastAPI app with identity-model's
  `TokenValidationMiddleware` and `CurrentUser`. The hidden tests take real
  tokens from node-oidc-provider. The corpus is identity-model. The prompt
  names the symbols, so it tests little search.
- **`client-auth-uri`**: a FastAPI service's hand-rolled token endpoint takes
  client credentials from HTTP Basic, the form body and the query string. The
  prompt says only that a security review flagged it: fix what the OAuth spec
  forbids, keep what it allows, and cite the section in a comment. The answer
  is RFC 6749 §2.3.1: credentials MUST NOT be in the request URI, HTTP Basic
  MUST be supported, and the form body is allowed. The hidden tests reject
  query-string credentials, even valid ones, with `invalid_client` or
  `invalid_request` (both readings of §5.2); reject HTTP Basic plus a
  `client_secret` in the body, since a client MUST NOT use more than one
  authentication method in a request (§2.3); and
  still issue tokens over Basic and the form body; an agent that removes too
  much fails. `checks.py` passes only when a comment or docstring in `app/`
  cites §2.3.1 or §2.3 of RFC 6749, the sections of its two fixes
  (`rfc6749#section-2.3.1`, `RFC 6749 §2.3` or `Section 2.3.1 of RFC 6749`,
  alone or in a list of sections). The agent may
  add Python tests of its own (`test_*.py`, `*_test.py`, and a `conftest.py` or `__init__.py` beside them), but no `conftest.py` at the project root. The corpus is identity-model plus the spec
  documents of `retrieval/identity-bench.toml` (`task.json`'s
  `corpus.documents`), and the codecite arm's note says so. It needs no IdP,
  so `planted.sh` runs the hidden tests itself, and `check.sh` just runs it.
- **`backchannel-logout`**: a relying-party app's hand-rolled
  `POST /backchannel-logout` checks only a logout token's signature and
  audience. The prompt says only that the endpoint accepts tokens it should
  reject and must conform to the spec it implements; it names no
  identity-model symbol and no spec section. An agent can find identity-model's
  logout-token validation and call it, or find Back-Channel Logout 1.0 §2.6
  and implement it; both pass. The corpus is all of identity-model plus the
  spec documents of `retrieval/identity-bench.toml`, at their pinned commit
  and `sha256`. Hidden tests mint logout tokens with a test key and serve
  discovery and JWKS from a stub in their conftest, so the task needs no IdP:
  a valid token (with `sub` and `sid`, `sid` only, `sub` only) ends the
  session, and a bad signature, wrong `iss` or `aud`, a `nonce`, a missing or
  malformed `events`, neither `sub` nor `sid`, or a missing `iat` or `jti` is
  rejected with 400 and leaves the session alone. `checks.py` fails a new
  dependency, a change outside `app/` other than docs and the agent's own test
  modules, and a provider URL written into `app/`. Its `reference/` holds a
  solution that calls identity-model and a hand-written one.

- **`spec-questions`**: one question about the identity specs per sample,
  asked in an agent's words rather than the spec's (`questions.toml`, from
  the `spec-plain` and `spec-requirement` classes of
  `retrieval/queries.toml` or labelled anew; see `tasks/spec-questions/`).
  The current spec documents of `retrieval/identity-bench.toml` sit in
  `/opt/specs/<document>.html` in both arms, copied from the bytes codecite
  indexed and checked against their pinned `sha256`; the `eval` corpus is
  identity-model plus those documents, so codecite's citations name the same
  documents as the labels. The agent writes `answer: <value>` and `section:
  <document>#section-<number>` to `/work/answer.txt`. `score.py` passes a
  sample only when the answer is one of the question's accepted forms
  (ignoring case, quotes and a final full stop) and the section is an
  accepted one; a step of an accepted section counts, and codecite's
  `@sha256:` pin is allowed. A missing file, a third line or prose fails.
  The task has no project and no IdP: its sandbox has no network at all
  (`inspect/compose-offline.yaml`). One epoch is a run per question, so its
  `task.json` sets 1 epoch on the quick tier and 3 on the full.

Each task's `check.sh` proves it is well-formed; `backchannel-logout`'s needs
only uv and an identity-model checkout, and `spec-questions`' only python3
(it runs `score.py` on every question's reference answer and on planted
wrong ones). CI runs both (`eval-task`):

```
IDENTITY_MODEL_REPO=~/repos/auth/identity-model evals/tasks/backchannel-logout/check.sh
```

Each task is registered in `src/codecite_evals/_registry.py` and runs as
`codecite_evals/<task>` with `_` for `-`. The sandbox image holds one task's
project and corpus, so `inspect/prepare.sh <task-id>` stages it before the
image is built; after switching tasks, rebuild it (`docker compose -f
inspect/compose.yaml build`). `panel.sh` runs the task `TASK_ID` names
(default `fastapi-middleware`).
Every task takes `-T arms=baseline` to run the baseline arm alone, a
calibration (see below).

## Two tiers

The A/B eval runs at one of two tiers. They share the task, the scorer and the
report; only the panel and the number of epochs differ.

| Tier | Panel | Epochs per sample | Takes | Use it |
| --- | --- | --- | --- | --- |
| `quick` | claude-haiku-5.5, claude-sonnet-5.5, gpt-6.1-sol | 5; spec-questions 1 | under 20 minutes in CI for a code task | on every PR that touches indexing, search, ranking or the CLI's output |
| `full` | all 9 models | 10; spec-questions 3 | within CI's 6-hour job limit | before a milestone closes, after a quick loss, or when the task set changes |

A code task has one sample per arm, so its runs per arm are its epochs. The
spec-question task has one sample per question per arm, so its runs per arm
are questions × epochs: 34 on the quick tier, 102 on the full. A task's
`task.json` sets its own epochs (`"epochs": {"quick": 1, "full": 3}`), and
`EPOCHS` overrides both.

The quick verdict is directional, never the gate: at 5 runs per arm its
intervals are wide, so a quick loss means "run the full tier", not "stop". The
full tier is the gate in `docs/design.md` (Evaluation plan). The quick panel's
third model is the fastest pi model by round one's medians; change it in
`panel.sh` when a full run's medians say another is faster.

### In CI

`.github/workflows/evals.yml` runs both tiers. It is advisory, like `bench`,
and never a required check.

- **Quick:** add the `eval` label to a PR. The tier runs then and again on
  every push while the label is on, one job per task (`fastapi-middleware`
  and `spec-questions`, the eval job's matrix), and their scorecards go to
  one PR comment, updated in place, a table per task. A new push cancels a
  stale run. Fork PRs never run: they get no secrets.
- **Full:** run the workflow by hand (Actions → Evals → Run workflow) with
  `tier: full`, the `task` to run and `arms` (`ab`, the default; `aa`; or
  `baseline` to calibrate; see below). Its scorecard goes to the job summary.
  To finish a run that stopped part way, run it again with `resume` set to
  that run's id: it downloads that run's logs and reruns only their missing
  samples with `inspect eval-retry`.
- **A/A:** dispatch with `arms: aa` to run an A/A round at either tier, and
  `epochs` to run more epochs than the tier's own (a quick tier at more than
  its 5 epochs gets the full tier's time limits).

The job builds codecite, fetches identity-model at the task's pinned commit
and caches the two sandbox images, keyed on a hash of their build contexts
(which hold the codecite binary and the identity-model commit), so an
unchanged binary doesn't rebuild. It reads only the `OPENROUTER_EVAL_API_KEY`
secret. The Inspect logs, with the scorecard, upload as the
`eval-logs-<tier>-<task>` artifact (`eval-logs-<tier>-aa-<task>` for an A/A
round, `eval-logs-calibration-<task>` for a calibration).

## A/A rounds

An A/A round runs the task with two baseline arms, `baseline-a` and
`baseline-b`, with the same prompt and the same sandbox. Whatever separates
them is noise, so its intervals say how wide the A/B intervals would be with
no effect at all, at that number of epochs. `-T arms=aa` selects it (the
default, `ab`, is baseline against codecite); `ARMS=aa inspect/panel.sh quick`
runs a tier that way into `inspect/logs/<tier>-aa/<task>/`. The scorecard heads the
round as A/A and gives it no verdict: it names each measure whose interval
noise alone pushed past the 5% margin.

## The scorecard

`codecite-eval-report` prints three tables per task (a calibration prints one):

- **Per arm:** runs, how many called codecite (`used codecite`), pass rate, failures, timeouts, sandbox errors, median
  total tokens and median wall time, and the model's verdict.
- **From the transcripts:** the median turns (model calls that returned),
  output tokens, model time (the sum of the calls' generation times), other
  time (wall time less model time: tools, sandbox and waiting), and decode
  rate (output tokens per second of model time), normalised time (wall time
  with the model time recomputed at the model's decode rate over both arms,
  so a slower host for one arm drops out), with the upstream providers
  OpenRouter says served the calls, counted. A provider or decode-speed gap
  between arms shows here. The provider is in each call's raw response,
  which Inspect keeps only with `--log-model-api` (`panel.sh` passes it);
  calls without one count as `not logged`.
- **Intervals:** the 95% bootstrap interval of every measure, second arm
  against the first, with `*` on each that reaches past 5% either side of 0.

The verdict uses pass rate, total tokens and wall time (`docs/design.md`,
Success criteria). Logs whose samples kept no model events leave the
transcript measures blank.

## Running the A/B eval locally

Needs Docker, [uv](https://docs.astral.sh/uv/), a release build of codecite
and an identity-model checkout (`IDENTITY_MODEL_REPO`, default
`~/repos/auth/identity-model`).

```
cargo build --release
evals/inspect/prepare.sh             # or: evals/inspect/prepare.sh spec-questions
cd evals
uv sync --locked
set -a; . ~/.config/codecite/evals.env; set +a
export OPENROUTER_API_KEY="$OPENROUTER_EVAL_API_KEY"
uv run --locked inspect eval codecite_evals/fastapi_middleware --model openrouter/anthropic/claude-haiku-5.5 -M strict_tools=false --epochs 2
inspect/panel.sh quick               # or: TASK_ID=spec-questions inspect/panel.sh full
uv run --locked codecite-eval-report --tier quick
```

The package registers its tasks with Inspect through the `inspect_ai` entry
point, so Inspect runs them by name from any directory: `codecite_evals/ab`
takes the task's id (`-T task_id=<id>`) and its arms (`-T arms=ab|aa|baseline`),
and each registered task also runs by its own name.
`codecite-eval-report` is the scorecard's command. Both find `tasks/`,
`inspect/compose.yaml` and `inspect/logs/` through
`src/codecite_evals/config.py`: under this `evals/` directory, or under
`CODECITE_EVALS_DIR` when it is set.

The unit tests in `tests/` run offline, and CI runs them on every PR:
`uv run --locked pytest`.

Each tier's logs land in `inspect/logs/<tier>/<task>/` (gitignored), with the
last scorecard in `scorecard.md`. `codecite-eval-report --tier <tier>` reads
every task's logs and prints a table for each, so tasks never pool, and `uv
run --locked inspect view` browses them. Being ignored, they go with the
worktree: copy them out before `git worktree
remove`, or pass `--log-dir` a directory outside the checkout. A full run
takes an hour or more: run it detached from your shell (for example
`systemd-run --user --unit=codecite-panel -p MemoryMax=16G inspect/panel.sh
full`), since a killed run leaves its sandboxes behind and its log unfinished.
`RETRY=1 inspect/panel.sh full` reruns only the unfinished logs' missing
samples (`inspect eval-retry`). `panel.sh` raises Inspect's subprocess and docker CLI limits to twice
`MAX_SANDBOXES`, so no sample waits on them. `EPOCHS` and `MAX_SANDBOXES` override a
tier's defaults; arguments after the tier go to `inspect eval`.

`panel.sh` runs every model of a tier in one Inspect process, so the slow
models no longer add up one after another. Each sandbox creates its own Docker
network, and Docker's default address pools hold about 31, so `MAX_SANDBOXES`
defaults to 14; past the pools, sandbox starts fail with "all predefined
address pools have been fully subnetted" and take the model's whole run with
them (#111). CI gives the daemon /24 pools out of `10.200.0.0/16` (256
networks) and runs 8 sandboxes at once, Inspect's own default for the
runner's 4 vCPU. Past what the CPUs carry, samples wait on one another, and
the wait counts in their wall time: at 20 sandboxes the medians were ~314 s
against ~70 s of working time. To do the same locally, add
`"default-address-pools": [{"base": "10.200.0.0/16", "size": 24}]` to
`/etc/docker/daemon.json` and restart Docker.

## Calibrating a new task

A task has to be hard enough that the arms can differ on correctness: one
every model passes in both arms can't show codecite helping. Getting it there
means running it and adjusting it, and adjusting it while watching codecite's
arm could tune it until codecite wins. So calibrate on the baseline arm only,
then freeze the task before its first A/B run (`docs/design.md`, Agent
benchmark).

1. Add `tasks/<id>/` with a new id, and prove it with its `check.sh` and
   `planted.sh`. Leave `frozen_at` out of its `task.json`.
2. Run the baseline arm on the quick panel: dispatch the Evals workflow with
   `task: <id>` and `arms: baseline` (tier `quick`), or locally
   `evals/inspect/prepare.sh <id>`, rebuild the image, then `TASK_ID=<id>
   ARMS=baseline evals/inspect/panel.sh quick`. Logs land in
   `inspect/logs/calibration/<id>/` (the `eval-logs-calibration-<id>` artifact
   in CI). The calibration table gives
   each model's pass rate, median turns, tokens and time, the pooled pass
   rate and whether the task is in band. It has no verdict and is never a
   pass.
3. The task is in band when baseline passes 40–80% of runs pooled over the
   quick panel's models and no model passes every run. Too easy or too hard:
   adjust the prompt, project, hidden tests or checks and calibrate again.
   Never run the codecite arm while calibrating.
4. Freeze it: set `frozen_at` in its `task.json` to the commit its first A/B
   run runs at, in the PR that adds the task to the panel. From then on CI's
   frozen-task check (`uv run --locked codecite-eval-frozen`) fails any PR
   that changes a file under `tasks/<id>/` or moves `frozen_at`. A task that
   needs changing after that is a new task: copy it to a new id and
   calibrate that.

## How a sample runs

- The sandbox image holds pi, the task's project with its dependencies
  installed, identity-model's source at the task's pinned commit, and codecite
  with the `eval` corpus already indexed. A spec-question sample instead
  starts in an empty `/work/project` with the spec documents in `/opt/specs`.
  The baseline arm deletes codecite
  before the agent starts and runs pi with `--no-mcp`. The codecite arm runs
  `codecite mcp` through pi's built-in MCP support (`-e builtin:mcp`, with the
  server's exposure `direct`), so `mcp__codecite__search` and
  `mcp__codecite__read` sit in pi's tool list next to `grep` and `find`, with
  codecite's descriptions; it keeps codecite on `PATH` too, and its prompt
  gets a note naming both. Otherwise the arms are identical.
- The scorecard's `used codecite` column counts, per arm, the runs with at
  least one call to codecite, read from the model calls in the Inspect log: a
  native tool call, or a `bash` call that runs `codecite`. Text that only
  names codecite doesn't count. A model whose codecite arm used it in fewer
  than half its usable runs gets the verdict `not measured`: the arms then
  differ by a tool the agent never touched.
- The sandbox shares the IdP's network namespace and has no route to the
  internet. pi's model calls leave through Inspect's agent bridge, so Inspect
  records every call and its tokens, and the agent never holds a key.
- OpenRouter picks the host within the account's privacy settings (ZDR and
  its allowed providers); the panel pins none.
- The hidden tests and checks reach the sandbox only after the agent exits. A
  sample passes when the hidden tests and every check pass. For a spec
  question, the expected answer and accepted sections reach it only then too,
  with `score.py`, which reads `/work/answer.txt`; the sample passes when both
  the answer and the section are right. A spec question's sandbox has no
  network at all, not even an IdP's.
- An agent that runs past 600 s, or whose sandbox fails mid-run, fails: what
  it left behind is scored, but the sample can't pass. The scorecard counts
  the two apart: `timeouts` for the agent running out of time, `sandbox
  errors` for the sandbox failing under it, each listed under the table with
  its model, arm and epoch and named in that model's verdict. pi's output
  overflowing Inspect's stream limit or failing to decode is the agent's
  failure, in neither column. Hidden tests or checks that hang fail too. A
  provider error that pi reports is infrastructure: the sample errors and
  Inspect retries it (`--retry-on-error 2`); errored samples are never
  scored.
