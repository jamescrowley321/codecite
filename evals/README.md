# evals: the A/B eval harness

The same coding agent does the same task twice: in a **baseline** arm, and in a
**codecite** arm with the `codecite` CLI on `PATH` and a prebuilt corpus. Hidden
tests score both. See
`docs/superpowers/specs/2026-10-20-ab-eval-harness-design.md` for why.

Node 26, ES modules, no npm dependencies. Tests: `node --test evals/`.

## Layout

| Path | What |
| --- | --- |
| `run.mjs` | The CLI: `compose`, `call`, `score` |
| `plans/` | Plan specs: task, agents, arms, runs, seed, timeout, local repo paths |
| `tasks/<id>/` | A task: `task.json`, `prompt.md`, starting `project/`, `hidden/` tests, `reference/` |
| `fixtures/` | Test services the hidden tests need, such as the IdP |
| `lib/plan.mjs` | Validate, freeze and expand a plan; render each arm's prompt |
| `lib/workspace.mjs` | Prepare a round and each run's isolated directories |
| `lib/telemetry.mjs` | Usage sums where unknown stays `null` |
| `lib/secrets.mjs` | Read the keys from `~/.config/codecite/evals.env` |
| `lib/agent.mjs` | The agent adapter contract (`AgentResult`, `command`, `parse`) and adapter loading |
| `lib/call.mjs` | The call loop: run, retry, time out, redact, record |
| `lib/guards.mjs` | Fatal/infra/agent classification and the circuit breaker |
| `lib/taint.mjs` | Flag runs that fetched code or touched hidden or other runs' paths |
| `lib/openrouter.mjs` | The key's remaining limit and the round's cost estimate |
| `agents/<name>.mjs` | One adapter per agent; the plan's `agent` names it |

## Rounds

A round lives outside any git repo, in `$CODECITE_EVALS_DIR` (default
`~/.local/state/codecite-evals/`), so an agent can't walk up into the hidden
tests. The harness refuses a rounds directory inside a git work tree.

```
node evals/run.mjs compose --plan evals/plans/round-1.json --dry-run
node evals/run.mjs compose --plan evals/plans/round-1.json [--round <id>] [--codecite-bin <path>]
```

`--dry-run` prints the run count per agent/model pair and writes nothing.
Without it, `compose` freezes the plan to `<round>/plan.json` (with the
codecite commit and the task's identity-model commit) and prepares the round:

- `identity-model/`: identity-model at the pinned commit, fetched from the
  local repo named in the plan's `repos` into a throwaway repo and copied out
  of its checkout with no `.git`, read-only.
- `corpus.toml` and `xdg/`: corpus `eval`, indexed once per round from that
  same throwaway repo as a `git` source (codecite won't index a `local`
  source without `.git`), so the export is exactly the indexed tree. The
  throwaway repo and codecite's clone cache are deleted after indexing; no
  file the runs see names the real repo.
- `round.json`: the round's paths and `taintPaths`, the paths a run must not
  touch (the real repos, `plan.json`, `round.json`, `corpus.toml`, `runs/`).
- `bin-codecite/codecite`: built from this checkout with `cargo build
  --release`, or copied from `--codecite-bin`.

Each run gets `runs/<run-id>/` with a fresh `project/` (its
`@IDENTITY_MODEL@` placeholder pointing at the export, the task's `setup`
already run), a clean `home/`, its own `xdg/` and a `PATH`. Both arms' `PATH`
drop every directory that holds a `codecite`, `~/.cargo/bin` and this repo's
`target/`; the codecite arm then prepends `bin-codecite` and gets a copy of
the corpus. The run order is shuffled by the plan's seed.

## Calls

```
node evals/run.mjs call --round <id>
```

`call` runs the round's runs one at a time in `seq` order. Each run's
adapter (`agents/<agent>.mjs`, contract in `lib/agent.mjs`) turns the run
into a command; the agent gets exactly the adapter's environment, in its own
process group, with `SIGTERM` at the plan's `timeoutSec` and `SIGKILL` 10 s
later. `runs/<run-id>/` then holds `stdout.jsonl`, `stderr.txt` and
`result.json`: `{run, status, wallMs, attempts, error, exitCode, signal,
...AgentResult without transcript, tainted}`, where `status` is `ok`,
`timeout`, `agent_error` or `infra_error`. Every secret value is replaced by
`[REDACTED]` before anything is written.

- **Resume:** a run whose `result.json` says `ok` or `timeout` is skipped; any
  other run is redone from a fresh run directory.
- **Fatal:** provider status 400, 401, 402 or 404, or an adapter's
  `rejectReason`, stops the round.
- **Infra:** a spawn or setup failure, or a 5xx before any turn, is retried up
  to 3 times, then recorded as `infra_error`.
- **Breaker:** the round stops once at least 10 attempts are in and 80% or
  more of them ended in anything but `ok` or `timeout`.
- **Spend:** before any run, the OpenRouter estimate for the pending runs
  (300k input and 20k output tokens per run at the model's live price;
  Claude counts as $0) must fit in the key's remaining limit, and the key
  must have a limit. `compose --dry-run` prints the same estimate.
- **Taint:** `tainted` lists shell commands matching `github.com`, `git
  clone`, `curl`, `wget`, `pip install` or `uv add`, and shell commands or
  file-tool paths reaching `evals/tasks`, `test_hidden`, `/reference/`,
  `codecite-worktrees` or a `taintPaths` entry outside the run's own
  directory. Only commands and paths are scanned, never file contents.

## Secrets

`~/.config/codecite/evals.env`, mode 0600, holds `OPENROUTER_EVAL_API_KEY`
and `CLAUDE_CODE_OAUTH_TOKEN`. Only the `call` phase reads it. The harness
refuses the file when group or others can read it, and never prints a value.
