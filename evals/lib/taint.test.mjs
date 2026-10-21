import { test } from 'node:test';
import assert from 'node:assert/strict';
import { taintHits } from './taint.mjs';

// Host paths: the round and the real repos. The agent never sees them (the
// sandbox hides them); it works at the neutral in-sandbox paths below.
const ROUND = '/state/evals/r1';
const WORK = '/work';
const PROJECT = `${WORK}/project`;
const EXPORT = `${WORK}/identity-model`;
const ctx = {
  taintPaths: ['/home/u/repos/auth/identity-model', '/home/u/repos/codecite', `${ROUND}/plan.json`, `${ROUND}/round.json`, `${ROUND}/corpus.toml`, `${ROUND}/runs`, `${ROUND}/task`],
  ownRunDir: WORK,
  cwd: PROJECT,
};
const hits = (shellCommands = [], readPaths = []) => taintHits({ shellCommands, readPaths }, ctx);

test('flags git clone in a bash call', () => {
  assert.equal(hits(['cd /tmp && git clone https://example.org/x.git']).length, 1);
});

test('flags each network pattern', () => {
  for (const c of [
    'curl https://x.org',
    'wget http://x.org/a',
    'pip install requests',
    'pip3 install requests',
    'python -m pip install requests',
    'uv add httpx',
    'echo github.com',
    'curl -sL https://raw.githubusercontent.com/o/r/main/x.py',
    'wget https://objects.githubusercontent.com/x.tar.gz',
    'python -c "import urllib.request as u; u.urlopen(\'https://raw.githubusercontent.com/o/r/x.py\')"',
  ]) {
    assert.ok(hits([c]).length > 0, c);
  }
});

test('uv pip install taints: it fetches a package the task does not have', () => {
  // The spec lists `pip install`; the prompt says "no new packages".
  assert.equal(hits(['uv pip install pytest']).length, 1);
});

test('ignores look-alikes', () => {
  assert.deepEqual(hits(['echo curly', 'uv sync', 'uv run pytest -q', 'ls -la', 'pip list', 'uv pip list']), []);
});

test('a curl or wget of loopback only does not taint', () => {
  const ok = [
    'curl -s http://127.0.0.1:8000/health',
    'curl -s "http://localhost:8000/items" -H "Authorization: Bearer x"',
    'curl http://[::1]:8000/',
    'curl -o out.json localhost:8000/items',
    'wget -qO- http://localhost:8000/health',
    'uv run uvicorn app.main:app & sleep 2; curl -s 127.0.0.1:8000/health; kill %1',
  ];
  for (const c of ok) assert.deepEqual(hits([c]), [], c);
});

test('a curl that also reaches a non-loopback host, or names no host, taints', () => {
  const bad = ['curl -s http://127.0.0.1:8000/ https://example.org/', 'curl -s $URL', 'curl example.org', 'curl http://127.0.0.1.evil.org/', 'curl http://localhost.evil.org/'];
  for (const c of bad) assert.equal(hits([c]).length, 1, c);
});

test('ignores github.com inside a file the agent read', () => {
  // Only commands and paths are scanned; file contents never reach taintHits.
  assert.deepEqual(hits(['cat README.md'], [`${PROJECT}/README.md`]), []);
});

test('flags a find that reaches evals/tasks', () => {
  assert.equal(hits(["find / -path '*evals/tasks*' -name '*.py'"]).length, 1);
  assert.equal(hits(['ls /home/u/repos/codecite-worktrees']).length, 1);
  assert.equal(hits(['grep -r foo ../test_hidden.py']).length, 1);
});

test('flags a read path into the hidden tests', () => {
  assert.equal(hits([], ['/home/u/repos/codecite/evals/tasks/x/hidden/test_hidden.py']).length, 1);
  assert.equal(hits([], [`${ROUND}/task/hidden/conftest.py`]).length, 1);
  assert.equal(hits([`cat ${ROUND}/task/reference/app/main.py`]).length, 1);
});

test('flags a look at the mount table, which names the host dirs behind /work', () => {
  assert.equal(hits(['cat /proc/self/mountinfo']).length, 1);
  assert.equal(hits(['findmnt']).length, 1);
  assert.equal(hits([], ['/proc/1/mountinfo']).length, 1);
  assert.deepEqual(hits(['df -h .', 'mount | head -0 || true']), [], '/proc/mounts and df show no host paths');
});

test("flags a read of another run's project by its host path", () => {
  assert.equal(hits([], [`${ROUND}/runs/pi-x-codecite-1/project/app/main.py`]).length, 1);
  assert.equal(hits([`cat ${ROUND}/runs/pi-x-codecite-1/project/app/main.py`]).length, 1);
});

test('flags cat of plan.json', () => {
  assert.equal(hits([`cat ${ROUND}/plan.json`]).length, 1);
  assert.equal(hits([`cat "${ROUND}/round.json"`]).length, 1);
});

test('flags the real repo, but not a longer sibling path', () => {
  assert.equal(hits(['ls /home/u/repos/auth/identity-model/py']).length, 1);
  assert.deepEqual(hits(['ls /home/u/repos/auth/identity-model-other']), []);
  assert.equal(hits(['cd /home/u/repos/auth && cat identity-model/x.py']).length, 1);
  assert.equal(hits([], ['/home/u/repos/codecite/README.md']).length, 1);
});

test('one hit per offending command or path', () => {
  assert.equal(hits([`git clone https://github.com/x/y ${ROUND}/plan.json`]).length, 1);
});

test('walking up from the project does not taint: the round is not reachable from the sandbox', () => {
  // The ancestor rule is gone: the round dir and everything above it are
  // hidden, so a walk up only finds the neutral /work and the host's /.
  const fine = [
    'ls /',
    'find / -name middleware.py 2>/dev/null',
    'grep -rn "/" app/',
    "python - <<'PY'\nprint(a / b)\nPY",
    'ls ../../..',
    'cd ../../.. && cat plan.json',
    'find ../../.. -name "*.py"',
    'cd / && ls',
    'du -sh /',
  ];
  for (const c of fine) assert.deepEqual(hits([c]), [], c);
  assert.deepEqual(hits([], ['/', '../../..', WORK, '../../../plan.json']), []);
});

test("ignores the run's own paths, absolute and relative", () => {
  assert.deepEqual(
    hits(
      [`ls ${PROJECT}`, 'ls .', 'ls ./app', 'cd app && cat main.py', `cd ${WORK} && ls project`, 'cat ../home/.bashrc', 'cat app/main.py'],
      [PROJECT, '.', 'app', `${PROJECT}/app/main.py`, 'app/main.py'],
    ),
    [],
  );
});

test('a normal session has no hits', () => {
  const shell = ['ls', 'ls -la', 'cat app/main.py', 'uv run pytest -q', 'uv sync', `grep -rn TokenValidation ${EXPORT}/py`, `cat ${EXPORT}/py/packages/x/middleware.py`, 'python -c "import sys; print(sys.path)"', "sed -i 's/a/b/' app/main.py", 'codecite search eval "token validation"', '/usr/bin/env python --version', 'echo $HOME | wc -c'];
  const reads = ['app/main.py', `${PROJECT}/pyproject.toml`, `${EXPORT}/py/packages/x/middleware.py`, `${EXPORT}/README.md`];
  assert.deepEqual(hits(shell, reads), []);
});
