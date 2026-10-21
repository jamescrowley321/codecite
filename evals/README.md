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
| `lib/idp.mjs` | Start the eval IdP for the score phase |
| `lib/score.mjs` | Run the hidden tests against each run's project, sandboxed |
| `lib/pytest-plugin/` | The `-p` plugin that marks pytest as started |
| `lib/scorecard.mjs` | The no-loss comparison (bootstrap intervals) and the Markdown scorecard |
| `agents/<name>.mjs` | One adapter per agent; the plan's `agent` names it |

## Rounds

A round lives outside any git repo, in `$CODECITE_EVALS_DIR` (default
`~/.local/state/codecite-evals/`), so an agent can't walk up into the hidden
tests. The harness refuses a rounds directory inside a git work tree.

```
node evals/run.mjs compose --plan evals/plans/round-1.json --dry-run
node evals/run.mjs compose --plan evals/plans/round-1.json [--round <id>] [--codecite-bin <path>]
```

`compose` first checks every non-Claude model slug against OpenRouter's live
model list (no key needed) and refuses a plan naming one it doesn't list.
`--dry-run` then prints the run count per agent/model pair and the cost
estimate, and writes nothing. Without it, `compose` freezes the plan to
`<round>/plan.json` (with the codecite commit and the task's identity-model
commit) and prepares the round:

- `task/`: a copy of the task dir (`task.json`, `prompt.md`, `project/`,
  `hidden/`, `reference/`), which the frozen plan's `taskDef.dir` points at.
  Runs and scoring read the task from here, never from the live checkout, so
  editing `evals/tasks/` after compose doesn't change a round.
- `idp/`: a copy of the task's IdP fixture (`fixtures/node-oidc`, with its
  installed `node_modules`; run `npm ci` there before compose), which `score`
  starts instead of the live one.
- `identity-model/`: identity-model at the pinned commit, fetched from the
  local repo named in the plan's `repos` into a throwaway repo and copied out
  of its checkout with no `.git`, read-only.
- `corpus.toml` and `xdg/`: corpus `eval`, indexed once per round from that
  same throwaway repo as a `git` source (codecite won't index a `local`
  source without `.git`), so the export is exactly the indexed tree. The
  throwaway repo and codecite's clone cache are deleted after indexing; no
  file the runs see names the real repo.
- `round.json`: the round's paths and `taintPaths`, the paths a run must not
  touch (the real repos, this codecite checkout, `task/`, `plan.json`,
  `round.json`, `corpus.toml`, `runs/`).
- `bin-codecite/codecite`: built from this checkout with `cargo build
  --release`, or copied from `--codecite-bin`.

Each run gets `runs/<run-id>/` with a fresh `project/`, a clean `home/`, its
own `xdg/` and a `PATH`. Inside the sandbox they are at fixed, neutral paths,
the same for every arm, run and round:

