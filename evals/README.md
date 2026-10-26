# evals

Two benchmarks decide whether codecite earns its place (`docs/design.md`,
Evaluation plan):

- **A/B agent eval** (`inspect/`): the same coding agent does the same task
  twice, in a **baseline** arm and in a **codecite** arm with the `codecite`
  CLI on `PATH` and a prebuilt corpus. Hidden tests and deterministic checks
  score both. Runs on [Inspect AI](https://inspect.aisi.org.uk/).
- **Retrieval benchmark** (`retrieval/`): labelled queries run through
  `codecite-bench`; CI runs it on every PR.

## Layout

| Path | What |
| --- | --- |
| `inspect/task.py` | The Inspect task: one sample per arm, pi in a Docker sandbox, the scorer |
| `inspect/prepare.sh` | Stage the sandbox image's build context in `inspect/build/` |
| `inspect/Dockerfile`, `inspect/compose.yaml` | The sandbox image and the IdP beside it, on a network with no route out |
| `inspect/pi-models.json` | Points pi at Inspect's agent bridge |
| `inspect/panel.sh` | Run every model in the panel, one at a time, then the report |
| `inspect/report.py` | The scorecard: per model, both arms and the no-loss verdict |
| `tasks/<id>/` | A task: `task.json`, `prompt.md`, starting `project/`, `hidden/` tests, `checks.py`, `reference/`, and `check.sh`/`planted.sh` to prove the task and checks are sound |
| `fixtures/node-oidc/` | The IdP the hidden tests take tokens from |
| `retrieval/` | The retrieval benchmark's queries and corpus manifest |

## Tasks

- **`fastapi-middleware`:** protect a small FastAPI app with identity-model's
  `TokenValidationMiddleware`. The prompt names the package and the classes to
  use; hidden tests take real tokens from the IdP (`fixtures/node-oidc/`).
- **`backchannel-logout`:** a relying-party app's hand-rolled
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

Each task's `check.sh` proves it is well-formed; `backchannel-logout`'s needs
only uv and an identity-model checkout, and CI runs it (`eval-task`):

```
IDENTITY_MODEL_REPO=~/repos/auth/identity-model evals/tasks/backchannel-logout/check.sh
```

The harness (`inspect/task.py`, `inspect/prepare.sh`) runs
`fastapi-middleware`; choosing the task, and adding a task's spec documents to
the sandbox corpus, come with #127.

## Running the A/B eval

Needs Docker, [uv](https://docs.astral.sh/uv/), a release build of codecite
and an identity-model checkout (`IDENTITY_MODEL_REPO`, default
`~/repos/auth/identity-model`).

```
cargo build --release
evals/inspect/prepare.sh
cd evals
uv sync --locked
set -a; . ~/.config/codecite/evals.env; set +a
export OPENROUTER_API_KEY="$OPENROUTER_EVAL_API_KEY"
uv run --locked inspect eval inspect/task.py --model openrouter/anthropic/claude-haiku-5.5 -M strict_tools=false --epochs 2
inspect/panel.sh                     # the whole panel, EPOCHS=10 by default
uv run --locked python inspect/report.py
```

Logs land in `inspect/logs/` (gitignored); `uv run --locked inspect view`
browses them. Being ignored, they go with the worktree: copy them out before
`git worktree remove`, or pass `--log-dir` a directory outside the checkout.
A panel takes hours: run it detached from your shell (for example
`systemd-run --user --unit=codecite-panel -p MemoryMax=16G inspect/panel.sh`),
since a killed run leaves its sandboxes behind and its log unfinished.
`inspect eval-retry <log>` reruns only an unfinished log's missing samples.

Several models can run in one `inspect eval` process (a comma-separated
`--model`). Each sandbox creates its own Docker network, and Docker's default
address pools hold about 31, so keep `--max-sandboxes` under that (14 works);
past it, sandbox starts fail with "all predefined address pools have been fully
subnetted" and take the model's whole run with them (#111).

## How a sample runs

- The sandbox image holds pi, the task's project with its dependencies
  installed, identity-model's source at the task's pinned commit, and codecite
  with the `eval` corpus already indexed. The baseline arm deletes codecite
  before the agent starts; the codecite arm gets it on `PATH` and a note in
  the prompt. Otherwise the arms are identical.
- The sandbox shares the IdP's network namespace and has no route to the
  internet. pi's model calls leave through Inspect's agent bridge, so Inspect
  records every call and its tokens, and the agent never holds a key.
- OpenRouter picks the host within the account's privacy settings (ZDR and
  its allowed providers); the panel pins none.
- The hidden tests and checks reach the sandbox only after the agent exits. A
  sample passes when the hidden tests and every check pass.
- An agent that runs past 600 s, or whose sandbox fails mid-run, fails: what
  it left behind is scored, but the sample can't pass. Hidden tests or checks
  that hang fail too. A provider error that pi reports is infrastructure: the
  sample errors and Inspect retries it (`--retry-on-error 2`); errored samples
  are never scored.
