// The call phase: run every planned run through its agent adapter, serially
// in seq order, and record each result. Stops on a fatal provider error or
// rejected run, on runaway failure (circuit breaker), and refuses to start
// when the OpenRouter estimate exceeds the key's remaining limit.

import { spawn } from 'node:child_process';
import { StringDecoder } from 'node:string_decoder';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { emptyAgentResult, loadAdapter as defaultLoadAdapter } from './agent.mjs';
import { CircuitBreaker, classify } from './guards.mjs';
import { estimateUsd, keyRemainingUsd as orKeyRemaining, modelPrices as orModelPrices } from './openrouter.mjs';
import { expandRuns, renderPrompt } from './plan.mjs';
import { requireSecret } from './secrets.mjs';
import { taintHits } from './taint.mjs';
import { prepareRun } from './workspace.mjs';

/** Every status a result.json can hold. */
export const STATUSES = new Set(['ok', 'timeout', 'agent_error', 'infra_error', 'fatal']);
/** Statuses a resumed `call` skips. */
export const DONE = new Set(['ok', 'timeout']);
export const MAX_ATTEMPTS = 3;
export const KILL_GRACE_MS = 10_000;
export const REDACTED = '[REDACTED]';

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

function priorStatus(round, run) {
  const file = join(round.dir, 'runs', run.id, 'result.json');
  if (!existsSync(file)) return null;
  try {
    return JSON.parse(readFileSync(file, 'utf8')).status ?? null;
  } catch {
    return null;
  }
}

function resetDir(dir) {
  if (!existsSync(dir)) return;
  // Agents may leave read-only files behind; make them removable.
  const chmod = spawn('chmod', ['-R', 'u+w', dir]);
  return new Promise((res) => chmod.on('close', res).on('error', res)).then(() => rmSync(dir, { recursive: true, force: true }));
}

/**
 * Spawn the agent in its own process group. On timeout, or when
 * `earlyReject(line)` returns a reason for a stdout line, the group gets
 * SIGTERM, then SIGKILL `killGraceMs` later. Wall time runs from spawn to
 * exit. Strays left in the group are killed when the agent exits.
 */