| Host | In the sandbox |
| --- | --- |
| `runs/<run-id>/project/` | `/work/project` (the agent's cwd) |
| `runs/<run-id>/home/` | `/work/home` (`HOME`) |
| `runs/<run-id>/xdg/` | `/work/xdg` (`XDG_DATA_HOME`) |
| `identity-model/` | `/work/identity-model` (read-only) |
| `bin-codecite/` | `/work/bin` (read-only, codecite arm only) |

So the agent's cwd (which both agents put in their system prompt), its
environment, `PATH`, prompt and pi's `models.json` never name its arm, its
run, the round or the evals dir. The project's `@IDENTITY_MODEL@` placeholder
and the prompt's `{identity_model}` are `/work/identity-model`. The task's
`setup` (`uv sync`) runs before the clock starts in the same sandbox, at the
same paths, so the venv it builds is valid there; a `uv` setup gets the
host's Python (the one `uv python find` picks, bound read-only) and a
per-round uv cache (`<round>/uv-cache`, bound for setup only). Both arms'
`PATH` drop every directory that holds a `codecite`, `~/.cargo/bin` and this
repo's `target/`; the codecite arm then prepends `/work/bin` and gets a copy
of the corpus. The run order is shuffled by the plan's seed.

## Sandbox

Every agent runs inside `bwrap` (`lib/sandbox.mjs`). There is no
unsandboxed fallback: `call` refuses to start a round when `bwrap` isn't on
`PATH` or a probe of the sandbox's namespace setup fails.

- **Root:** the host's `/` is bound read-only one top-level entry at a time
  (each directory bound, each symlink recreated) rather than as one bind, so
  the sandbox's own root stays a tmpfs where bwrap can make the `/work`
  mount points, then `/` is remounted read-only.
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
  at `/work/project`, `/work/home` and `/work/xdg`. `/tmp` is a fresh, empty
  tmpfs per run. The run dir's other files (`stdout.jsonl`, `setup.log`) are
  not visible.
- **Bound back read-only:** the round's `identity-model/` export at
  `/work/identity-model`, `bin-codecite/` at `/work/bin` (codecite arm only:
  the arms' sandboxes differ by that one bind), and what the commands need,
  at their own paths, found from the command itself: its
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
- **Argv:** bwrap is pid 1 inside the sandbox, so its argv is any agent
  process's `/proc/1/cmdline` (`ps` shows it). Its binds name the round, the
  run and its arm, so `spawnSandbox` hands them over on a pipe (`--args 3`):
  its argv is only `bwrap --args 3 -- <command>`.
- **Secret files:** a file an adapter needs that holds a secret (pi's
  `auth.json`) is its spec's `dataFiles`: bwrap copies it from a pipe into its
  own tmpfs (`--bind-data`, mode 0600), so it never touches the host's disk,
  even if the harness dies mid-run. The empty mount point bwrap leaves is
  deleted after the run.
- **Kill:** on timeout or an early reject the call loop sends `SIGTERM` to
  every process inside the sandbox (found under bwrap in `/proc`: the agent
  has its own session, out of bwrap's group) but not to bwrap, which would
  die at once and take the agent with it. `SIGKILL` goes to bwrap's group 10 s
  later; killing bwrap ends its pid namespace and everything left in it.

Taint scanning stays as a second layer: it catches a run that tried to reach
the hidden paths or fetch code, even though the sandbox kept them out.

Known limits: the mount table (`/proc/self/mountinfo`, `findmnt`) shows the
host path behind each `/work` bind, which names the round, the run and its
arm; reading it taints the run. In the codecite arm the corpus file
(`/work/xdg/codecite/eval.db`) records the round's throwaway source path,
which no `codecite` command prints.

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
...AgentResult without transcript, tainted, secretInTranscript}`, where
`status` is `ok`, `timeout`, `agent_error`, `infra_error` or `fatal`. Every
secret value is replaced by `[REDACTED]` before anything is written;
`secretInTranscript` is true when that changed the agent's output (see
*Secrets*).

- **Resume:** a run whose `result.json` says `ok`, `timeout` or `agent_error`
  is skipped: all three are usable outcomes, and redoing failures would make
  the failure rate fall with each resume. Two exceptions are redone: a
  tainted run, and one whose total tokens are unknown (no usage reported,
  e.g. pi died before its first message). Its old dir is kept beside the new
  one as `runs/<run-id>.tainted-<n>` or `runs/<run-id>.unknown-<n>`, so the
  scorecard still lists it, and never counts. Any other run (`infra_error`,
  never finished) is redone from a fresh run directory. A round holding a
  `fatal` run refuses to resume until you fix the cause and pass
  `--retry-fatal`, which re-runs the fatal runs first.
- **Fatal:** provider status 401, 402 or 404, a 400 before any turn, or an
  adapter's `rejectReason` (from the init event, or `earlyReject` on a live
  stdout line, which kills the agent at once) is recorded as `fatal` and
  stops the round. A 400 after a turn is the run's own `agent_error`.
- **Rate limits:** a run that ends on the provider's rate or usage limit (a
  429; for Claude also a rejected `rate_limit_event` or the usage-limit text
  in its error result) is infra however many turns it took, never a failed
  run: retried with backoff and left out of the breaker. When the limit
  outlasts the retries, the run is recorded `infra_error` and the round stops
  (`rate_limit`): wait for the limit to reset, then run `call` again, which
  redoes it.
- **Infra:** a spawn or setup failure (a missing agent command, or bwrap
  failing before the agent starts), a 5xx before any turn, or an adapter's
  `endedOnError` before any turn (any status, or none: pi's 429, Claude Code's
  usage-limit 429), is retried up to 3 times, 5 s and then 20 s apart, then
  recorded as `infra_error`. `endedOnError` after a turn is `agent_error`,
  even when the agent exited 0.
- **Breaker:** the round stops once at least 10 attempts are in and 80% or
  more of them ended in anything but `ok` (timeouts count as failures here,
  though resume still skips them).
- **Spend:** before any run, the OpenRouter estimate for the pending runs
  (300k input and 20k output tokens per run at the model's live price;
  Claude counts as $0) must fit in the key's remaining limit, and the key
  must have a limit. `compose --dry-run` prints the same estimate.
- **Taint:** `tainted` lists shell commands matching `github.com`,
  `githubusercontent.com`, `git clone`, `pip install` (so `uv pip install`
  and `python -m pip install` too: the spec lists it and the prompt says "no
  new packages") or `uv add`; a `curl` or `wget` run as the command (its
  first word, after `VAR=value` and wrappers such as `sudo`, `env`,
  `timeout`; so `which curl`, `curl --version` and `grep curl` don't count)
  against anything but loopback (`127.0.0.0/8`, `localhost`, `::1`), or
  against a target it can't read (`curl $URL`, anything fed by `xargs`).
  Commands are split quote-aware for this, so a quoted `-d "a=b&c=d"` stays
  whole, and option values (`-d`, `-H`, `-o`, ...) and redirection targets are
  never read as hosts; and shell commands or file-tool paths naming `evals/tasks`,
  `test_hidden`, `/reference/`, `codecite-worktrees`, `mountinfo` or
  `findmnt`. Each command is split into segments on `&&`, `||`, `;`, `|`,
  `&` and newlines, with a cwd that starts at `/work/project` and follows
  `cd`. Every word is resolved against it, as is every file-tool path. A path
  taints when it is inside a `taintPaths` entry (the real repos, this
  checkout, the round's `task/`, `plan.json`, `round.json`, `corpus.toml`,
  `runs/`), never for being an ancestor of one: the sandbox hides the round
  and everything above the run's dirs, so `ls /`, `find /` or a `/` in a
  `grep` or a Python division don't taint. Only commands and paths are
  scanned, never file contents. The scan is lexical: a path built from
  `$VAR`, `$(...)`, globs or a script the agent wrote is not resolved, so
  tainting is detection only; the sandbox is what keeps those paths out of
  reach. A tainted run is redone on resume (see *Resume*).

## Secrets

`~/.config/codecite/evals.env`, mode 0600, holds `OPENROUTER_EVAL_API_KEY`
and `CLAUDE_CODE_OAUTH_TOKEN`. Only the `call` phase reads it. The harness
refuses the file when group or others can read it, and never prints a value.

When `CLAUDE_CODE_OAUTH_TOKEN` is unset, the `claude` adapter falls back to
the owner's `~/.claude/.credentials.json` (see *claude* below), read fresh
for each run and never written.

Both files are read by the harness, outside the sandbox. Inside it, neither
exists. But each agent holds the one credential it needs: Claude Code's
access token in its environment, pi's OpenRouter key in its agent dir's
`auth.json` (not its environment, so `env` doesn't show it). The sandbox
can't stop an agent printing it (`env`, `cat ~/pi/auth.json`), and whatever
the agent prints goes to its model provider; redaction cleans only the files
on disk. A run whose output held a secret has `secretInTranscript: true`
(and `SECRET-IN-OUTPUT` on its progress line); `call` ends with a warning
naming every such run of the round, and the scorecard lists them: rotate the
OpenRouter key and log in to Claude Code again. Neither agent ever gets the
refresh token or the other agent's credential.

## Agents

### claude

`agents/claude.mjs` runs `claude -p <prompt> --model <model>
--output-format stream-json --verbose --strict-mcp-config --mcp-config
'{"mcpServers":{}}' --setting-sources '' --disallowedTools WebFetch,WebSearch
--dangerously-skip-permissions --no-session-persistence` (the same in both
arms). Its environment is built from nothing: `HOME` and `CLAUDE_CONFIG_DIR`
are `/work/home`, `XDG_DATA_HOME` is `/work/xdg`, `DISABLE_AUTOUPDATER=1` and
`CLAUDE_CODE_OAUTH_TOKEN`; `ANTHROPIC_API_KEY` is never set, so the run is on
the owner's subscription, not billed per token.

- **Auth:** `CLAUDE_CODE_OAUTH_TOKEN` from `evals.env`, else
  `claudeAiOauth.accessToken` from `~/.claude/.credentials.json`. From the
  file, its `refreshToken` is listed in the spec's `secrets` (redacted, never
  passed), and its `expiresAt` is checked before each run: a login that has
  expired, or would before the run's timeout plus a minute, stops the round
  with "log in to Claude Code to refresh it" rather than a fatal 401 partway
  through. The harness never refreshes the login; run `claude`, then resume.
- **Config leaks:** `--setting-sources ''` loads no user, project or local
  settings, and the empty `CLAUDE_CONFIG_DIR` has no `CLAUDE.md`, memory,
  plugins or hooks. The run is rejected (`config leaked`, fatal, killed at
  once by `earlyReject`) on a hook event, or an init event that lists an MCP
  server, an `mcp__` tool, a non-builtin plugin, or a plugin-namespaced
  (`plugin:name`) skill or slash command.
- **Usage:** from the `result` event (`usage`, `total_cost_usd`,
  `num_turns`, `is_error`, `api_error_status`). Total tokens are input,
  output, cache reads and cache writes. A run killed at timeout has no
  `result` event: its tokens are summed from the assistant messages' `usage`,
  each message id once (stream-json repeats it on every content block), and
  its cost is unknown. An `is_error` result sets `endedOnError`, so a 429
  before any turn (a usage limit) is infra, not a failed run.

### pi

`agents/pi.mjs` runs `pi -p --mode json --provider openrouter --model <slug>`
with `--no-session --no-mcp --no-extensions --no-skills
--no-prompt-templates --no-themes --no-approve --tools
read,bash,edit,write,grep,find,ls` (the same in both arms), the prompt last
after `--`. Its environment is built from nothing: `HOME` is `/work/home`,
`PI_CODING_AGENT_DIR` is `/work/home/pi`, `PI_TELEMETRY=0` and
`PI_SKIP_VERSION_CHECK=1`; no key. The agent dir holds only an `auth.json`
(mode 0600) with the evals key as pi's `openrouter` API-key credential
(handed to bwrap on a pipe, see *Secret files*, so no round dir ever holds
the key), and a
`models.json` setting OpenRouter's `X-Title` header to an opaque per-arm
label (`eval-<8 hex>`, from the round id and arm), so spend shows per arm on
the OpenRouter dashboard while the agent, which can read the file, can't
tell its arm from it; the scorecard maps each label back to its arm
(`PI_TELEMETRY=0` also stops pi sending its own attribution headers).

pi reports what it loaded in the session's system message: the system
prompt's `sections` and `toolsAdded`. The run is rejected (`config leaked`,
fatal, killed at once by `earlyReject`) when that message has a section
other than `preamble`, `tools`, `rules`, `docs`, `cwd` and `project_context`
(for example `skills`, or `addendum` from an `APPEND_SYSTEM.md`), a tool
outside the list above, or a context file (`AGENTS.md`, `CLAUDE.md`) outside
the run's cwd. A replaced system prompt (`SYSTEM.md`) is not detectable from
the stream; the empty agent dir and `--no-approve` are what keep it out.

Usage is the sum of each assistant `message_end`'s `usage`. Its `costUsd`
is pi's own estimate from its model price table, not what OpenRouter bills,
and runs low against the OpenRouter dashboard; read spend there. `turns` counts
assistant messages that didn't end in error; `providerStatus` is the status
at the head of the last assistant message's `errorMessage` when the run
ended on an error. pi exits 0 even when the provider ended the run, so the
adapter sets `endedOnError` whenever the last assistant message ended in
error: a fatal status stays fatal, an error before any turn (a 429, a
connection error) is infra and retried, and one after a turn is
`agent_error`, never `ok`.

## Scoring

```
node evals/run.mjs score --round <id> [--rescore]
```

`score` holds no secrets. It starts the IdP (`fixtures/node-oidc`, with this
Node on a free loopback port; run `npm ci` in `evals/fixtures/node-oidc`
first, or `startIdp` refuses), scores every run whose `result.json` status is
`ok`, `timeout` or `agent_error` and that has no pass/fail score yet (all of
them with `--rescore`; unscored runs are tried again, 3 attempts in all), then writes `<round>/scorecard.md` and prints each
pair's verdict and the round's. `infra_error` and `fatal` runs are not
scored.

- **Command:** the frozen plan's `taskDef.score`, with `{hidden}` set to
  `/work/hidden`, where the round's `task/hidden/` is bound, run in
  `/work/project`, the run's `project/` at the agent's own path (its venv and
  lock name it; the task's conftest puts the cwd on `sys.path`). The
  environment is built from nothing: the cleaned `PATH`, `TEST_IDP`, `HOME`
  and the XDG cache at a fresh `runs/<id>/score/`, bound at `/work/score`
  (uv's cache, so `--with pytest==9.1.1 --with httpx==0.28.1`, pinned in
  `task.json`, download there), `PYTHONDONTWRITEBYTECODE=1`, `PYTHONPATH` at
  the marker plugin (`/work/score-plugin`), and `PYTEST_ADDOPTS=-p
  codecite_score_started --junitxml=/work/score/report.xml`.
- **Sandbox:** the score command imports the agent's code, so it runs in the
  same bubblewrap sandbox as the agent (`roundSandbox`): the same hides (the
  owner's home, `/tmp`, `/run`, the round dir, its `taintPaths`), the export
  and the tools (uv, the venv's base Python, their CA files) read-only, the
  project read-write. Two differences: the task's `hidden/` and the marker
  plugin are bound read-only, and the run's own `home/` and `xdg/` are not
  bound (`score/` is its home instead). The network is shared, so the IdP on `127.0.0.1` is
  reachable. A bwrap failure is a harness error, never a failed run.
- **Pass:** the command exits 0 and its JUnit report shows at least one test
  and no failure, error or skip.
- **Two stages.** A scoring fault is never the run's result, and the agent's
  own breakage always is:
  - *Infra* (unscored: `passed: null`, the reason in `unscored`): the IdP's
    discovery document not answering before or after; the task's
    `scoreWarm` command (`uv run --no-project` with the same pinned
    `--with` deps, run in `score/` with the venv's base Python, outside the
    agent's project) failing or timing out; the score command timing out
    before pytest started. An unscored run doesn't count, so its pair reads
    INCOMPLETE.
  - *Agent* (failed): after a good warm step, no report is a failed run,
    either "before pytest started" (the agent's project failed to sync or
    import under uv) or "pytest started" (its code ended the process). A
    timeout after pytest started fails too. The started marker is written by
    `lib/pytest-plugin/codecite_score_started.py`, loaded with `-p` from a
    read-only bind, before any conftest imports the agent's code.
  - *No warm step:* `loadTask` (so `compose`) requires `scoreWarm`. A plan
    frozen before that has none, so it can't tell a fetch failure from a
    broken project: there, no report before pytest started is unscored.
- **Retries:** an unscored run is tried up to 3 times in all, across `score`
  calls (warm-step failures and timeouts before pytest started count; an IdP
  outage doesn't); then it is a fatal scoring error (`fatal: true`), skipped by later
  calls until `--rescore`, shown at the top of the scorecard, and `score`
  exits 1. An IdP that is down stops the phase. A missing `project/` is a
  harness fault and stops it too.
- **Known limit:** the agent's code runs in the same pytest process as the
  hidden tests, so code written to do it could forge a passing report (patch
  pytest, rewrite the JUnit file). Nothing in this task's prompt invites
  that; a suspicious pass needs its transcript read.
- `score.json` holds `passed`, `failures` (test ids or the no-report
  reason), `unscored`, `scoreAttempts`, `fatal`, `tests`, `exitCode`,
  `timedOut`, `pytestStarted`, `wallMs` and the commands; `score.log` both
  stages' output.

### The scorecard

Per agent/model pair, each non-baseline arm against the baseline:

- **Usable runs:** scored (passed or failed), status `ok`, `timeout` or
  `agent_error`, with known total tokens, not tainted, not a kept-aside
  `<id>.tainted-<n>` or `<id>.unknown-<n>` dir. Excluded (infra, fatal,
  unscored, unknown tokens), retried and tainted runs are listed.
- **Measures:** correctness is the difference in pass rate, codecite minus
  baseline, in points; tokens and time are the relative difference of the
  medians (codecite / baseline - 1) of total tokens and wall time over all
  usable runs, failed ones included.
- **Loss:** each measure gets a 95% percentile bootstrap interval: each arm's
  usable runs resampled with replacement on their own, 10 000 times, from a
  seed derived from the plan's seed and the pair, so a scorecard is
  reproducible. A loss is an interval past the margin at its favourable end:
  correctness when the upper bound is below -5 points, tokens or time when
  the lower bound is above +5%. This replaces the spec's plain comparison of
  the numbers (codecite's median more than 5% above baseline's), which at 20
  runs per arm stops on noise most of the time; the owner chose the
  interval. A measure with an unknown (`null`) value in either arm has no
  interval and is `loss: null`: it can't pass.
- **Pair verdict:** `INCOMPLETE` when either arm has fewer usable runs than
  the plan's `runs`; else `STOP` on any loss; else `INCOMPLETE` on any
  unknown measure; else `PASS`. The round is `STOP` if any pair is,
  `INCOMPLETE` if any is, else `PASS`.
- Each measure's row shows the point estimate and its interval, e.g.
  `+6.0% (95% CI -3.1% to +15.2%)`.
- Also shown: IQRs, medians over passing runs only, cost, tool calls, the
  share of runs that called `codecite`, pi's X-Title label per arm, runs that
  printed a secret, the codecite note and the frozen plan.
