import { test } from 'node:test';
import assert from 'node:assert/strict';
import { emptyUsage, addUsage } from './telemetry.mjs';

const nulls = { input: null, output: null, cacheRead: null, cacheWrite: null, totalTokens: null, costUsd: null };

test('emptyUsage is all unknown', () => {
  assert.deepEqual({ ...emptyUsage() }, nulls);
});

test('unknown stays unknown', () => {
  const u = addUsage(emptyUsage(), { ...nulls, input: 5 });
  assert.equal(u.input, 5);
  assert.equal(u.output, null);
});

test('null + n is null once a value is unknown', () => {
  const first = addUsage(emptyUsage(), { ...nulls, input: 5, output: 1 });
  const second = addUsage(first, { ...nulls, input: 3 });
  assert.equal(second.input, 8);
  assert.equal(second.output, null);
  assert.equal(addUsage(second, { ...nulls, output: 4 }).output, null);
});

test('null + null from emptyUsage stays null', () => {
  assert.deepEqual({ ...addUsage(emptyUsage(), emptyUsage()) }, nulls);
  assert.deepEqual({ ...addUsage(addUsage(emptyUsage(), nulls), nulls) }, nulls);
});
