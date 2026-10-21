// The call phase: run every planned run through its agent adapter, serially
// in seq order, and record each result. Stops on a fatal provider error or
// rejected run, on runaway failure (circuit breaker), and refuses to start
// when the OpenRouter estimate exceeds the key's remaining limit. Every agent
// runs inside the bubblewrap sandbox (sandbox.mjs); there is no unsandboxed
// fallback.

import { spawn } from 'node:child_process';
import { StringDecoder } from 'node:string_decoder';
import { accessSync, constants, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { emptyAgentResult, loadAdapter as defaultLoadAdapter } from './agent.mjs';
import { CircuitBreaker, classify } from './guards.mjs';
import { estimateUsd, keyRemainingUsd as orKeyRemaining, modelPrices as orModelPrices } from './openrouter.mjs';
import { expandRuns, renderPrompt } from './plan.mjs';
import { descendants, findBwrap, IN_SANDBOX, probeBwrap, roundSandbox, spawnSandbox } from './sandbox.mjs';
import { requireSecret } from './secrets.mjs';
import { taintHits } from './taint.mjs';
import { prepareRun } from './workspace.mjs';

/** Every status a result.json can hold. */
export const STATUSES = new Set(['ok', 'timeout', 'agent_error', 'infra_error', 'fatal']);
/**
 * Statuses a resumed `call` skips: every usable outcome, failed runs
 * included. Redoing agent_error would re-draw exactly the failures, so the
 * failure rate would fall with each resume.
 */
export const DONE = new Set(['ok', 'timeout', 'agent_error']);
export const MAX_ATTEMPTS = 3;
/** Waits before the 2nd, 3rd, ... attempt at a run that failed on infra. */
export const BACKOFF_MS = [5_000, 20_000, 60_000];
export const KILL_GRACE_MS = 10_000;
export const REDACTED = '[REDACTED]';

export { roundSandbox };

/**
 * ctx.secrets plus the extra values an adapter's spec says it used (a token
 * the adapter read itself), as one secrets object for redact().
 */
export function runSecrets(secrets, spec) {
  const extra = Array.isArray(spec?.secrets) ? spec.secrets.filter((v) => typeof v === 'string' && v) : [];
  return { ...secrets, ...Object.fromEntries(extra.map((v, i) => [`adapter${i}`, v])) };
}

/** Replace every secret value in `text` with [REDACTED]. */
export function redact(text, secrets) {
  let out = text;
  for (const v of Object.values(secrets ?? {})) if (typeof v === 'string' && v) out = out.replaceAll(v, REDACTED);
  return out;
}

/** A round from disk: round.json plus its frozen plan.json as `plan`. */
export function loadRound(roundDir) {
  const read = (f) => {
    try {
      return JSON.parse(readFileSync(join(roundDir, f), 'utf8'));
    } catch (e) {
      throw new Error(`call: cannot read ${join(roundDir, f)}: ${e.message}`);
    }
  };
  const plan = read('plan.json');
  return { ...read('round.json'), plan };
}

function priorResult(round, run) {
  const file = join(round.dir, 'runs', run.id, 'result.json');
  if (!existsSync(file)) return null;
  try {
    return JSON.parse(readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

/**
 * Why a finished run must be run again although its status is DONE, else
 * null: "tainted" (it reached outside its sandbox) or "unknown" (its total
 * tokens are unknown, so it can't count toward the token measure). Such a
 * run is redone on resume, and its old dir is kept beside the new one as
 * `<id>.<why>-<n>`, so the scorecard still lists it.
 */
export function redoReason(result) {
  if (!result || !DONE.has(result.status)) return null;
  if (Array.isArray(result.tainted) && result.tainted.length > 0) return 'tainted';
  const t = result.usage?.totalTokens;
  if (typeof t !== 'number' || !Number.isFinite(t)) return 'unknown';
  return null;
}

/**
 * Every run dir of the round (kept-aside ones too) whose result.json says a
 * secret was redacted from its output: the agent printed its credential,
 * which then went to the model provider.
 */
export function secretRuns(roundDir) {
  const runs = join(roundDir, 'runs');
  if (!existsSync(runs)) return [];
  return readdirSync(runs)
    .sort()
    .filter((d) => {
      try {
        return JSON.parse(readFileSync(join(runs, d, 'result.json'), 'utf8')).secretInTranscript === true;
      } catch {
        return false;
      }
    });
}

/** Move a run's dir aside as `<id>.<why>-<n>`, n one more than any kept so far. */
function keepAside(round, run, why) {
  const runs = join(round.dir, 'runs');
  const prefix = `${run.id}.${why}-`;
  const n = 1 + Math.max(0, ...readdirSync(runs).filter((d) => d.startsWith(prefix)).map((d) => Number(d.slice(prefix.length)) || 0));
  renameSync(join(runs, run.id), join(runs, `${prefix}${n}`));
}

function resetDir(dir) {
  if (!existsSync(dir)) return;
  // Agents may leave read-only files behind; make them removable.
  const chmod = spawn('chmod', ['-R', 'u+w', dir]);
  return new Promise((res) => chmod.on('close', res).on('error', res)).then(() => rmSync(dir, { recursive: true, force: true }));
}

/**
 * Spawn the sandboxed agent (bwrap) in its own process group. On timeout, or
 * when `earlyReject(line)` returns a reason for a stdout line, every process
 * inside the sandbox gets SIGTERM, then bwrap's group SIGKILL `killGraceMs`
 * later; killing bwrap ends its pid namespace, which kills everything left
 * inside. Wall time runs from spawn to exit.
 */
function spawnAgent({ sb, cwd, input }, { timeoutMs, killGraceMs, earlyReject }) {
  return new Promise((resolve) => {
    const out = [];
    const err = [];
    const t0 = performance.now();
    let timedOut = false;
    let rejected = null;
    let spawnError = null;
    let exit = null;
    let done = false;
    let killTimer;
    let child;
    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      clearTimeout(killTimer);
      clearTimeout(closeTimer);
      resolve({
        stdout: Buffer.concat(out).toString('utf8'),
        stderr: Buffer.concat(err).toString('utf8'),
        wallMs: Math.round(exit?.wallMs ?? performance.now() - t0),
        exitCode: exit?.code ?? null,
        signal: exit?.signal ?? null,
        timedOut,
        earlyReject: rejected,
        spawnError,
      });
    };
    const signal = (pid, sig) => {
      try {
        process.kill(pid, sig);
      } catch {
        // already gone
      }
    };
    // SIGKILL: bwrap's group and everything under it. Killing bwrap ends its
    // pid namespace, so nothing inside survives.
    const killGroup = (sig) => {
      if (!child?.pid) return;
      for (const pid of descendants(child.pid)) signal(pid, sig);
      signal(-child.pid, sig);
    };
    // SIGTERM goes to the agent's processes only, not to bwrap: bwrap dies
    // on it at once and takes the namespace with it, so the agent would never
    // get its grace. The agent has its own session (--new-session), out of
    // bwrap's group, so it is found by walking bwrap's descendants.
    const terminate = () => {
      if (!child?.pid) return;
      const inside = descendants(child.pid).filter((pid) => !isBwrap(pid));
      if (inside.length === 0) signal(-child.pid, 'SIGTERM');
      for (const pid of inside) signal(pid, 'SIGTERM');
    };
    const stop = () => {
      terminate();
      killTimer = setTimeout(() => killGroup('SIGKILL'), killGraceMs);
    };
    const timer = setTimeout(() => {
      timedOut = true;
      stop();
    }, timeoutMs);
    let closeTimer;
    try {
      child = spawnSandbox(sb, { cwd, detached: true, stdio: [input == null ? 'ignore' : 'pipe', 'pipe', 'pipe'] });
    } catch (e) {
      spawnError = e.code ? `${e.code}: ${e.message}` : e.message;
      finish();
      return;
    }
    const decoder = new StringDecoder('utf8');
    let pending = '';
    child.stdout.on('data', (c) => {
      out.push(c);
      if (!earlyReject || rejected) return;
      pending += decoder.write(c);
      let nl;
      while (!rejected && (nl = pending.indexOf('\n')) >= 0) {
        const line = pending.slice(0, nl);
        pending = pending.slice(nl + 1);
        try {
          rejected = earlyReject(line) || null;
        } catch {
          rejected = null;
        }
        if (rejected) {
          clearTimeout(timer);
          stop();
        }
      }
    });
    child.stderr.on('data', (c) => err.push(c));
    if (input != null) {
      child.stdin.on('error', () => {});
      child.stdin.end(input);
    }
    child.on('error', (e) => {
      spawnError = e.code ? `${e.code}: ${e.message}` : e.message;
      if (!child.pid) finish();
    });
    child.on('exit', (code, signal) => {
      exit = { code, signal, wallMs: performance.now() - t0 };
      clearTimeout(timer);
      clearTimeout(killTimer);
      killGroup('SIGKILL');
      // A stray that escaped the group may hold the pipes open.
      closeTimer = setTimeout(finish, 2000);
    });
    child.on('close', finish);
  });
}

/**
 * One attempt at one run. Returns {kind, result, secrets}, where `secrets`
 * is everything to redact from this run's output; writes stdout/stderr.
 */
async function attemptRun(run, round, adapter, opts) {
  const { secrets, timeoutMs, killGraceMs, basePath, bwrap, sandbox } = opts;
  const { plan } = round;
  const out = join(round.dir, 'runs', run.id);
  await resetDir(out);
  const base = { run, status: 'infra_error', wallMs: null, exitCode: null, signal: null, error: null };

  let dirs;
  try {
    dirs = prepareRun(run, round, { basePath, bwrap, sandbox });
  } catch (e) {
    mkdirSync(out, { recursive: true });
    return { kind: 'infra', result: { ...base, error: e.message, ...strip(emptyAgentResult()), tainted: [], secretInTranscript: false }, secrets };
  }

  const arm = plan.arms.find((a) => a.name === run.arm);
  const prompt = renderPrompt(plan.taskDef, arm);
  // A throwing adapter (e.g. a missing secret) is a setup error: stop the round.
  const spec = adapter.command(run, dirs, { prompt, secrets, plan, round });
  const hidden = runSecrets(secrets, spec);
  const earlyReject = typeof adapter.earlyReject === 'function' ? (line) => adapter.earlyReject(line) : null;
  const proc = await spawnSandboxed(spec, dirs, round, { bwrap, sandbox, timeoutMs, killGraceMs, earlyReject });
  for (const f of Array.isArray(spec.secretFiles) ? spec.secretFiles : []) rmSync(f, { force: true });
  const stdout = redact(proc.stdout, hidden);
  const stderr = redact(proc.stderr, hidden);
  // The agent's environment holds its credential; a run that printed it
  // (`env`, `printenv`) sent it to the model provider. Redaction cleans the
  // files only, so the run is flagged and the key should be rotated.
  const secretInTranscript = stdout !== proc.stdout || stderr !== proc.stderr;
  writeFileSync(join(out, 'stdout.jsonl'), stdout);
  writeFileSync(join(out, 'stderr.txt'), stderr);

  let agent = emptyAgentResult();
  let error = null;
  if (proc.spawnError) {
    error = `spawn ${spec.cmd}: ${proc.spawnError}`;
  } else {
    try {
      agent = { ...agent, ...adapter.parse(stdout, stderr) };
    } catch (e) {
      error = `parse: ${e.message}`;
    }
  }
  if (proc.earlyReject) agent.rejectReason = proc.earlyReject;

  const kind = classify({ ...agent, spawnError: proc.spawnError });
  let status;
  if (kind === 'fatal') {
    status = 'fatal';
    error = agent.rejectReason ? `run rejected: ${agent.rejectReason}` : `provider status ${agent.providerStatus}`;
  } else if (kind === 'infra') {
    status = 'infra_error';
    if (agent.rateLimited) error ??= `provider rate or usage limit${agent.providerStatus == null ? '' : ` (${agent.providerStatus})`} after ${agent.turns ?? '?'} turns`;
    if (agent.endedOnError && agent.turns > 0) {
      const what = agent.providerError ? `: ${String(agent.providerError).slice(0, 300)}` : agent.providerStatus == null ? '' : ` (${agent.providerStatus})`;
      error ??= `provider error after ${agent.turns} turns${what}`;
    }
    if (agent.providerError) error ??= `provider error before any turn: ${String(agent.providerError).slice(0, 300)}`;
    error ??= agent.providerStatus == null ? 'provider error before any turn' : `provider status ${agent.providerStatus} before any turn`;
  } else if (proc.timedOut) {
    status = 'timeout';
  } else if (agent.endedOnError) {
    // Only a 400 after a turn gets here (classify): the run's own failure.
    status = 'agent_error';
    error ??= `agent ended on a provider error${agent.providerStatus == null ? '' : ` (${agent.providerStatus})`}`;
  } else if (proc.exitCode === 0 && !error) {
    status = 'ok';
  } else {
    status = 'agent_error';
    error ??= `agent exited ${proc.exitCode ?? proc.signal}`;
  }

  // The agent's paths are in-sandbox ones: its cwd is /work/project and
  // everything under /work is its own.
  const tainted = taintHits(
    { shellCommands: agent.shellCommands ?? [], readPaths: agent.readPaths ?? [] },
    { taintPaths: round.taintPaths ?? [], ownRunDir: IN_SANDBOX.root, cwd: spec.cwd ?? dirs.project },
  );
  return {
    kind,
    result: { ...base, status, wallMs: proc.wallMs, exitCode: proc.exitCode, signal: proc.signal, error, ...strip(agent), tainted, secretInTranscript },
    secrets: hidden,
  };
}

const isBwrap = (pid) => {
  try {
    return readFileSync(`/proc/${pid}/comm`, 'utf8').trim() === 'bwrap';
  } catch {
    return false;
  }
};

/**
 * Run `spec` inside roundSandbox. A missing command, or bwrap failing before
 * the agent starts, is a spawn error.
 */
async function spawnSandboxed(spec, dirs, round, { bwrap, sandbox = {}, ...run }) {
  const failed = (spawnError) => ({ stdout: '', stderr: '', wallMs: 0, exitCode: null, signal: null, timedOut: false, earlyReject: null, spawnError });
  try {
    accessSync(spec.cmd, constants.X_OK);
  } catch (e) {
    return failed(`${e.code ?? 'error'}: ${spec.cmd} is not an executable file`);
  }
  let sb;
  try {
    sb = roundSandbox(spec, dirs, round, { bwrap, sandbox });
  } catch (e) {
    return failed(`sandbox: ${e.message}`);
  }
  // bwrap itself starts in the host project dir; --chdir puts the agent at spec.cwd.
  const proc = await spawnAgent({ sb, cwd: dirs.host?.project ?? dirs.project, input: spec.input }, run);
  // bwrap's own errors (a bind it couldn't make) come before any agent output.
  const bwrapError = /^bwrap: .*/.exec(proc.stderr)?.[0];
  if (!proc.spawnError && proc.exitCode !== 0 && proc.stdout === '' && bwrapError) proc.spawnError = bwrapError;
  return proc;
}

function strip({ transcript, ...rest }) {
  return rest;
}

async function checkSpend(plan, runs, { secrets, keyRemainingUsd, modelPrices, log }) {
  if (!runs.some((r) => r.agent !== 'claude')) return;
  const estimate = estimateUsd(plan, { prices: await modelPrices(), runs });
  const remaining = await keyRemainingUsd(requireSecret(secrets, 'openrouter'));
  if (remaining == null) throw new Error('call: the OpenRouter key has no spending limit; set one before running a round');
  if (estimate > remaining) {
    throw new Error(`call: estimate $${estimate.toFixed(2)} exceeds the key's remaining $${remaining.toFixed(2)}; refusing to start`);
  }
  log(`spend: estimate $${estimate.toFixed(2)} of $${remaining.toFixed(2)} remaining on the key`);
}

/**
 * Run a round's pending runs. A run whose result.json has status ok or
 * timeout is skipped, so `call` resumes. A round holding a `fatal` run
 * refuses to resume unless `retryFatal`, which re-runs the fatal runs first.
 * Infra failures are retried up to `maxAttempts` times. Each attempt counts
 * in the circuit breaker; only `ok` is a success there.
 *
 * @param {string} roundDir
 * @param {object} deps  `secrets` (readSecrets()) is required; the rest default
 *   to the real thing: loadAdapter, keyRemainingUsd(key), modelPrices(),
 *   log(line), timeoutMs (plan.timeoutSec), killGraceMs (10 s), maxAttempts
 *   (3), basePath (PATH for prepareRun), retryFatal (false), searchPath (the
 *   PATH bwrap is looked up on), sandbox ({ro, rw, alwaysHidden}: tests only).
 * A rate or usage limit (adapter `rateLimited`) is infra at any turn: retried
 * with backoff, and when it outlasts the retries the round pauses
 * (`stopped: "rate_limit"`); the run is infra_error, so resume redoes it.
 *
 * @returns {Promise<{ran: number, skipped: number, stopped: null|"fatal"|"breaker"|"rate_limit", reason: string|null, secretRuns: string[]}>}
 *   `secretRuns`: every run of the round whose output held a secret (see secretRuns()).
 */
export async function callRound(roundDir, deps = {}) {
  const {
    secrets,
    loadAdapter = defaultLoadAdapter,
    keyRemainingUsd = orKeyRemaining,
    modelPrices = orModelPrices,
    log = console.log,
    killGraceMs = KILL_GRACE_MS,
    maxAttempts = MAX_ATTEMPTS,
    backoffMs = BACKOFF_MS,
    basePath,
    retryFatal = false,
    searchPath = process.env.PATH,
    sandbox,
  } = deps;
  if (!secrets) throw new Error('call: secrets are required');
  const round = loadRound(roundDir);
  const bwrap = findBwrap(searchPath);
  if (!bwrap) throw new Error('call: bwrap not found on PATH; every agent runs in a bubblewrap sandbox, so install bubblewrap (apt install bubblewrap)');
  const probe = probeBwrap(bwrap);
  if (probe) throw new Error(`call: bwrap probe failed, so agents can't be sandboxed: ${probe}`);
  const { plan } = round;
  const timeoutMs = deps.timeoutMs ?? plan.timeoutSec * 1000;

  const runs = expandRuns(plan);
  const priorResults = new Map(runs.map((r) => [r.id, priorResult(round, r)]));
  const prior = new Map(runs.map((r) => [r.id, priorResults.get(r.id)?.status ?? null]));
  const redo = new Map(runs.map((r) => [r.id, redoReason(priorResults.get(r.id))]));
  const fatal = runs.filter((r) => prior.get(r.id) === 'fatal');
  if (fatal.length > 0 && !retryFatal) {
    throw new Error(
      `call: ${fatal.map((r) => r.id).join(', ')} ended fatal; fix the cause, then pass --retry-fatal to re-run ${fatal.length === 1 ? 'it' : 'them'} first`,
    );
  }
  const pending = [...fatal, ...runs.filter((r) => (!DONE.has(prior.get(r.id)) || redo.get(r.id)) && prior.get(r.id) !== 'fatal')];
  const skipped = runs.length - pending.length;
  if (skipped > 0) log(`skipping ${skipped} finished runs`);
  const redone = pending.filter((r) => redo.get(r.id));
  if (redone.length > 0) log(`redoing ${redone.length} runs that were tainted or had unknown tokens; the old ones are kept beside them`);

  const adapters = new Map();
  for (const name of new Set(pending.map((r) => r.agent))) adapters.set(name, await loadAdapter(name));
  await checkSpend(plan, pending, { secrets, keyRemainingUsd, modelPrices, log });

  const breaker = new CircuitBreaker({ min: 10, ratio: 0.8 });
  let ran = 0;
  for (const run of pending) {
    if (redo.get(run.id)) keepAside(round, run, redo.get(run.id));
    let attempts = 0;
    let outcome;
    for (;;) {
      attempts++;
      outcome = await attemptRun(run, round, adapters.get(run.agent), { secrets, timeoutMs, killGraceMs, basePath, bwrap, sandbox });
      // A rate limit is the provider's; it says nothing about runaway failure.
      if (!outcome.result.rateLimited) breaker.record(outcome.result.status === 'ok');
      if (outcome.kind !== 'infra' || attempts >= maxAttempts || breaker.tripped) break;
      const wait = backoffMs[Math.min(attempts - 1, backoffMs.length - 1)] ?? 0;
      if (wait > 0) {
        log(redact(`${run.id}: infra failure (${outcome.result.error}); retrying in ${Math.round(wait / 1000)} s`, outcome.secrets));
        await new Promise((res) => setTimeout(res, wait));
      }
    }
    const result = { ...outcome.result, attempts };
    writeFileSync(join(round.dir, 'runs', run.id, 'result.json'), redact(`${JSON.stringify(result, null, 2)}\n`, outcome.secrets));
    ran++;
    const taint = result.tainted.length ? ` TAINTED(${result.tainted.length})` : '';
    const leak = result.secretInTranscript ? ' SECRET-IN-OUTPUT' : '';
    log(`[${run.seq}/${runs.length}] ${run.id} ${result.status} ${result.wallMs ?? '-'}ms${taint}${leak}`);
    if (outcome.result.rateLimited && outcome.kind === 'infra') {
      return {
        ran,
        skipped,
        stopped: 'rate_limit',
        reason: `${run.id}: the provider's rate or usage limit persisted over ${attempts} attempts; wait for it to reset, then run call again to resume (the run is redone)`,
        secretRuns: secretRuns(round.dir),
      };
    }
        if (outcome.kind === 'fatal') return { ran, skipped, stopped: 'fatal', reason: redact(`${run.id}: ${result.error}`, outcome.secrets), secretRuns: secretRuns(round.dir) };
    if (breaker.tripped) {
      return { ran, skipped, stopped: 'breaker', reason: `${breaker.failed} of ${breaker.calls} calls failed`, secretRuns: secretRuns(round.dir) };
    }
  }
  return { ran, skipped, stopped: null, reason: null, secretRuns: secretRuns(round.dir) };
}
