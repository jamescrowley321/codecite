import { test } from 'node:test';
import assert from 'node:assert/strict';
import { emptyAgentResult, loadAdapter } from './agent.mjs';

test('emptyAgentResult is all unknown', () => {
  const r = emptyAgentResult();
  assert.equal(r.turns, null);
  assert.equal(r.providerStatus, null);
  assert.equal(r.rejectReason, null);
  assert.deepEqual(r.shellCommands, []);
  assert.deepEqual(r.readPaths, []);
  assert.equal(r.usage.totalTokens, null);
});

test('loadAdapter refuses a name that is not [a-z0-9-]+', async () => {
  await assert.rejects(loadAdapter('../lib/call'), /bad adapter name/);
});
