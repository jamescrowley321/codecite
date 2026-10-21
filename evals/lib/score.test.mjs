import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { startIdp } from './idp.mjs';
import { loadTask } from './plan.mjs';
import { findBwrap, findOnPath } from './sandbox.mjs';
import { parseJunit, scoreCommand, scoreRound, scoreRun } from './score.mjs';
import { cleanPath, REPO_ROOT } from './workspace.mjs';

const BWRAP = findBwrap(process.env.PATH);
const noBwrap = BWRAP ? false : 'bwrap is not on PATH; install bubblewrap to run the scoring tests';

test('scoreCommand substitutes {hidden}', () => {
  const task = { dir: '/t', hidden: 'hidden', score: ['uv', 'run', 'pytest', '-q', '{hidden}', '--x={hidden}/a'] };
  assert.deepEqual(scoreCommand(task), ['uv', 'run', 'pytest', '-q', '/t/hidden', '--x=/t/hidden/a']);
  assert.throws(() => scoreCommand({ dir: '/t', hidden: 'hidden' }), /score/);
});

test("task.json pins the scoring deps and turns off pytest's cache", () => {
  const task = loadTask('fastapi-middleware');
  assert.ok(task.score.includes('-p') && task.score.includes('no:cacheprovider'), task.score.join(' '));
  assert.ok(task.score.includes('pytest==9.1.1') && task.score.includes('httpx==0.28.1'), task.score.join(' '));
});

/**
 * A stand-in IdP: its discovery document answers 200 for the first `upFor`
 * requests, then 503.
 */
async function fakeIdp(upFor = Infinity) {
  let n = 0;
  const srv = createServer((req, res) => {
    n++;
    res.writeHead(req.url === '/.well-known/openid-configuration' && n <= upFor ? 200 : 503, { 'content-type': 'application/json' });
    res.end('{}');
  });
  await new Promise((r) => srv.listen(0, '127.0.0.1', r));
  return { url: `http://127.0.0.1:${srv.address().port}`, stop: () => new Promise((r) => srv.close(r)) };
}
// Nothing listens on the discard port.
const DEAD_IDP = 'http://127.0.0.1:9';
// A score command that writes a one-test passing JUnit report where
// PYTEST_ADDOPTS asks for it.
const PASSING = ['/bin/sh', '-c', 'printf \'<testsuite tests="1"><testcase classname="c" name="t"/></testsuite>\' > "${PYTEST_ADDOPTS#--junitxml=}"', '{hidden}'];

/** A synthetic round under `base`: one task with hidden/, and `ids` runs with a project. */
function synthRound(name, score, ids = ['me']) {
  const dir = join(base, name);
  const roundDir = join(dir, 'round');
  mkdirSync(join(dir, 'task', 'hidden'), { recursive: true });
  const task = { id: 't', dir: join(dir, 'task'), hidden: 'hidden', setup: [], score };
  for (const id of ids) mkdirSync(join(roundDir, 'runs', id, 'project'), { recursive: true });
  const round = { dir: roundDir, export: null, taintPaths: [task.dir], plan: { taskDef: task } };
  return { roundDir, task, round, runDir: (id = ids[0]) => join(roundDir, 'runs', id) };
}

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
  const idp = await fakeIdp();
  process.env.LEAK_SENTINEL = '1';
  let r;
  try {
    r = await scoreRun(runDir, task, idp.url, { round, bwrap: BWRAP });
  } finally {
    delete process.env.LEAK_SENTINEL;
    await idp.stop();
  }
  const log = readFileSync(join(runDir, 'score.log'), 'utf8');
  assert.doesNotMatch(log, /LEAK/, log);
  assert.match(log, /HIDDEN-READABLE/, log);
  assert.doesNotMatch(log, /HIDDEN-WRITABLE/, log);
  assert.match(log, /PROJECT-WRITABLE/, log);
  assert.match(log, new RegExp(`IDP=${idp.url} SENTINEL=none`), log);
  assert.ok(existsSync(join(runDir, 'project', 'made-by-score')));
  // No JUnit report: unscored, not failed.
  assert.equal(r.passed, null);
  assert.match(r.unscored, /no test report \(exit 3\)/);
  const saved = JSON.parse(readFileSync(join(runDir, 'score.json'), 'utf8'));
  assert.equal(saved.passed, null);
  assert.equal(saved.exitCode, 3);
});

test('a score command that exits 0 without a test report is unscored, never a pass', { skip: noBwrap }, async () => {
  const { task, round, runDir } = synthRound('zero', ['/bin/true', '{hidden}']);
  const idp = await fakeIdp();
  try {
    const r = await scoreRun(runDir(), task, idp.url, { round, bwrap: BWRAP });
    assert.equal(r.passed, null);
    assert.match(r.unscored, /no test report/);
  } finally {
    await idp.stop();
  }
});

test('a report with every test passing and exit 0 passes', { skip: noBwrap }, async () => {
  const { task, round, runDir } = synthRound('pass', PASSING);
  const idp = await fakeIdp();
  try {
    assert.deepEqual(await scoreRun(runDir(), task, idp.url, { round, bwrap: BWRAP }), { passed: true, failures: [] });
  } finally {
    await idp.stop();
  }
});

