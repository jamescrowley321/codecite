// The score phase: run the task's hidden tests against each run's project,
// inside the same bubblewrap sandbox the agent ran in (the score command
// imports the agent's code), and record score.json beside result.json.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadRound, roundSandbox } from './call.mjs';
import { expandRuns } from './plan.mjs';
import { findBwrap, findOnPath, IN_SANDBOX, probeBwrap } from './sandbox.mjs';
import { cleanPath } from './workspace.mjs';

/** Run statuses whose project is scored (and that count toward a verdict). */
export const SCORED = new Set(['ok', 'timeout', 'agent_error']);
export const SCORE_TIMEOUT_MS = 600_000;

/**
 * The task's score command with `{hidden}` set to where its hidden/ dir is
 * bound in the scoring sandbox (default IN_SANDBOX.hidden).
 */
export function scoreCommand(task, hidden = IN_SANDBOX.hidden) {
  if (!Array.isArray(task.score) || task.score.length === 0) throw new Error(`score: task ${task.id ?? task.dir} has no score command`);
  return task.score.map((a) => a.replaceAll('{hidden}', hidden));
}

const unescape = (s) =>
  s
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(Number(n)))
    .replace(/&amp;/g, '&');

const attr = (attrs, name) => {
  const m = new RegExp(`\\b${name}="([^"]*)"`).exec(attrs);
  return m ? unescape(m[1]) : '';
};

/**
 * A pytest JUnit XML report as {tests, failures}, where `failures` names
 * every test case that failed, errored (collection errors included) or was
 * skipped. Null when `xml` is no report.
 */
export function parseJunit(xml) {
  if (typeof xml !== 'string' || !/<testsuite\b/.test(xml)) return null;
  const failures = [];
  let tests = 0;
  const re = /<testcase\b([^>]*?)(?:\/>|>([\s\S]*?)<\/testcase>)/g;
  for (const m of xml.matchAll(re)) {
    tests++;
    const cls = attr(m[1], 'classname');
    const name = attr(m[1], 'name');
    const id = cls ? `${cls}::${name}` : name;
    const body = m[2] ?? '';
    if (/<(failure|error)\b/.test(body)) failures.push(id);
    else if (/<skipped\b/.test(body)) failures.push(`${id} (skipped)`);
  }
  return { tests, failures };
}

/** Spawn the sandboxed score command; on timeout SIGKILL bwrap's group (its pid namespace goes with it). */
function spawnScore(sb, cwd, timeoutMs) {
  return new Promise((resolvePromise) => {
    const out = [];
    const t0 = performance.now();
    let timedOut = false;
    const child = spawn(sb.cmd, sb.args, { cwd, env: sb.env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const timer = setTimeout(() => {
      timedOut = true;
      try {
        process.kill(-child.pid, 'SIGKILL');
      } catch {
        // gone
      }
    }, timeoutMs);
    child.stdout.on('data', (c) => out.push(c));
    child.stderr.on('data', (c) => out.push(c));
    let spawnError = null;
    child.on('error', (e) => {
      spawnError = e.message;
    });
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      resolvePromise({
        output: Buffer.concat(out).toString('utf8'),
        exitCode: code,
        signal,
        timedOut,
        spawnError,
        wallMs: Math.round(performance.now() - t0),
      });
    });
  });
}

const isDir = (p) => {
  try {
    return statSync(p).isDirectory();
  } catch {
    return false;
  }
};

/** The pytest plugin dir (read-only in the sandbox) holding the started marker. */
export const PLUGIN_DIR = fileURLToPath(new URL('./pytest-plugin/', import.meta.url)).replace(/\/$/, '');
export const PLUGIN = 'codecite_score_started';

/** The base Python of the project's venv (pyvenv.cfg `home`), else null. */
function venvBasePython(project) {
  try {
    const cfg = readFileSync(join(project, '.venv', 'pyvenv.cfg'), 'utf8');
    const home = /^home\s*=\s*(.+?)\s*$/m.exec(cfg)?.[1];
    if (!home) return null;
    return ['python3', 'python'].map((n) => join(home, n)).find((p) => existsSync(p)) ?? null;
  } catch {
    return null;
  }
}

const tail = (output) => {
  const last = output.trim().split('\n').pop() ?? '';
  return last ? `: ${last.slice(0, 300)}` : '';
};

