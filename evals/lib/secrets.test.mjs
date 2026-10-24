import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, chmodSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { readSecrets, requireSecret } from './secrets.mjs';

function envFile(text, mode = 0o600) {
  const path = join(mkdtempSync(join(tmpdir(), 'secrets-')), 'evals.env');
  writeFileSync(path, text);
  chmodSync(path, mode);
  return path;
}

test('reads both secrets', () => {
  const path = envFile('# keys\nOPENROUTER_EVAL_API_KEY=sk-or-123\nexport CLAUDE_CODE_OAUTH_TOKEN="tok-456"\n');
  assert.deepEqual(readSecrets({ path }), { openrouter: 'sk-or-123', claudeOauth: 'tok-456' });
});

test('a missing variable is null', () => {
  const path = envFile('OPENROUTER_EVAL_API_KEY=sk-or-123\n');
  assert.deepEqual(readSecrets({ path }), { openrouter: 'sk-or-123', claudeOauth: null });
});

test('refuses a world-readable env file', () => {
  const path = envFile('OPENROUTER_EVAL_API_KEY=sk-or-123\n', 0o644);
  assert.throws(() => readSecrets({ path }), (e) => e.message.includes(path) && !e.message.includes('sk-or-123'));
  const group = envFile('OPENROUTER_EVAL_API_KEY=sk-or-123\n', 0o640);
  assert.throws(() => readSecrets({ path: group }), /readable/);
});

test('a missing file throws naming the path', () => {
  const path = join(mkdtempSync(join(tmpdir(), 'secrets-')), 'nope.env');
  assert.throws(() => readSecrets({ path }), (e) => e.message.includes(path));
});

test('error names the variable, not the value', () => {
  const path = envFile('OPENROUTER_EVAL_API_KEY=sk-or-SECRET\nthis line sk-or-SECRET is malformed\n');
  assert.throws(() => readSecrets({ path }), (e) => /line 2/.test(e.message) && !e.message.includes('SECRET'));
  assert.throws(
    () => requireSecret({ openrouter: null, claudeOauth: 'tok-SECRET' }, 'openrouter'),
    (e) => e.message.includes('OPENROUTER_EVAL_API_KEY') && !e.message.includes('SECRET'),
  );
  assert.equal(requireSecret({ openrouter: 'k', claudeOauth: null }, 'openrouter'), 'k');
});
