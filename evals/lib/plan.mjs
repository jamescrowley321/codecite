// Plans: validate a plan spec, freeze it into a round, expand it into runs,
// and render each arm's prompt.

import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mulberry32, shuffle } from './rng.mjs';
import { IN_SANDBOX } from './sandbox.mjs';

export const TASKS_DIR = fileURLToPath(new URL('../tasks/', import.meta.url));

const fail = (field, why) => {
  throw new Error(`plan: ${field} ${why}`);
};
const isStr = (v) => typeof v === 'string' && v.length > 0;
const isPosInt = (v) => Number.isInteger(v) && v > 0;

/** Throws naming the first bad field; returns the spec unchanged. */
export function validatePlan(spec) {
  if (spec === null || typeof spec !== 'object' || Array.isArray(spec)) fail('plan', 'must be an object');
  if (!isStr(spec.task)) fail('task', 'must be a task id');
  if (!isPosInt(spec.runs)) fail('runs', 'must be a positive integer');
  if (!Number.isInteger(spec.seed)) fail('seed', 'must be an integer');
  if (!isPosInt(spec.timeoutSec)) fail('timeoutSec', 'must be a positive integer');
  if (!spec.repos || typeof spec.repos !== 'object' || Object.keys(spec.repos).length === 0) {
    fail('repos', 'must map each corpus repo name to a local path');
  }
  for (const [name, path] of Object.entries(spec.repos)) {
    if (!isStr(path)) fail(`repos.${name}`, 'must be a path');
  }
  if (!Array.isArray(spec.agents) || spec.agents.length === 0) fail('agents', 'must be a non-empty array');
  const pairs = new Set();
  spec.agents.forEach((a, i) => {
    if (!isStr(a?.agent)) fail(`agents[${i}].agent`, 'must be a string');
    if (!isStr(a?.model)) fail(`agents[${i}].model`, 'must be a string');
    const key = `${a.agent} ${a.model}`;
    if (pairs.has(key)) fail(`agents[${i}]`, 'repeats an agent/model pair');
    pairs.add(key);
  });
  if (!Array.isArray(spec.arms) || spec.arms.length === 0) fail('arms', 'must be a non-empty array');
  const names = new Set();
  spec.arms.forEach((arm, i) => {
    if (!isStr(arm?.name) || !/^[a-z0-9-]+$/.test(arm.name)) fail(`arms[${i}].name`, 'must be [a-z0-9-]+');
    if (names.has(arm.name)) fail(`arms[${i}].name`, 'is repeated');
    names.add(arm.name);
    if (arm.codecite !== undefined && typeof arm.codecite !== 'boolean') fail(`arms[${i}].codecite`, 'must be a boolean');
    if (arm.note !== undefined && !isStr(arm.note)) fail(`arms[${i}].note`, 'must be a string');
  });
  const baseline = spec.arms.find((a) => a.name === 'baseline');
  if (!baseline) fail('arms', 'needs an arm named "baseline"');
  if (baseline.codecite || baseline.note) fail('arms.baseline', 'must have no codecite and no note');
  return spec;
}

/** Read and validate a plan spec file. */
export function loadPlan(path) {
  let spec;
  try {
    spec = JSON.parse(readFileSync(path, 'utf8'));
  } catch (e) {
    throw new Error(`plan: cannot read ${path}: ${e.message}`);
  }
  return validatePlan(spec);
}

/** Throws when `dir` (or its nearest existing ancestor) is inside a git work tree. */
export function assertOutsideGit(dir) {
  let probe = resolve(dir);
  while (!existsSync(probe)) probe = dirname(probe);
  const r = spawnSync('git', ['-C', probe, 'rev-parse', '--is-inside-work-tree'], { encoding: 'utf8' });
  if (r.status === 0 && r.stdout.trim() === 'true') {
    throw new Error(`rounds: ${dir} is inside a git work tree; set CODECITE_EVALS_DIR outside any repo`);
  }
}

/** Where rounds live: $CODECITE_EVALS_DIR or ~/.local/state/codecite-evals, never inside git. */
export function roundsDir() {
  const dir = resolve(process.env.CODECITE_EVALS_DIR || join(homedir(), '.local', 'state', 'codecite-evals'));
  assertOutsideGit(dir);
  return dir;
}