const UV_ON_PATH = findOnPath('uv', cleanPath());
test('uv failing to fetch the scoring deps leaves the run unscored, not failed', { skip: noBwrap || (!UV_ON_PATH && 'uv is not on PATH') }, async () => {
  // --offline with a version no cache holds: uv fails before pytest, as it does when PyPI is down.
  const { task, round, runDir } = synthRound('uv-down', ['uv', 'run', '--offline', '--no-project', '--with', 'pytest==0.0.1', 'pytest', '-q', '{hidden}']);
  const idp = await fakeIdp();
  try {
    const r = await scoreRun(runDir(), task, idp.url, { round, bwrap: BWRAP });
    assert.equal(r.passed, null, readFileSync(join(runDir(), 'score.log'), 'utf8'));
    assert.match(r.unscored, /no test report \(exit [1-9]/);
    assert.deepEqual(r.failures, []);
  } finally {
    await idp.stop();
  }
});

test('the IdP down before scoring leaves the run unscored', { skip: noBwrap }, async () => {
  const { task, round, runDir } = synthRound('idp-before', PASSING);
  const r = await scoreRun(runDir(), task, DEAD_IDP, { round, bwrap: BWRAP });
  assert.equal(r.passed, null);
  assert.match(r.unscored, /IdP down before scoring/);
  assert.equal(existsSync(join(runDir(), 'score', 'report.xml')), false, 'the command never ran');
});

test('the IdP down after scoring leaves the run unscored, even with a passing report', { skip: noBwrap }, async () => {
  const { task, round, runDir } = synthRound('idp-after', PASSING);
  const idp = await fakeIdp(1);
  try {
    const r = await scoreRun(runDir(), task, idp.url, { round, bwrap: BWRAP });
    assert.equal(r.passed, null);
    assert.match(r.unscored, /IdP down after scoring: discovery returned 503/);
    assert.ok(existsSync(join(runDir(), 'score', 'report.xml')), 'the command ran');
  } finally {
    await idp.stop();
  }
});

/** Plan, round and result.json files on disk for scoreRound. */
function roundForScoreRound(name, score) {
  const s = synthRound(name, score, ['a-m-baseline-1', 'a-m-codecite-1']);
  const plan = { roundId: name, runs: 1, seed: 1, agents: [{ agent: 'a', model: 'm' }], arms: [{ name: 'baseline' }, { name: 'codecite', codecite: true }], taskDef: s.task };
  writeFileSync(join(s.roundDir, 'plan.json'), JSON.stringify(plan));
  writeFileSync(join(s.roundDir, 'round.json'), JSON.stringify({ dir: s.roundDir, export: null, taintPaths: [s.task.dir] }));
  for (const id of ['a-m-baseline-1', 'a-m-codecite-1']) writeFileSync(join(s.runDir(id), 'result.json'), JSON.stringify({ status: 'ok' }));
  return s;
}

test('scoreRound tries unscored runs again and skips scored ones', { skip: noBwrap }, async () => {
  const { roundDir, runDir } = roundForScoreRound('round-retry', PASSING);
  writeFileSync(join(runDir('a-m-baseline-1'), 'score.json'), JSON.stringify({ passed: null, failures: [], unscored: 'no test report' }));
  writeFileSync(join(runDir('a-m-codecite-1'), 'score.json'), JSON.stringify({ passed: false, failures: ['kept'] }));
  const idp = await fakeIdp();
  try {
    const summary = await scoreRound(roundDir, { idpUrl: idp.url, bwrap: BWRAP, log: () => {} });
    assert.deepEqual(summary, { scored: 1, unscored: 0, skipped: 1 });
    assert.equal(JSON.parse(readFileSync(join(runDir('a-m-baseline-1'), 'score.json'), 'utf8')).passed, true);
    assert.deepEqual(JSON.parse(readFileSync(join(runDir('a-m-codecite-1'), 'score.json'), 'utf8')).failures, ['kept']);
  } finally {
    await idp.stop();
  }
});

test('scoreRound stops when the IdP is down, recording the run as unscored', { skip: noBwrap }, async () => {
  const { roundDir, runDir } = roundForScoreRound('round-idp-down', PASSING);
  await assert.rejects(scoreRound(roundDir, { idpUrl: DEAD_IDP, bwrap: BWRAP, log: () => {} }), /IdP down before scoring/);
  const saved = ['a-m-baseline-1', 'a-m-codecite-1']
    .map((id) => join(runDir(id), 'score.json'))
    .filter((f) => existsSync(f))
    .map((f) => JSON.parse(readFileSync(f, 'utf8')));
  assert.equal(saved.length, 1, 'it stopped after the first run');
  assert.equal(saved[0].passed, null);
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
    for (const d of [join(refDir, 'project'), join(startDir, 'project'), join(task.dir, task.hidden)]) {
      assert.equal(existsSync(join(d, '.pytest_cache')), false, `no .pytest_cache in ${d}`);
    }
  } finally {
    await idp.stop();
  }
});
