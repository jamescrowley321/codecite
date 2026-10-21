import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { startIdp } from './idp.mjs';
import { loadTask } from './plan.mjs';
import { findBwrap, findOnPath } from './sandbox.mjs';
import { parseJunit, scoreCommand, scoreRun } from './score.mjs';
import { cleanPath, REPO_ROOT } from './workspace.mjs';

const BWRAP = findBwrap(process.env.PATH);
const noBwrap = BWRAP ? false : 'bwrap is not on PATH; install bubblewrap to run the scoring tests';

test('scoreCommand substitutes {hidden}', () => {
  const task = { dir: '/t', hidden: 'hidden', score: ['uv', 'run', 'pytest', '-q', '{hidden}', '--x={hidden}/a'] };
  assert.deepEqual(scoreCommand(task), ['uv', 'run', 'pytest', '-q', '/t/hidden', '--x=/t/hidden/a']);
  assert.throws(() => scoreCommand({ dir: '/t', hidden: 'hidden' }), /score/);
});

test('the task scores without pytest writing a cache', () => {
  const task = loadTask('fastapi-middleware');
  assert.ok(task.score.includes('-p') && task.score.includes('no:cacheprovider'), task.score.join(' '));
});

test('parseJunit lists failures, errors and skips', () => {
  const xml = `<?xml version="1.0" encoding="utf-8"?><testsuites><testsuite name="pytest" errors="1" failures="1" skipped="1" tests="4">
<testcase classname="test_hidden" name="test_ok" time="0.1" />
<testcase classname="test_hidden" name="test_bad" time="0.1"><failure message="assert 401 == 200">x</failure></testcase>
<testcase classname="test_hidden" name="test_err" time="0.1"><error message="fixture">y</error></testcase>
<testcase classname="test_hidden" name="test_skip[a&amp;b]" time="0.0"><skipped message="no" /></testcase>
</testsuite></testsuites>`;
  assert.deepEqual(parseJunit(xml), {
    tests: 4,
    failures: ['test_hidden::test_bad', 'test_hidden::test_err', 'test_hidden::test_skip[a&b] (skipped)'],
  });
  assert.deepEqual(parseJunit('<testsuites><testsuite tests="0"></testsuite></testsuites>'), { tests: 0, failures: [] });
  assert.equal(parseJunit('garbage'), null);
});

// A synthetic round on disk, for the sandbox checks.
let base;
before(() => {
  base = mkdtempSync(join(tmpdir(), 'evals-score-'));
});
after(() => rmSync(base, { recursive: true, force: true }));

test("scoring can't read the owner's secrets or other runs, and reads hidden/ read-only", { skip: noBwrap }, async () => {
  const dir = join(base, 'probe');
  const owner = join(dir, 'owner');
  mkdirSync(owner, { recursive: true });
  const planted = join(owner, 'evals.env');
  writeFileSync(planted, 'OPENROUTER_EVAL_API_KEY=planted\n');
  const roundDir = join(dir, 'round');
  const runDir = join(roundDir, 'runs', 'me');
  mkdirSync(join(runDir, 'project'), { recursive: true });
  mkdirSync(join(runDir, 'home'), { recursive: true });
  writeFileSync(join(runDir, 'home', 'agent-home.txt'), 'planted\n');
  mkdirSync(join(roundDir, 'runs', 'sibling', 'project'), { recursive: true });
  writeFileSync(join(roundDir, 'runs', 'sibling', 'project', 'answer.py'), 'planted\n');
  writeFileSync(join(roundDir, 'plan.json'), '{}\n');
  const taskDir = join(dir, 'task');
  mkdirSync(join(taskDir, 'hidden'), { recursive: true });
  writeFileSync(join(taskDir, 'hidden', 'test_probe.py'), 'planted\n');
  mkdirSync(join(taskDir, 'reference'), { recursive: true });
  writeFileSync(join(taskDir, 'reference', 'main.py'), 'planted\n');

  // Exit codes only: nothing planted is ever printed.
  const script = [
    'for p in "$@"; do if test -r "$p"; then echo "LEAK $p"; fi; done',
    `test -r "${join(taskDir, 'hidden', 'test_probe.py')}" && echo HIDDEN-READABLE`,
    `touch "${join(taskDir, 'hidden', 'x')}" 2>/dev/null && echo HIDDEN-WRITABLE`,
    'touch made-by-score && echo PROJECT-WRITABLE',
    'echo "IDP=$TEST_IDP SENTINEL=${LEAK_SENTINEL:-none}"',
    'exit 3',
  ].join('\n');
  const task = {
    dir: taskDir,
    hidden: 'hidden',
    setup: [],
    score: ['/bin/sh', '-c', script, 'probe', planted, join(roundDir, 'plan.json'), join(roundDir, 'runs', 'sibling', 'project', 'answer.py'), join(runDir, 'home', 'agent-home.txt'), join(taskDir, 'reference', 'main.py'), join(homedir(), '.config', 'codecite', 'evals.env'), join(homedir(), '.claude', '.credentials.json')],
  };
  const round = { dir: roundDir, export: null, taintPaths: [taskDir], plan: { taskDef: task } };
  process.env.LEAK_SENTINEL = '1';
  let r;
  try {
    r = await scoreRun(runDir, task, 'http://127.0.0.1:9', { round, bwrap: BWRAP });
  } finally {
    delete process.env.LEAK_SENTINEL;
  }
  const log = readFileSync(join(runDir, 'score.log'), 'utf8');
  assert.doesNotMatch(log, /LEAK/, log);
  assert.match(log, /HIDDEN-READABLE/, log);
  assert.doesNotMatch(log, /HIDDEN-WRITABLE/, log);
  assert.match(log, /PROJECT-WRITABLE/, log);
  assert.match(log, /IDP=http:\/\/127\.0\.0\.1:9 SENTINEL=none/, log);
  assert.ok(existsSync(join(runDir, 'project', 'made-by-score')));
  assert.equal(r.passed, false);
  assert.ok(r.failures.length > 0);
  const saved = JSON.parse(readFileSync(join(runDir, 'score.json'), 'utf8'));
  assert.equal(saved.passed, false);
  assert.equal(saved.exitCode, 3);
});

