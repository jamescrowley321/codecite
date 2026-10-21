// The agent adapter contract. An adapter turns one run into a command line
// and turns that command's output into an AgentResult. Adapters live at
// evals/agents/<name>.mjs (default export); the plan's `agent` is the name.

import { emptyUsage } from './telemetry.mjs';

/**
 * @typedef {{input: number|null, output: number|null, cacheRead: number|null, cacheWrite: number|null,
 *   totalTokens: number|null, costUsd: number|null}} Usage  See telemetry.mjs.
 *
 * @typedef {object} AgentResult
 * @property {Usage} usage            Token and cost totals; a field the agent didn't report is null.
 * @property {number|null} turns      Model turns taken; null when unknown.
 * @property {number|null} toolCalls  Tool calls made; null when unknown.
 * @property {number|null} codeciteCalls  Shell calls that ran `codecite`; null when unknown.
 * @property {string} transcript      Human-readable transcript; not written to result.json.
 * @property {number|null} providerStatus  The last provider HTTP error status, else null.
 * @property {object|null} init       The agent's init/config report (model, tools, ...), if any.
 * @property {string|null} rejectReason  Non-null when the adapter refuses the run as invalid
 *   (e.g. it sees the owner's own config loaded). Fatal: the round stops.
 * @property {string[]} shellCommands  Every command the agent ran through its shell tool.
 * @property {string[]} readPaths     Every path its read/edit/write/grep/find/ls-style tools touched,
 *   as the agent gave it (relative paths resolve against the run's project dir).
 * @property {boolean} [endedOnError]  True when the agent's last model call ended in a
 *   provider error (any status, or none, e.g. a connection error). For agents that exit 0
 *   anyway (pi): the run is then infra_error before any turn, else agent_error, never ok.
 *   Optional; agents whose exit code already reflects the error may leave it unset.
 *
 * @typedef {object} AgentCommand
 * @property {string} cmd
 * @property {string[]} args
 * @property {Record<string,string>} env  The agent's whole environment; nothing else is inherited.
 *   Secrets go here and only here, never in args.
 * @property {string} cwd
 * @property {string} [input]  Written to the agent's stdin, then closed; stdin is empty otherwise.
 * @property {{path: string, data: string}[]} [dataFiles]  Files the agent needs that hold a secret
 *   (pi's auth.json), at in-sandbox paths. bwrap copies each from a pipe into its own tmpfs, 0600,
 *   so it never touches the host's disk.
 * @property {string[]} [secretFiles]  Host paths the harness deletes once the agent exits (the
 *   empty mount points dataFiles leave, or any file the adapter wrote holding a secret).
 * @property {string[]} [secrets]  Extra secret values this run uses that are not in
 *   ctx.secrets (e.g. a token the adapter read itself). The harness redacts them
 *   from stdout, stderr and result.json exactly like ctx.secrets.
 *
 * @typedef {object} AgentContext
 * @property {string} prompt   renderPrompt(task, arm, round) for this run.
 * @property {{openrouter: string|null, claudeOauth: string|null}} secrets  From readSecrets().
 * @property {object} plan     The frozen plan (plan.json).
 * @property {object} round    round.json plus `plan`.
 *
 * @typedef {object} Adapter
 * @property {string} name
 * @property {(run: object, dirs: object, ctx: AgentContext) => AgentCommand} command
 *   `run` is {id, agent, model, arm, rep, seq}; `dirs` is prepareRun's {out, project, home, xdg, bin, path,
 *   host}: project/home/xdg/bin are the neutral in-sandbox paths (/work/...) the agent sees, which go
 *   in its env and cwd; `host` holds the same dirs on the host, for files the adapter writes first.
 *   Throwing (e.g. requireSecret) stops the round.
 * @property {(stdout: string, stderr: string) => AgentResult} parse
 *   Gets the output with secrets already redacted. Throwing records an agent_error.
 *   Adapters should set `rejectReason` from the agent's init event (e.g. it loaded
 *   the owner's config, or the wrong model), which makes the run fatal.
 * @property {(line: string) => string|null} [earlyReject]
 *   Optional. Called with each raw stdout line while the agent runs; a non-null
 *   reason kills the agent at once (SIGTERM, then SIGKILL) and becomes its
 *   `rejectReason`, so a bad run stops before it spends.
 */

/** An AgentResult with everything unknown, for runs whose output can't be parsed. */
export function emptyAgentResult() {
  return {
    usage: emptyUsage(),
    turns: null,
    toolCalls: null,
    codeciteCalls: null,
    transcript: '',
    providerStatus: null,
    init: null,
    rejectReason: null,
    shellCommands: [],
    readPaths: [],
    endedOnError: false,
  };
}

/** Import evals/agents/<name>.mjs and check it has the adapter shape. */
export async function loadAdapter(name) {
  if (!/^[a-z0-9-]+$/.test(name)) throw new Error(`agent: bad adapter name "${name}"`);
  const mod = await import(new URL(`../agents/${name}.mjs`, import.meta.url).href);
  const a = mod.default;
  if (typeof a?.command !== 'function' || typeof a?.parse !== 'function') {
    throw new Error(`agent: evals/agents/${name}.mjs must default-export {name, command, parse}`);
  }
  return a;
}
