// The score phase: run the task's hidden tests against each run's project,
// inside the same bubblewrap sandbox the agent ran in (the score command
// imports the agent's code), and record score.json beside result.json.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { loadRound, roundSandbox } from './call.mjs';
import { expandRuns } from './plan.mjs';
import { findBwrap, findOnPath, probeBwrap } from './sandbox.mjs';
import { cleanPath } from './workspace.mjs';

/** Run statuses whose project is scored (and that count toward a verdict). */
export const SCORED = new Set(['ok', 'timeout', 'agent_error']);
export const SCORE_TIMEOUT_MS = 600_000;

/** The task's score command with `{hidden}` set to its hidden/ dir. */
export function scoreCommand(task) {
  if (!Array.isArray(task.score) || task.score.length === 0) throw new Error(`score: task ${task.id ?? task.dir} has no score command`);
  const hidden = resolve(task.dir, task.hidden);
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

/**
 * Score one run: the task's score command in `runDir/project`, with
 * TEST_IDP=idpUrl and `{hidden}` set to the task's hidden/ dir, inside the
 * round's agent sandbox (the round dir, its taintPaths and the owner's home
 * hidden; the project read-write; the export, the score command's tools, the
 * venv's Python and hidden/ read-only; the network shared, so the IdP on
 * localhost is reachable). The run's own home/ is not bound: scoring gets a
 * fresh `runDir/score/` as HOME (uv's cache lives there). It passes only when
 * the command exits 0 and its JUnit report (asked for via PYTEST_ADDOPTS)
 * shows at least one test and no failure, error or skip.
 *
 * A fault of the scoring harness is never the run's result: when the IdP's
 * discovery document doesn't answer before or after the command, or the
 * command leaves no JUnit report (uv couldn't fetch pytest from PyPI, the
 * network was down, or the process ended before pytest reported), the run
 * is unscored: `passed: null` with the reason in `unscored`, which keeps it
 * out of the verdict (its pair reads INCOMPLETE) until it is scored again.
 * A score command that runs past `timeoutMs` is a failure: every hidden
 * test's IdP call has its own 10 s timeout and uv's fetches theirs, so only
 * the agent's code hangs that long. Writes score.json and score.log.
 *
 * @param {object} opts  `round` (default: loaded from runDir's round dir),
 *   `bwrap` (default: on PATH), `basePath` (the PATH to clean, default
 *   process.env.PATH), `timeoutMs` (default 10 min), `sandbox` ({ro, rw,
 *   alwaysHidden}: tests only).
 * @returns {Promise<{passed: boolean|null, failures: string[], unscored?: string}>}
 */
export async function scoreRun(runDir, task, idpUrl, opts = {}) {
  const { basePath = process.env.PATH, timeoutMs = SCORE_TIMEOUT_MS, sandbox = {} } = opts;
  const bwrap = opts.bwrap ?? findBwrap(process.env.PATH);
  if (!bwrap) throw new Error('score: bwrap not found on PATH; scoring runs the agent\'s code in the same bubblewrap sandbox');
  const round = opts.round ?? loadRound(dirname(dirname(resolve(runDir))));
  const project = join(runDir, 'project');
  const hidden = resolve(task.dir, task.hidden);
  if (!isDir(hidden)) throw new Error(`score: the task's hidden tests are not at ${hidden}`);
  const argv = scoreCommand(task);

  const home = join(runDir, 'score');
  rmSync(home, { recursive: true, force: true });
  mkdirSync(home, { recursive: true });
  const report = join(home, 'report.xml');
  const record = (fields) => {
    const { passed, failures, unscored } = fields;
    writeFileSync(join(runDir, 'score.json'), `${JSON.stringify({ passed, failures, ...fields, command: argv, idpUrl, scoredAt: new Date().toISOString() }, null, 2)}\n`);
    return unscored ? { passed, failures, unscored } : { passed, failures };
  };
  const unscoredRecord = (reason, fields = {}) => record({ passed: null, failures: [], unscored: reason, tests: null, exitCode: null, ...fields });
  if (!isDir(project)) {
    writeFileSync(join(runDir, 'score.log'), '');
    return unscoredRecord('no project directory');
  }

  const path = cleanPath(basePath);
  const cmd = findOnPath(argv[0], path);
  if (!cmd) throw new Error(`score: \`${argv[0]}\` not found on PATH (${path})`);
  const env = {
    PATH: path,
    HOME: home,
    XDG_CACHE_HOME: join(home, '.cache'),
    XDG_CONFIG_HOME: join(home, '.config'),
    LANG: 'C.UTF-8',
    TEST_IDP: idpUrl,
    PYTHONDONTWRITEBYTECODE: '1',
    PYTEST_ADDOPTS: `--junitxml=${report}`,
    UV_PYTHON_DOWNLOADS: 'never',
    UV_NO_PROGRESS: '1',
  };
  const spec = { cmd, args: argv.slice(1), env, cwd: project };
  const dirs = { project, home, xdg: null, bin: null };
  // The agent's sandbox for this task: the same hides, export and tool binds.
  const scoringRound = { ...round, plan: { ...round.plan, taskDef: task } };
  const sb = roundSandbox(spec, dirs, scoringRound, {
    bwrap,
    sandbox: { ro: [hidden, ...(sandbox.ro ?? [])], rw: sandbox.rw ?? [], alwaysHidden: sandbox.alwaysHidden },
  });
  const before = await idpProblem(idpUrl);
  if (before) {
    writeFileSync(join(runDir, 'score.log'), '');
    return unscoredRecord(`IdP down before scoring: ${before}`);
  }
  const proc = await spawnScore(sb, project, timeoutMs);
  writeFileSync(join(runDir, 'score.log'), proc.output);
  if (proc.spawnError) throw new Error(`score: spawn ${bwrap}: ${proc.spawnError}`);
  // bwrap failing to build the sandbox is the harness's fault, not the run's.
  const bwrapError = /^bwrap: .*/.exec(proc.output)?.[0];
  if (proc.exitCode !== 0 && bwrapError && proc.output.trim() === bwrapError.trim()) throw new Error(`score: sandbox failed: ${bwrapError}`);

  const procFields = { exitCode: proc.exitCode, signal: proc.signal, timedOut: proc.timedOut, wallMs: proc.wallMs };
  const after = await idpProblem(idpUrl);
  if (after) return unscoredRecord(`IdP down after scoring: ${after}`, procFields);

  const parsed = existsSync(report) ? parseJunit(readFileSync(report, 'utf8')) : null;
  const lastLine = proc.output.trim().split('\n').pop() ?? '';
  if (!parsed && !proc.timedOut) {
    return unscoredRecord(`no test report (exit ${proc.exitCode ?? proc.signal})${lastLine ? `: ${lastLine.slice(0, 300)}` : ''}`, procFields);
  }
  const failures = [];
  if (proc.timedOut) failures.push(`score timed out after ${Math.round(timeoutMs / 1000)} s`);
  if (parsed) {
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

/**
 * Score a round: every run whose result.json status is in SCORED and that
 * has no pass/fail score yet (all of them with `rescore`); unscored runs are
 * tried again. Infra and fatal runs are not scored; they never count. Stops
 * (throws) when the IdP goes down, after recording that run as unscored.
 *
 * @returns {Promise<{scored: number, unscored: number, skipped: number}>}
 */
export async function scoreRound(roundDir, { idpUrl, rescore = false, log = console.log, bwrap = findBwrap(process.env.PATH), ...opts }) {
  if (!bwrap) throw new Error('score: bwrap not found on PATH; install bubblewrap (apt install bubblewrap)');
  const probe = probeBwrap(bwrap);
  if (probe) throw new Error(`score: bwrap probe failed, so scoring can't be sandboxed: ${probe}`);
  const round = loadRound(roundDir);
  const task = round.plan.taskDef;
  let scored = 0;
  let unscored = 0;
  let skipped = 0;
  for (const run of expandRuns(round.plan)) {
    const runDir = join(round.dir, 'runs', run.id);
    let status;
    try {
      status = JSON.parse(readFileSync(join(runDir, 'result.json'), 'utf8')).status;
    } catch {
      skipped++;
      continue; // never called
    }
    let prior = null;
    try {
      prior = JSON.parse(readFileSync(join(runDir, 'score.json'), 'utf8'));
    } catch {
      // not scored yet
    }
    // An unscored run (passed: null) is always tried again.
    if (!SCORED.has(status) || (!rescore && typeof prior?.passed === 'boolean')) {
      skipped++;
      continue;
    }
    const r = await scoreRun(runDir, task, idpUrl, { ...opts, round, bwrap });
    if (r.unscored) {
      unscored++;
      log(`${run.id} unscored: ${r.unscored}`);
      // Every later run would be unscored too.
      if (/^IdP down/.test(r.unscored)) throw new Error(`score: stopped at ${run.id}: ${r.unscored}`);
      continue;
    }
    scored++;
    log(`${run.id} ${r.passed ? 'passed' : `failed (${r.failures.length}: ${r.failures.slice(0, 3).join('; ')})`}`);
  }
  return { scored, unscored, skipped };
}
