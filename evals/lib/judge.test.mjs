import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runDiff } from './judge.mjs';

test('runDiff refuses a run whose app/ holds a symlink, and never reads its target', () => {
  const root = mkdtempSync(join(tmpdir(), 'judge-'));
  try {
    const secret = join(root, 'evals.env');
    writeFileSync(secret, 'OPENROUTER_API_KEY=sk-or-planted\n');
    const task = { dir: join(root, 'task'), project: 'project' };
    mkdirSync(join(task.dir, 'project', 'app'), { recursive: true });
    writeFileSync(join(task.dir, 'project', 'app', 'main.py'), 'app = 1\n');
    const run = join(root, 'run');
    mkdirSync(join(run, 'project', 'app'), { recursive: true });
    writeFileSync(join(run, 'project', 'app', 'main.py'), 'app = 2\n');

    assert.match(runDiff(task, run), /\+app = 2/);
    symlinkSync(secret, join(run, 'project', 'app', 'leak.py'));
    assert.throws(() => runDiff(task, run), (e) => /symlinks.*leak\.py/.test(e.message) && !e.message.includes('sk-or-planted'));
    mkdirSync(join(run, 'project', 'app', 'pkg'));
    rmSync(join(run, 'project', 'app', 'leak.py'));
    symlinkSync(root, join(run, 'project', 'app', 'pkg', 'up'));
    assert.throws(() => runDiff(task, run), /pkg\/up/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
