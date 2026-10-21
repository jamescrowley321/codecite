import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync, existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { loadPlan, loadTask, expandRuns, renderPrompt, freezePlan, roundsDir } from './plan.mjs';

function spec(over = {}) {
  return {
    task: 'fastapi-middleware',
    runs: 20,
    seed: 20261020,
    timeoutSec: 1200,
    repos: { 'identity-model': '~/repos/auth/identity-model' },
    agents: [
      { agent: 'claude', model: 'claude-sonnet-5-5' },
      { agent: 'pi', model: 'google/gemini-4-argon' },
      { agent: 'pi', model: 'openai/gpt-6.1-sol' },
      { agent: 'pi', model: 'deepseek/deepseek-v4.1-flash' },
    ],
    arms: [
      { name: 'baseline' },
      { name: 'codecite', codecite: true, note: 'Use codecite on `{identity_model}`.' },
    ],
    ...over,
  };
}

function writeSpec(s) {
  const dir = mkdtempSync(join(tmpdir(), 'plan-'));
  const path = join(dir, 'plan.json');
  writeFileSync(path, JSON.stringify(s));
  return path;
}

test('rejects a plan without a baseline arm', () => {
  const path = writeSpec(spec({ arms: [{ name: 'codecite', codecite: true, note: 'x' }] }));
  assert.throws(() => loadPlan(path), /arms/);
});

test('rejects a bad field and names it', () => {
  assert.throws(() => loadPlan(writeSpec(spec({ runs: 0 }))), /runs/);
  assert.throws(() => loadPlan(writeSpec(spec({ seed: 'x' }))), /seed/);
  assert.throws(() => loadPlan(writeSpec(spec({ agents: [{ agent: 'pi' }] }))), /agents\[0\]\.model/);
  assert.throws(() => loadPlan(writeSpec(spec({ repos: {} }))), /repos/);
});

test('loads the round-1 plan', () => {
  const plan = loadPlan(new URL('../plans/round-1.json', import.meta.url).pathname);
  assert.equal(plan.runs, 20);
  assert.equal(plan.seed, 20261020);
  assert.equal(expandRuns(plan).length, 160);
});

test('expandRuns gives runs × agents × arms runs', () => {
  const plan = loadPlan(writeSpec(spec()));
  assert.equal(expandRuns(plan).length, 20 * 4 * 2);
});

test('same seed gives the same order', () => {
  const plan = loadPlan(writeSpec(spec()));
  const a = expandRuns(plan).map((r) => r.id);
  const b = expandRuns(plan).map((r) => r.id);
  assert.deepEqual(a, b);
  const c = expandRuns({ ...plan, seed: plan.seed + 1 }).map((r) => r.id);
  assert.notDeepEqual(a, c);
  assert.deepEqual(expandRuns(plan).map((r) => r.seq), a.map((_, i) => i + 1));
});

test('ids are unique', () => {
  const runs = expandRuns(loadPlan(writeSpec(spec())));
  assert.equal(new Set(runs.map((r) => r.id)).size, runs.length);
  assert.ok(runs.some((r) => r.id === 'pi-google_gemini-4-argon-codecite-1'));
});

test('renderPrompt: arms identical up to the appended note, the export at its in-sandbox path', () => {
  const task = { promptText: 'Source at `{identity_model}`.\nDo it.\n' };
  // The round's host export path never reaches the prompt: it names the round.
  const round = { export: '/state/codecite-evals/r1/identity-model' };
  const base = renderPrompt(task, { name: 'baseline' }, round);
  const cc = renderPrompt(task, { name: 'codecite', codecite: true, note: 'Corpus at `{identity_model}`.' }, round);
  assert.equal(base, 'Source at `/work/identity-model`.\nDo it.\n');
  assert.ok(cc.startsWith(base));
  assert.equal(cc.slice(base.length), '\nCorpus at `/work/identity-model`.\n');
});

test('freezePlan writes plan.json under the rounds dir', () => {
  const dir = mkdtempSync(join(tmpdir(), 'rounds-'));
  const old = process.env.CODECITE_EVALS_DIR;
  process.env.CODECITE_EVALS_DIR = dir;
  try {
    assert.equal(roundsDir(), dir);
    const plan = freezePlan(loadPlan(writeSpec(spec())), { codeciteCommit: 'abc', roundId: 'r1' });
    const path = join(dir, 'r1', 'plan.json');
    assert.ok(existsSync(path));
    const saved = JSON.parse(readFileSync(path, 'utf8'));
    assert.equal(saved.codeciteCommit, 'abc');
    assert.equal(saved.identityModelCommit, '9458d60f4f2f20227bddf2594fd7d10cba66ea0a');
    assert.match(saved.taskDef.promptText, /\{identity_model\}/);
    assert.deepEqual(saved, plan);
    // The task is frozen with the plan: copied into the round, which runs
    // and scoring read from, never the live checkout.
    const frozen = join(dir, 'r1', 'task');
    assert.equal(saved.taskDef.dir, frozen);
    for (const f of ['task.json', 'prompt.md', 'project/app/main.py', 'project/pyproject.toml', 'hidden/test_hidden.py', 'hidden/conftest.py', 'reference/app/main.py', 'check.sh']) {
      assert.ok(existsSync(join(frozen, f)), f);
    }
    assert.equal(existsSync(join(frozen, 'project', '.venv')), false);
    assert.throws(() => freezePlan(loadPlan(writeSpec(spec())), { codeciteCommit: 'abc', roundId: 'r1' }), /exists/);
  } finally {
    if (old === undefined) delete process.env.CODECITE_EVALS_DIR;
    else process.env.CODECITE_EVALS_DIR = old;
  }
});

test('loadTask and freezePlan reject a task with no scoreWarm, naming it', () => {
  const tasksDir = mkdtempSync(join(tmpdir(), 'evals-tasks-'));
  const write = (id, extra) => {
    mkdirSync(join(tasksDir, id), { recursive: true });
    writeFileSync(join(tasksDir, id, 'prompt.md'), 'Do it.\n');
    const def = { id, project: 'project', prompt: 'prompt.md', hidden: 'hidden', corpus: { repo: 'identity-model', commit: 'abc' }, setup: [], score: ['true'], ...extra };
    writeFileSync(join(tasksDir, id, 'task.json'), JSON.stringify(def));
  };
  write('nowarm', {});
  write('emptywarm', { scoreWarm: [] });
  write('noscore', { scoreWarm: ['true'], score: undefined });
  write('ok', { scoreWarm: ['true'] });
  assert.throws(() => loadTask('nowarm', tasksDir), /task nowarm: scoreWarm/);
  assert.throws(() => loadTask('emptywarm', tasksDir), /task emptywarm: scoreWarm/);
  assert.throws(() => loadTask('noscore', tasksDir), /task noscore: score/);
  assert.deepEqual(loadTask('ok', tasksDir).scoreWarm, ['true']);
  assert.throws(() => freezePlan({ ...spec(), task: 'nowarm' }, { codeciteCommit: 'abc', roundId: 'r-nowarm', tasksDir }), /task nowarm: scoreWarm/);
});

test('the real task has a warm step', () => {
  assert.ok(loadTask('fastapi-middleware').scoreWarm.length > 0);
});