test('a score command that exits 0 without a test report does not pass', { skip: noBwrap }, async () => {
  const runDir = join(base, 'zero', 'round', 'runs', 'me');
  mkdirSync(join(runDir, 'project'), { recursive: true });
  mkdirSync(join(base, 'zero', 'hidden'), { recursive: true });
  const task = { dir: join(base, 'zero'), hidden: 'hidden', setup: [], score: ['/bin/true', '{hidden}'] };
  const round = { dir: join(base, 'zero', 'round'), export: null, taintPaths: [], plan: { taskDef: task } };
  const r = await scoreRun(runDir, task, 'http://127.0.0.1:9', { round, bwrap: BWRAP });
  assert.equal(r.passed, false);
  assert.match(r.failures.join(' '), /no test report/);
});

// The real task: the reference passes, the starting project fails.
const IDENTITY_MODEL_REPO = process.env.IDENTITY_MODEL_REPO ?? join(homedir(), 'repos', 'auth', 'identity-model');
const UV = findOnPath('uv', cleanPath());
const IDP_READY = existsSync(join(REPO_ROOT, 'evals', 'fixtures', 'node-oidc', 'node_modules'));
const realSkip =
  noBwrap ||
  (!UV && 'uv is not on PATH') ||
  (!existsSync(join(IDENTITY_MODEL_REPO, '.git')) && `no identity-model repo at ${IDENTITY_MODEL_REPO} (set IDENTITY_MODEL_REPO)`) ||
  (!IDP_READY && 'the IdP fixture has no node_modules (npm ci in evals/fixtures/node-oidc)');

test('the reference passes and the starting project fails', { skip: realSkip, timeout: 600_000 }, async (t) => {
  const task = loadTask('fastapi-middleware');
  const roundDir = join(base, 'real');
  const exportDir = join(roundDir, 'identity-model');
  mkdirSync(exportDir, { recursive: true });
  const archive = spawnSync('sh', ['-c', `git -C "$1" archive "$2" | tar -x -C "$3"`, 'sh', IDENTITY_MODEL_REPO, task.corpus.commit, exportDir], {
    encoding: 'utf8',
  });
  assert.equal(archive.status, 0, archive.stderr);

  const prepare = (id, mainPy) => {
    const runDir = join(roundDir, 'runs', id);
    const project = join(runDir, 'project');
    cpSync(join(task.dir, task.project), project, { recursive: true, filter: (s) => !/(^|\/)(\.venv|__pycache__)$/.test(s) });
    const pyproject = join(project, 'pyproject.toml');
    writeFileSync(pyproject, readFileSync(pyproject, 'utf8').replaceAll('@IDENTITY_MODEL@', exportDir));
    if (mainPy) cpSync(mainPy, join(project, 'app', 'main.py'));
    const sync = spawnSync(UV, ['sync', '--quiet'], { cwd: project, encoding: 'utf8', env: { ...process.env, PATH: cleanPath() } });
    assert.equal(sync.status, 0, sync.stderr);
    return runDir;
  };
  const refDir = prepare('reference', join(task.dir, 'reference', 'app', 'main.py'));
  const startDir = prepare('start', null);

  let idp;
  try {
    idp = await startIdp();
  } catch (e) {
    t.skip(`the IdP would not start: ${e.message}`);
    return;
  }
  try {
    const round = { dir: roundDir, export: exportDir, taintPaths: [REPO_ROOT], plan: { taskDef: task } };
    const ref = await scoreRun(refDir, task, idp.url, { round, bwrap: BWRAP });
    assert.deepEqual(ref, { passed: true, failures: [] }, readFileSync(join(refDir, 'score.log'), 'utf8').slice(-3000));
    const start = await scoreRun(startDir, task, idp.url, { round, bwrap: BWRAP });
    assert.equal(start.passed, false);
    assert.ok(start.failures.length >= 5, start.failures.join('\n'));
    const saved = JSON.parse(readFileSync(join(startDir, 'score.json'), 'utf8'));
    assert.equal(saved.passed, false);
    assert.equal(saved.tests, 7);
  } finally {
    await idp.stop();
  }
});
