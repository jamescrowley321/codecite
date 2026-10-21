import { test } from 'node:test';
import assert from 'node:assert/strict';
import { taintHits } from './taint.mjs';

const ROUND = '/state/evals/r1';
const OWN = `${ROUND}/runs/pi-x-baseline-1`;
const EXPORT = `${ROUND}/identity-model`;
const ctx = {
  taintPaths: ['/home/u/repos/auth/identity-model', '/home/u/repos/codecite', `${ROUND}/plan.json`, `${ROUND}/round.json`, `${ROUND}/corpus.toml`, `${ROUND}/runs`],
  ownRunDir: OWN,
  cwd: `${OWN}/project`,
};
const hits = (shellCommands = [], readPaths = []) => taintHits({ shellCommands, readPaths }, ctx);

test('flags git clone in a bash call', () => {
  assert.equal(hits(['cd /tmp && git clone https://example.org/x.git']).length, 1);
});

test('flags each network pattern', () => {
  for (const c of ['curl https://x.org', 'wget http://x.org/a', 'pip install requests', 'uv add httpx', 'echo github.com']) {
    assert.ok(hits([c]).length > 0, c);
  }
});

test('ignores look-alikes', () => {
  assert.deepEqual(hits(['echo curly', 'uv sync', 'uv run pytest -q', 'ls -la']), []);
});

test('ignores github.com inside a file the agent read', () => {
  // Only commands and paths are scanned; file contents never reach taintHits.
  assert.deepEqual(hits(['cat README.md'], [`${OWN}/project/README.md`]), []);
});

test('flags a find that reaches evals/tasks', () => {
  assert.equal(hits(["find / -path '*evals/tasks*' -name '*.py'"]).length, 1);
  assert.equal(hits(['ls /home/u/repos/codecite-worktrees']).length, 1);
  assert.equal(hits(['grep -r foo ../test_hidden.py']).length, 1);
});

test('flags a read path into the hidden tests', () => {
  assert.equal(hits([], ['/home/u/repos/codecite/evals/tasks/x/hidden/test_hidden.py']).length, 1);
});

test('flags a read of another run\'s project', () => {
  assert.equal(hits([], [`${ROUND}/runs/pi-x-codecite-1/project/app/main.py`]).length, 1);
  assert.equal(hits([`cat ${ROUND}/runs/pi-x-codecite-1/project/app/main.py`]).length, 1);
});

test('flags a relative path out of the run', () => {
  assert.equal(hits(['cat ../../pi-x-codecite-1/project/app/main.py']).length, 1);
  assert.equal(hits([], ['../../../plan.json']).length, 1);
});

test('ignores the run\'s own project path', () => {
  assert.deepEqual(hits([`cat ${OWN}/project/app/main.py`, 'cat app/main.py'], [`${OWN}/project/app/main.py`, 'app/main.py']), []);
});

test('flags cat of plan.json', () => {
  assert.equal(hits([`cat ${ROUND}/plan.json`]).length, 1);
  assert.equal(hits([`cat "${ROUND}/round.json"`]).length, 1);
});

test('flags the real repo, but not a longer sibling path', () => {
  assert.equal(hits(['ls /home/u/repos/auth/identity-model/py']).length, 1);
  assert.deepEqual(hits(['ls /home/u/repos/auth/identity-model-other']), []);
});

test('one hit per offending command or path', () => {
  assert.equal(hits([`git clone https://github.com/x/y ${ROUND}/plan.json`]).length, 1);
});

test('flags relative walks up to the round', () => {
  const cases = [
    'cd ../../.. && cat plan.json',
    'ls ../../..',
    'find ../../.. -name "*.py"',
    `grep -rn foo ${ROUND}`,
    'cd /home/u/repos/auth && cat identity-model/x.py',
    'cd ../../.. && ls',
    'cd .. ; cd .. ; cat ../plan.json',
    'cd ../.. && cat other-run/project/app/main.py',
  ];
  for (const c of cases) assert.equal(hits([c]).length, 1, c);
});

test('flags read paths that reach the round', () => {
  assert.equal(hits([], ['../../..']).length, 1);
  assert.equal(hits([], [ROUND]).length, 1);
  assert.equal(hits([], ['/home/u/repos/codecite/README.md']).length, 1);
});

test('a cd into an ancestor alone does not taint, and cwd does not carry across commands', () => {
  assert.deepEqual(hits(['cd ../../..', 'cat plan.json']), []);
});

test('ignores the run\'s own project path, absolute and relative', () => {
  assert.deepEqual(
    hits(
      [`ls ${OWN}/project`, 'ls .', 'ls ./app', 'cd app && cat main.py', `cd ${OWN} && ls project`, 'cat ../home/.bashrc'],
      [`${OWN}/project`, '.', 'app', `${OWN}/project/app/main.py`],
    ),
    [],
  );
});

test('a normal session has no hits', () => {
  const shell = ['ls', 'ls -la', 'cat app/main.py', 'uv run pytest -q', 'uv sync', `grep -rn TokenValidation ${EXPORT}/py`, `cat ${EXPORT}/py/packages/x/middleware.py`, 'python -c "import sys; print(sys.path)"', "sed -i 's/a/b/' app/main.py", 'codecite search eval "token validation"', '/usr/bin/env python --version', 'echo $HOME | wc -c'];
  const reads = ['app/main.py', `${OWN}/project/pyproject.toml`, `${EXPORT}/py/packages/x/middleware.py`, `${EXPORT}/README.md`];
  assert.deepEqual(hits(shell, reads), []);
});