function spawnAgent({ cmd, args = [], env = {}, cwd, input }, { timeoutMs, killGraceMs, earlyReject }) {
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
    const killGroup = (sig) => {
      try {
        if (child?.pid) process.kill(-child.pid, sig);
      } catch {
        // already gone
      }
    };
    const stop = () => {
      killGroup('SIGTERM');
      killTimer = setTimeout(() => killGroup('SIGKILL'), killGraceMs);
    };
    const timer = setTimeout(() => {
      timedOut = true;
      stop();
    }, timeoutMs);
    let closeTimer;
    try {
      child = spawn(cmd, args, { cwd, env, detached: true, stdio: [input == null ? 'ignore' : 'pipe', 'pipe', 'pipe'] });
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
  const { secrets, timeoutMs, killGraceMs, basePath } = opts;
  const { plan } = round;
  const out = join(round.dir, 'runs', run.id);
  await resetDir(out);
  const base = { run, status: 'infra_error', wallMs: null, exitCode: null, signal: null, error: null };

  let dirs;
  try {
    dirs = prepareRun(run, round, { basePath });
  } catch (e) {
    mkdirSync(out, { recursive: true });
    return { kind: 'infra', result: { ...base, error: e.message, ...strip(emptyAgentResult()), tainted: [] }, secrets };
  }

  const arm = plan.arms.find((a) => a.name === run.arm);
  const prompt = renderPrompt(plan.taskDef, arm, round);
  // A throwing adapter (e.g. a missing secret) is a setup error: stop the round.
  const spec = adapter.command(run, dirs, { prompt, secrets, plan, round });
  const hidden = runSecrets(secrets, spec);
  const earlyReject = typeof adapter.earlyReject === 'function' ? (line) => adapter.earlyReject(line) : null;
  const proc = await spawnAgent(spec, { timeoutMs, killGraceMs, earlyReject });
  const stdout = redact(proc.stdout, hidden);
  const stderr = redact(proc.stderr, hidden);
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
    error ??= agent.providerStatus == null ? 'provider error before any turn' : `provider status ${agent.providerStatus} before any turn`;
  } else if (proc.timedOut) {
    status = 'timeout';
  } else if (agent.endedOnError) {
    // An agent that exits 0 after the provider ended its run (pi) did not finish.
    status = 'agent_error';
    error ??= `agent ended on a provider error${agent.providerStatus == null ? '' : ` (${agent.providerStatus})`}`;
  } else if (proc.exitCode === 0 && !error) {
    status = 'ok';
  } else {
    status = 'agent_error';
    error ??= `agent exited ${proc.exitCode ?? proc.signal}`;
  }

  const tainted = taintHits(
    { shellCommands: agent.shellCommands ?? [], readPaths: agent.readPaths ?? [] },
    { taintPaths: round.taintPaths ?? [], ownRunDir: out, cwd: spec.cwd ?? dirs.project },
  );
  return {
    kind,
    result: { ...base, status, wallMs: proc.wallMs, exitCode: proc.exitCode, signal: proc.signal, error, ...strip(agent), tainted },
    secrets: hidden,
  };
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
 *   (3), basePath (PATH for prepareRun), retryFatal (false).
 * @returns {Promise<{ran: number, skipped: number, stopped: null|"fatal"|"breaker", reason: string|null}>}
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
    basePath,
    retryFatal = false,
  } = deps;
  if (!secrets) throw new Error('call: secrets are required');
  const round = loadRound(roundDir);
  const { plan } = round;
  const timeoutMs = deps.timeoutMs ?? plan.timeoutSec * 1000;

  const runs = expandRuns(plan);
  const prior = new Map(runs.map((r) => [r.id, priorStatus(round, r)]));
  const fatal = runs.filter((r) => prior.get(r.id) === 'fatal');
  if (fatal.length > 0 && !retryFatal) {
    throw new Error(
      `call: ${fatal.map((r) => r.id).join(', ')} ended fatal; fix the cause, then pass --retry-fatal to re-run ${fatal.length === 1 ? 'it' : 'them'} first`,
    );
  }
  const pending = [...fatal, ...runs.filter((r) => !DONE.has(prior.get(r.id)) && prior.get(r.id) !== 'fatal')];
  const skipped = runs.length - pending.length;
  if (skipped > 0) log(`skipping ${skipped} finished runs`);

  const adapters = new Map();
  for (const name of new Set(pending.map((r) => r.agent))) adapters.set(name, await loadAdapter(name));
  await checkSpend(plan, pending, { secrets, keyRemainingUsd, modelPrices, log });

  const breaker = new CircuitBreaker({ min: 10, ratio: 0.8 });
  let ran = 0;
  for (const run of pending) {
    let attempts = 0;
    let outcome;
    for (;;) {
      attempts++;
      outcome = await attemptRun(run, round, adapters.get(run.agent), { secrets, timeoutMs, killGraceMs, basePath });
      breaker.record(outcome.result.status === 'ok');
      if (outcome.kind !== 'infra' || attempts >= maxAttempts || breaker.tripped) break;
    }
    const result = { ...outcome.result, attempts };
    writeFileSync(join(round.dir, 'runs', run.id, 'result.json'), redact(`${JSON.stringify(result, null, 2)}\n`, outcome.secrets));
    ran++;
    const taint = result.tainted.length ? ` TAINTED(${result.tainted.length})` : '';
    log(`[${run.seq}/${runs.length}] ${run.id} ${result.status} ${result.wallMs ?? '-'}ms${taint}`);
    if (outcome.kind === 'fatal') return { ran, skipped, stopped: 'fatal', reason: redact(`${run.id}: ${result.error}`, outcome.secrets) };
    if (breaker.tripped) {
      return { ran, skipped, stopped: 'breaker', reason: `${breaker.failed} of ${breaker.calls} calls failed` };
    }
  }
  return { ran, skipped, stopped: null, reason: null };
}
