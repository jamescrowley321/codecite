# evals: the A/B eval harness

The same coding agent does the same task twice: in a **baseline** arm, and in a
**codecite** arm with the `codecite` CLI on `PATH` and a prebuilt corpus. Hidden
tests score both. See
`docs/superpowers/specs/2026-10-20-ab-eval-harness-design.md` for why.

Node 26, ES modules, no npm dependencies. Tests: `node --test evals/`.
`call` and its tests need [bubblewrap](https://github.com/containers/bubblewrap)
(`bwrap`, 0.6 or later, usable unprivileged): `apt install bubblewrap`.

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
| `lib/sandbox.mjs` | The bubblewrap sandbox every agent runs in |
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
  touch (the real repos, this codecite checkout, `plan.json`, `round.json`,
  `corpus.toml`, `runs/`).
- `bin-codecite/codecite`: built from this checkout with `cargo build
  --release`, or copied from `--codecite-bin`.

Each run gets `runs/<run-id>/` with a fresh `project/` (its
`@IDENTITY_MODEL@` placeholder pointing at the export, the task's `setup`
already run), a clean `home/`, its own `xdg/` and a `PATH`. Both arms' `PATH`
drop every directory that holds a `codecite`, `~/.cargo/bin` and this repo's
`target/`; the codecite arm then prepends `bin-codecite` and gets a copy of
the corpus. The run order is shuffled by the plan's seed.

## Sandbox

Every agent runs inside `bwrap` (`lib/sandbox.mjs`). There is no
unsandboxed fallback: `call` refuses to start a round when `bwrap` isn't on
`PATH` or a probe of the sandbox's namespace setup fails.

- **Hidden:** the host is mounted read-only, then empty tmpfs mounts cover
  `/home` (and `$HOME` if it's elsewhere), `/root`, `/tmp`, `/var/tmp`,
  `/run`, `$CODECITE_EVALS_DIR`, the round dir and every directory in its
  `taintPaths` (the real repos, this checkout), wherever they live. Each is
  hidden at its real path too, so a `$HOME` that is a symlink (`/home ->
  /var/home`, a home on `/data`) hides its target. So the owner's
  `~/.claude/.credentials.json`, `~/.config/codecite/evals.env`, `~/.pi`,
  `~/.ssh`, the session and system buses, agent sockets, the hidden tests,
  `plan.json`, `round.json` and other runs are not there.
- **Daemon sockets:** all of `/run` is hidden because it holds the system
  bus, systemd's and snapd's sockets, and any container daemon socket that
  appears later. Only the resolver config that `/etc/resolv.conf` points at
  is bound back. A container daemon socket elsewhere (`/var/snap/lxd/...`,
  `/var/lib/lxd/unix.socket`) is covered with `/dev/null`: for a member of
  the `docker`, `lxd` or `incus` group, reaching one is root.
- **Bound back read-write:** only the run's `project/`, `home/` and `xdg/`,
  at their real paths. `/tmp` is a fresh, empty tmpfs per run. The run dir's
  other files (`stdout.jsonl`, `setup.log`) are not visible.
- **Bound back read-only:** the round's `identity-model/` export,
  `bin-codecite/` (codecite arm only: the arms' sandboxes differ by that one
  bind), and what the commands need, found from the command itself: its
  real path and every symlink on the way (`~/.local/bin/claude` ->
  `~/.local/share/claude/versions/<v>`); for an ELF, its loader and the
  libraries the loader lists (Homebrew's node needs about 50 under
  `/home/linuxbrew`), and for a libcrypto, the `openssl.cnf`, `cert.pem` and
  `certs/` of the OPENSSLDIR compiled into it (not its `private/`), without
  which Homebrew's node can't verify TLS; for a script, its shebang interpreter (`env node` is
  looked up on the run's `PATH`) and its node package (pi's
  `@earendil-works/pi-coding-agent`). The same goes for the task's setup
  command (`uv`) and the project venv's base Python (from `pyvenv.cfg`), so
  the agent can run the project. Nothing else under any hidden dir is bound.
- **Namespaces:** `--unshare-pid`, `--unshare-ipc`, `--unshare-uts`,
  `--new-session` (no TIOCSTI into the owner's terminal) and
  `--die-with-parent`. The network is shared: agents need their provider's
  API.
- **Environment:** bwrap is spawned with exactly the adapter's env and
  passes it on, plus the `PWD` its `--chdir` sets. Values never go in
  bwrap's argv (`--setenv`), which `/proc` shows every local user.
- **Kill:** on timeout or an early reject the call loop sends `SIGTERM` to
  every process inside the sandbox (found under bwrap in `/proc`: the agent
  has its own session, out of bwrap's group) but not to bwrap, which would
  die at once and take the agent with it. `SIGKILL` goes to bwrap's group 10 s
  later; killing bwrap ends its pid namespace and everything left in it.

Taint scanning stays as a second layer: it catches a run that tried to reach
the hidden paths or fetch code, even though the sandbox kept them out.

## Calls

```
node evals/run.mjs call --round <id> [--retry-fatal]
```

`call` runs the round's runs one at a time in `seq` order. Each run's
adapter (`agents/<agent>.mjs`, contract in `lib/agent.mjs`) turns the run
into a command; the agent runs in the sandbox above with exactly the
adapter's environment, with `SIGTERM` at the plan's `timeoutSec` and `SIGKILL` 10 s
later. `runs/<run-id>/` then holds `stdout.jsonl`, `stderr.txt` and
`result.json`: `{run, status, wallMs, attempts, error, exitCode, signal,
...AgentResult without transcript, tainted}`, where `status` is `ok`,
`timeout`, `agent_error`, `infra_error` or `fatal`. Every secret value is replaced by
`[REDACTED]` before anything is written.

- **Resume:** a run whose `result.json` says `ok` or `timeout` is skipped; any
  other run is redone from a fresh run directory. A round holding a `fatal`
  run refuses to resume until you fix the cause and pass `--retry-fatal`,
  which re-runs the fatal runs first.
- **Fatal:** provider status 401, 402 or 404, a 400 before any turn, or an
  adapter's `rejectReason` (from the init event, or `earlyReject` on a live
  stdout line, which kills the agent at once) is recorded as `fatal` and
  stops the round. A 400 after a turn is the run's own `agent_error`.
- **Infra:** a spawn or setup failure (a missing agent command, or bwrap
  failing before the agent starts), a 5xx before any turn, or an adapter's
  `endedOnError` before any turn (any status, or none), is retried up to 3
  times, then recorded as `infra_error`. `endedOnError` after a turn is
  `agent_error`, even when the agent exited 0.
- **Breaker:** the round stops once at least 10 attempts are in and 80% or
  more of them ended in anything but `ok` (timeouts count as failures here,
  though resume still skips them).
- **Spend:** before any run, the OpenRouter estimate for the pending runs
  (300k input and 20k output tokens per run at the model's live price;
  Claude counts as $0) must fit in the key's remaining limit, and the key
  must have a limit. `compose --dry-run` prints the same estimate.
- **Taint:** `tainted` lists shell commands matching `github.com`, `git
  clone`, `curl`, `wget`, `pip install` or `uv add`, and shell commands or
  file-tool paths naming `evals/tasks`, `test_hidden`, `/reference/` or
  `codecite-worktrees`. Each command is split into segments on `&&`, `||`,
  `;`, `|` and newlines, with a cwd that starts at the project and follows
  `cd`. Every word (and each non-`cd` segment's cwd) is resolved against it,
  as is every file-tool path. A path taints when it is inside a `taintPaths`
  entry (the real repos, this checkout, `plan.json`, `round.json`,
  `corpus.toml`, `runs/`), or equals or contains the round dir or a
  `taintPaths` entry (`ls ../../..`, `find /`), unless it is inside the
  run's own directory. Only commands and paths are scanned, never file
  contents. The scan is lexical: a path built from `$VAR`, `$(...)`, globs
  or a script the agent wrote is not resolved, so tainting is detection
  only; the sandbox is what keeps those paths out of reach.

## Secrets

`~/.config/codecite/evals.env`, mode 0600, holds `OPENROUTER_EVAL_API_KEY`
and `CLAUDE_CODE_OAUTH_TOKEN`. Only the `call` phase reads it. The harness
refuses the file when group or others can read it, and never prints a value.

When `CLAUDE_CODE_OAUTH_TOKEN` is unset, the `claude` adapter falls back to
`claudeAiOauth.accessToken` in the owner's `~/.claude/.credentials.json`, read
fresh for each run and never written. It passes the token to `claude -p` as
`CLAUDE_CODE_OAUTH_TOKEN` and lists it in its spec's `secrets`, so the call
loop redacts it from `stdout.jsonl`, `stderr.txt` and `result.json` like the
env file's values.

Both files are read by the harness, outside the sandbox. Inside it, neither
exists: an agent holds only the one access token (or key) in its own
environment, never the refresh token or the other agent's key.

## Agents

### pi

`agents/pi.mjs` runs `pi -p --mode json --provider openrouter --model <slug>`
with `--no-session --no-mcp --no-extensions --no-skills
--no-prompt-templates --no-themes --no-approve --tools
read,bash,edit,write,grep,find,ls` (the same in both arms), the prompt last
after `--`. Its environment is built from nothing: `HOME` is the run's
`home/`, `PI_CODING_AGENT_DIR` is `home/pi`, `OPENROUTER_API_KEY` is the evals
key, `PI_TELEMETRY=0` and `PI_SKIP_VERSION_CHECK=1`. The agent dir holds only
a `models.json` setting OpenRouter's `X-Title` header to
`codecite-evals-<arm>`, so spend shows per arm on the OpenRouter dashboard
(`PI_TELEMETRY=0` also stops pi sending its own attribution headers).

pi reports what it loaded in the session's system message: the system
prompt's `sections` and `toolsAdded`. The run is rejected (`config leaked`,
fatal, killed at once by `earlyReject`) when that message has a section
other than `preamble`, `tools`, `rules`, `docs`, `cwd` and `project_context`
(for example `skills`, or `addendum` from an `APPEND_SYSTEM.md`), a tool
outside the list above, or a context file (`AGENTS.md`, `CLAUDE.md`) outside
the run's cwd. A replaced system prompt (`SYSTEM.md`) is not detectable from
the stream; the empty agent dir and `--no-approve` are what keep it out.

Usage is the sum of each assistant `message_end`'s `usage`; `turns` counts
assistant messages that didn't end in error; `providerStatus` is the status
at the head of the last assistant message's `errorMessage` when the run
ended on an error. pi exits 0 even when the provider ended the run, so the
adapter sets `endedOnError` whenever the last assistant message ended in
error: a fatal status stays fatal, an error before any turn (a 429, a
connection error) is infra and retried, and one after a turn is
`agent_error`, never `ok`.