/**
 * Score one run: the task's score command in `runDir/project`, with
 * TEST_IDP=idpUrl and `{hidden}` set to IN_SANDBOX.hidden, inside the
 * round's agent sandbox (the round dir, its taintPaths and the owner's home
 * hidden; the project read-write at the agent's own /work/project; the
 * export at /work/identity-model, the score command's tools, the venv's
 * Python, hidden/ and the marker plugin read-only; the network shared, so
 * the IdP on localhost is reachable). The run's own home/ is not bound:
 * scoring gets a fresh `runDir/score/` as HOME, at /work/score (uv's cache
 * lives there). It passes only when the command exits 0 and its JUnit report
 * (asked for via PYTEST_ADDOPTS) shows at least one test and no failure,
 * error or skip.
 *
 * Two stages, so a fault of the scoring harness is never the run's result
 * and the agent's own breakage always is:
 *
 * - Infra (unscored, `passed: null`, the reason in `unscored`): the IdP's
 *   discovery document not answering before or after; the task's
 *   `scoreWarm` command (fetching the pinned scoring deps into this
 *   cache, outside the agent's project) failing or timing out; and the score
 *   command timing out before pytest started.
 * - Agent (failed): once the warm step succeeded, no report is a failure:
 *   either before pytest started (the agent's project failed to sync, or
 *   uv failed on it) or after (its code ended the process). A timeout after
 *   pytest started is a failure too. A plugin loaded with `-p` writes
 *   `score/started` before any conftest imports the agent's code.
 *
 * A plan frozen without `scoreWarm` (loadTask now requires it) can't tell
 * the two apart before pytest starts, so there no report before pytest
 * started is unscored.
 *
 * Writes score.json and score.log. A missing project/ is a harness fault:
 * it throws.
 *
 * @param {object} opts  `round` (default: loaded from runDir's round dir),
 *   `bwrap` (default: on PATH), `basePath` (the PATH to clean, default
 *   process.env.PATH), `timeoutMs` (each stage, default 10 min), `sandbox`
 *   ({ro, rw, alwaysHidden}: tests only).
 * @returns {Promise<{passed: boolean|null, failures: string[], unscored?: string}>}
 */
