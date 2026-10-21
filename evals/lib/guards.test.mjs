import { test } from 'node:test';
import assert from 'node:assert/strict';
import { classify, CircuitBreaker } from './guards.mjs';

const base = { providerStatus: null, turns: 3, rejectReason: null, spawnError: null };

test('classifies a 404 as fatal', () => {
  for (const s of [400, 401, 402, 404]) assert.equal(classify({ ...base, providerStatus: s }), 'fatal', String(s));
});

test('a rejectReason is fatal', () => {
  assert.equal(classify({ ...base, rejectReason: 'user config leaked into the run' }), 'fatal');
});

test('5xx before any turn is infra', () => {
  assert.equal(classify({ ...base, providerStatus: 503, turns: 0 }), 'infra');
  assert.equal(classify({ ...base, providerStatus: 500, turns: 0 }), 'infra');
  assert.equal(classify({ ...base, providerStatus: 503, turns: 2 }), 'agent');
  assert.equal(classify({ ...base, providerStatus: 503, turns: null }), 'agent');
});

test('a spawn failure is infra', () => {
  assert.equal(classify({ ...base, turns: null, spawnError: 'ENOENT' }), 'infra');
});

test('everything else is agent', () => {
  assert.equal(classify(base), 'agent');
  assert.equal(classify({ ...base, providerStatus: 429 }), 'agent');
  assert.equal(classify({ ...base, providerStatus: 200 }), 'agent');
});

test('breaker trips at 8 of 10', () => {
  const b = new CircuitBreaker({ min: 10, ratio: 0.8 });
  for (let i = 0; i < 2; i++) b.record(true);
  for (let i = 0; i < 7; i++) b.record(false);
  assert.equal(b.tripped, false);
  b.record(false);
  assert.equal(b.tripped, true);
});

test('breaker ignores the first 9', () => {
  const b = new CircuitBreaker({ min: 10, ratio: 0.8 });
  for (let i = 0; i < 9; i++) {
    b.record(false);
    assert.equal(b.tripped, false, `after ${i + 1}`);
  }
  b.record(false);
  assert.equal(b.tripped, true);
});

test('breaker stays closed at 7 of 10', () => {
  const b = new CircuitBreaker({ min: 10, ratio: 0.8 });
  for (let i = 0; i < 3; i++) b.record(true);
  for (let i = 0; i < 7; i++) b.record(false);
  assert.equal(b.tripped, false);
});