/** Load a task definition: task.json plus its directory and prompt text. */
export function loadTask(id, tasksDir = TASKS_DIR) {
  const dir = join(tasksDir, id);
  const def = JSON.parse(readFileSync(join(dir, 'task.json'), 'utf8'));
  for (const f of ['project', 'prompt', 'hidden']) if (!isStr(def[f])) fail(`task ${id}: ${f}`, 'is missing');
  if (!isStr(def.corpus?.repo) || !isStr(def.corpus?.commit)) fail(`task ${id}: corpus`, 'needs repo and commit');
  if (!Array.isArray(def.setup)) fail(`task ${id}: setup`, 'must be a command array');
  for (const f of ['score', 'scoreWarm']) {
    if (!Array.isArray(def[f]) || def[f].length === 0 || !def[f].every((x) => typeof x === 'string')) {
      fail(`task ${id}: ${f}`, f === 'scoreWarm' ? 'must be a command array (it fetches the scoring deps, so a fetch failure is never scored against the agent)' : 'must be a command array');
    }
  }
  return { ...def, dir, promptText: readFileSync(join(dir, def.prompt), 'utf8') };
}

/**
 * Freeze a spec into <roundsDir>/<roundId>/plan.json and return the frozen
 * Plan. The task dir (task.json, prompt, project/, hidden/, reference/, ...)
 * is copied into <round>/task and taskDef.dir points there, so runs and
 * scoring never read the live checkout. The round dir is hidden from agents.
 */
export function freezePlan(spec, { codeciteCommit, codeciteDirty = false, roundId, tasksDir = TASKS_DIR }) {
  validatePlan(spec);
  if (!isStr(roundId) || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(roundId)) fail('roundId', 'must be [A-Za-z0-9._-]+');
  if (!isStr(codeciteCommit)) fail('codeciteCommit', 'is missing');
  const live = loadTask(spec.task, tasksDir);
  if (!spec.repos[live.corpus.repo]) fail('repos', `has no path for "${live.corpus.repo}"`);
  const dir = join(roundsDir(), roundId);
  const taskDef = { ...live, dir: join(dir, 'task') };
  const plan = {
    ...spec,
    roundId,
    frozenAt: new Date().toISOString(),
    codeciteCommit,
    codeciteDirty,
    identityModelCommit: taskDef.corpus.commit,
    taskDef,
  };
  mkdirSync(dir, { recursive: true });
  try {
    writeFileSync(join(dir, 'plan.json'), `${JSON.stringify(plan, null, 2)}\n`, { flag: 'wx' });
  } catch (e) {
    if (e.code === 'EEXIST') throw new Error(`plan: round ${roundId} already exists at ${dir}`);
    throw e;
  }
  cpSync(live.dir, taskDef.dir, { recursive: true, filter: (src) => !/(^|\/)(\.venv|__pycache__|node_modules|\.pytest_cache)$/.test(src) });
  return plan;
}

/**
 * An arm's opaque attribution label (pi's OpenRouter X-Title): stable per
 * round and arm, so OpenRouter splits spend by arm, but naming neither, since
 * the agent can read it. The scorecard prints the mapping.
 */
export function armLabel(roundId, arm) {
  return `eval-${createHash('sha256').update(`${roundId}\0${arm}`).digest('hex').slice(0, 8)}`;
}

/** All runs of a plan, in a seeded shuffled order. */
export function expandRuns(plan) {
  const runs = [];
  for (const { agent, model } of plan.agents) {
    for (const arm of plan.arms) {
      for (let rep = 1; rep <= plan.runs; rep++) {
        runs.push({ id: `${agent}-${model.replaceAll('/', '_')}-${arm.name}-${rep}`, agent, model, arm: arm.name, rep });
      }
    }
  }
  return shuffle(runs, mulberry32(plan.seed)).map((r, i) => ({ ...r, seq: i + 1 }));
}

/**
 * The prompt for one arm: the task prompt, plus a blank line and the arm's
 * note. `{identity_model}` is the export's in-sandbox path, the same for
 * every round (never the round's host path, which names the round).
 */
export function renderPrompt(task, arm) {
  const sub = (s) => s.replaceAll('{identity_model}', IN_SANDBOX.export);
  const base = sub(task.promptText);
  if (!arm.note) return base;
  const note = sub(arm.note);
  return `${base}${base.endsWith('\n') ? '\n' : '\n\n'}${note}${note.endsWith('\n') ? '' : '\n'}`;
}