export async function scoreRun(runDir, task, idpUrl, opts = {}) {
  const { basePath = process.env.PATH, timeoutMs = SCORE_TIMEOUT_MS, sandbox = {} } = opts;
  const bwrap = opts.bwrap ?? findBwrap(process.env.PATH);
  if (!bwrap) throw new Error('score: bwrap not found on PATH; scoring runs the agent\'s code in the same bubblewrap sandbox');
  const round = opts.round ?? loadRound(dirname(dirname(resolve(runDir))));
  const project = join(runDir, 'project');
  if (!isDir(project)) throw new Error(`score: ${project} is missing; the run dir is damaged`);
  const hidden = resolve(task.dir, task.hidden);
  if (!isDir(hidden)) throw new Error(`score: the task's hidden tests are not at ${hidden}`);
  const argv = scoreCommand(task);
  const warm = Array.isArray(task.scoreWarm) && task.scoreWarm.length > 0 ? task.scoreWarm : null;

  // Host paths, and where the scoring sandbox sees them: the project and
  // export at the agent's own in-sandbox paths (its venv and lock name them),
  // a fresh score home, hidden/ and the marker plugin read-only.
  const home = join(runDir, 'score');
  rmSync(home, { recursive: true, force: true });
  mkdirSync(home, { recursive: true });
  const report = join(home, 'report.xml');
  const started = join(home, 'started');
  const at = IN_SANDBOX;
  const logs = [];
  const record = (fields) => {
    const { passed, failures, unscored } = fields;
    writeFileSync(join(runDir, 'score.log'), logs.join(''));
    writeFileSync(join(runDir, 'score.json'), `${JSON.stringify({ passed, failures, ...fields, command: argv, warm, idpUrl, scoredAt: new Date().toISOString() }, null, 2)}\n`);
    return unscored ? { passed, failures, unscored } : { passed, failures };
  };
  const unscoredRecord = (reason, fields = {}) => record({ passed: null, failures: [], unscored: reason, tests: null, exitCode: null, ...fields });

  const path = cleanPath(basePath);
  const env = {
    PATH: path,
    HOME: at.score,
    XDG_CACHE_HOME: join(at.score, '.cache'),
    XDG_CONFIG_HOME: join(at.score, '.config'),
    LANG: 'C.UTF-8',
    TEST_IDP: idpUrl,
    PYTHONDONTWRITEBYTECODE: '1',
    UV_PYTHON_DOWNLOADS: 'never',
    UV_NO_PROGRESS: '1',
  };
  const dirs = { project: at.project, home: at.score, xdg: null, bin: null, host: { project, home, xdg: null, bin: null } };
  // The agent's sandbox for this task: the same hides, export and tool binds.
  const scoringRound = { ...round, plan: { ...round.plan, taskDef: task } };
  const sandboxed = (cmdArgv, extraEnv, cwd) => {
    const cmd = findOnPath(cmdArgv[0], path);
    if (!cmd) throw new Error(`score: \`${cmdArgv[0]}\` not found on PATH (${path})`);
    const spec = { cmd, args: cmdArgv.slice(1), env: { ...env, ...extraEnv }, cwd };
    return roundSandbox(spec, dirs, scoringRound, {
      bwrap,
      sandbox: { ro: [[hidden, at.hidden], [PLUGIN_DIR, at.plugin], ...(sandbox.ro ?? [])], rw: sandbox.rw ?? [], alwaysHidden: sandbox.alwaysHidden },
    });
  };
  // `cwd` is the host dir bwrap starts in; the sandbox's own is the spec's.
  const run = async (sb, cwd, label) => {
    const proc = await spawnScore(sb, cwd, timeoutMs);
    logs.push(`== ${label}\n`, proc.output, proc.output.endsWith('\n') || !proc.output ? '' : '\n');
    if (proc.spawnError) throw new Error(`score: spawn ${bwrap}: ${proc.spawnError}`);
    // bwrap failing to build the sandbox is the harness's fault, not the run's.
    const bwrapError = /^bwrap: .*/.exec(proc.output)?.[0];
    if (proc.exitCode !== 0 && bwrapError && proc.output.trim() === bwrapError.trim()) throw new Error(`score: sandbox failed: ${bwrapError}`);
    return proc;
  };

  const before = await idpProblem(idpUrl);
  if (before) return unscoredRecord(`IdP down before scoring: ${before}`);

  // Infra stage: fetch the pinned scoring deps with the venv's base Python,
  // outside the agent's project, so PyPI and the network are known good.
  if (warm) {
    const py = venvBasePython(project);
    const w = await run(sandboxed(warm, py ? { UV_PYTHON: py } : {}, at.score), home, 'warm');
    if (w.timedOut) return unscoredRecord(`warm step timed out after ${Math.round(timeoutMs / 1000)} s`, { warmExitCode: w.exitCode });
    if (w.exitCode !== 0) return unscoredRecord(`warm step failed (exit ${w.exitCode ?? w.signal})${tail(w.output)}`, { warmExitCode: w.exitCode });
  }

  // Agent stage: the score command in the agent's project.
  const proc = await run(
    sandboxed(argv, { PYTEST_ADDOPTS: `-p ${PLUGIN} --junitxml=${join(at.score, 'report.xml')}`, PYTHONPATH: at.plugin, CODECITE_SCORE_STARTED: join(at.score, 'started') }, at.project),
    project,
    'score',
  );
  const pytestStarted = existsSync(started);
  const procFields = { exitCode: proc.exitCode, signal: proc.signal, timedOut: proc.timedOut, wallMs: proc.wallMs, pytestStarted };
  const after = await idpProblem(idpUrl);
  if (after) return unscoredRecord(`IdP down after scoring: ${after}`, procFields);
  if (proc.timedOut && !pytestStarted) return unscoredRecord(`score timed out after ${Math.round(timeoutMs / 1000)} s before pytest started`, procFields);

  const parsed = existsSync(report) ? parseJunit(readFileSync(report, 'utf8')) : null;
  // A plan frozen before scoreWarm existed: with no warm step, a fetch
  // failure and a broken project look the same before pytest starts, so
  // neither is scored against the agent.
  if (!warm && !parsed && !pytestStarted) {
    return unscoredRecord(`no test report before pytest started, and the plan has no warm step to tell a scoring fault from the project's (exit ${proc.exitCode ?? proc.signal})${tail(proc.output)}`, procFields);
  }
  const failures = [];
  if (proc.timedOut) failures.push(`score timed out after ${Math.round(timeoutMs / 1000)} s`);
  if (!parsed) {
    const exit = `exit ${proc.exitCode ?? proc.signal}`;
    failures.push(
      pytestStarted
        ? `no test report, pytest started: the process ended before reporting (${exit})${tail(proc.output)}`
        : `no test report, before pytest started: the project failed to sync or import (${exit})${tail(proc.output)}`,
    );
  } else {
    failures.push(...parsed.failures);
    if (parsed.tests === 0) failures.push('no tests ran');
    if (proc.exitCode !== 0 && failures.length === 0) failures.push(`exit ${proc.exitCode ?? proc.signal}`);
  }
  const passed = !proc.timedOut && proc.exitCode === 0 && parsed != null && parsed.tests > 0 && failures.length === 0;
  return record({ passed, failures, tests: parsed?.tests ?? null, ...procFields });
}

