import { test } from 'node:test';
import assert from 'node:assert/strict';
import { taintHits } from './taint.mjs';

const ROUND = '/state/evals/r1';
const OWN = `${ROUND}/runs/pi-x-baseline-1`;
const ctx = {
  taintPaths: ['/home/u/repos/auth/identity-model', `${ROUND}/plan.json`, `${ROUND}/round.json`, `${ROUND}/corpus.toml`, `${ROUND}/runs`],
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