/** Null when the IdP's discovery document answers 200, else what went wrong. */
export async function idpProblem(idpUrl, timeoutMs = 10_000) {
  try {
    const res = await fetch(`${idpUrl.replace(/\/$/, '')}/.well-known/openid-configuration`, { signal: AbortSignal.timeout(timeoutMs) });
    await res.arrayBuffer();
    return res.status === 200 ? null : `discovery returned ${res.status}`;
  } catch (e) {
    return `discovery failed: ${e.cause?.code ?? e.message}`;
  }
}

/** Attempts at a run that keeps coming back unscored before it is a fatal scoring error. */
export const MAX_SCORE_ATTEMPTS = 3;

/**
 * Score a round: every run whose result.json status is in SCORED and that
 * has no pass/fail score yet (all of them with `rescore`). Infra and fatal
 * runs are not scored; they never count. An unscored run is tried up to
 * MAX_SCORE_ATTEMPTS times in all (across `score` calls), then recorded as a
 * fatal scoring error (`fatal: true` in score.json), which later calls skip
 * until `rescore`. Only warm-step failures and timeouts before pytest
 * started count; an IdP outage doesn't, and stops (throws) the phase after
 * recording that run as unscored.
 *
 * @returns {Promise<{scored: number, unscored: number, skipped: number, fatal: string[]}>}
 *   `fatal`: every run of the round that is a fatal scoring error.
 */
export async function scoreRound(roundDir, { idpUrl, rescore = false, log = console.log, bwrap = findBwrap(process.env.PATH), maxAttempts = MAX_SCORE_ATTEMPTS, ...opts }) {
  if (!bwrap) throw new Error('score: bwrap not found on PATH; install bubblewrap (apt install bubblewrap)');
  const probe = probeBwrap(bwrap);
  if (probe) throw new Error(`score: bwrap probe failed, so scoring can't be sandboxed: ${probe}`);
  const round = loadRound(roundDir);
  const task = round.plan.taskDef;
  let scored = 0;
  let unscored = 0;
  let skipped = 0;
  const fatal = [];
  const readScore = (runDir) => {
    try {
      return JSON.parse(readFileSync(join(runDir, 'score.json'), 'utf8'));
    } catch {
      return null;
    }
  };
  for (const run of expandRuns(round.plan)) {
    const runDir = join(round.dir, 'runs', run.id);
    let status;
    try {
      status = JSON.parse(readFileSync(join(runDir, 'result.json'), 'utf8')).status;
    } catch {
      skipped++;
      continue; // never called
    }
    const prior = readScore(runDir);
    if (!SCORED.has(status) || (!rescore && typeof prior?.passed === 'boolean')) {
      skipped++;
      continue;
    }
    if (!rescore && prior?.fatal === true) {
      skipped++;
      fatal.push(run.id);
      continue;
    }
    let attempts = !rescore && prior?.passed === null ? (prior.scoreAttempts ?? 0) : 0;
    let r;
    for (;;) {
      r = await scoreRun(runDir, task, idpUrl, { ...opts, round, bwrap });
      if (!r.unscored) break;
      // The IdP is the harness's own: an outage never counts toward the
      // cap, and every later run would be unscored too, so stop.
      const idpDown = /^IdP down/.test(r.unscored);
      if (!idpDown) attempts++;
      const saved = readScore(runDir);
      const giveUp = !idpDown && attempts >= maxAttempts;
      writeFileSync(join(runDir, 'score.json'), `${JSON.stringify({ ...saved, scoreAttempts: attempts, fatal: giveUp }, null, 2)}\n`);
      if (idpDown) {
        log(`${run.id} unscored: ${r.unscored}`);
        throw new Error(`score: stopped at ${run.id}: ${r.unscored}`);
      }
      log(`${run.id} unscored (attempt ${attempts} of ${maxAttempts}): ${r.unscored}`);
      if (giveUp) break;
    }
    if (r.unscored) {
      unscored++;
      fatal.push(run.id);
      log(`${run.id} could not be scored after ${attempts} attempts: a fatal scoring error`);
      continue;
    }
    scored++;
    log(`${run.id} ${r.passed ? 'passed' : `failed (${r.failures.length}: ${r.failures.slice(0, 3).join('; ')})`}`);
  }
  return { scored, unscored, skipped, fatal };
}
