import { test } from 'node:test';
import assert from 'node:assert/strict';
import { estimateUsd, keyRemainingUsd, modelPrices } from './openrouter.mjs';

const plan = {
  runs: 2,
  seed: 1,
  agents: [
    { agent: 'claude', model: 'claude-sonnet-5-5' },
    { agent: 'pi', model: 'a/m' },
  ],
  arms: [{ name: 'baseline' }, { name: 'codecite', codecite: true }],
};

function fakeFetch(routes) {
  const calls = [];
  const fn = async (url, opts = {}) => {
    calls.push({ url, opts });
    const r = routes[url];
    if (!r) return { ok: false, status: 404, json: async () => ({}) };
    return { ok: r.status === undefined || r.status < 400, status: r.status ?? 200, json: async () => r.body };
  };
  fn.calls = calls;
  return fn;
}

test('estimate counts claude as 0 and prices the rest at 300k in, 20k out', () => {
  const prices = new Map([['a/m', { prompt: 1e-6, completion: 1e-5 }]]);
  // 4 pi runs x (300000 * 1e-6 + 20000 * 1e-5) = 4 x 0.5
  assert.equal(estimateUsd(plan, { prices }), 2);
});

test('estimate over a subset of runs', () => {
  const prices = new Map([['a/m', { prompt: 1e-6, completion: 1e-5 }]]);
  assert.equal(estimateUsd(plan, { prices, runs: [{ agent: 'pi', model: 'a/m' }] }), 0.5);
});

test('estimate refuses a model with no price', () => {
  assert.throws(() => estimateUsd(plan, { prices: new Map() }), /no price for a\/m/);
});

test('modelPrices reads the models list', async () => {
  const fetch = fakeFetch({
    'https://openrouter.ai/api/v1/models': { body: { data: [{ id: 'a/m', pricing: { prompt: '0.000001', completion: '0.00001' } }] } },
  });
  const prices = await modelPrices({ fetch });
  assert.deepEqual(prices.get('a/m'), { prompt: 1e-6, completion: 1e-5 });
});

test('keyRemainingUsd is limit minus usage, sent as a bearer token', async () => {
  const fetch = fakeFetch({ 'https://openrouter.ai/api/v1/key': { body: { data: { limit: 50, usage: 12.5 } } } });
  assert.equal(await keyRemainingUsd('sk-test', { fetch }), 37.5);
  assert.equal(fetch.calls[0].opts.headers.Authorization, 'Bearer sk-test');
});

test('keyRemainingUsd is null for a key with no limit', async () => {
  const fetch = fakeFetch({ 'https://openrouter.ai/api/v1/key': { body: { data: { limit: null, usage: 3 } } } });
  assert.equal(await keyRemainingUsd('sk-test', { fetch }), null);
});

test('keyRemainingUsd errors name the status, never the key', async () => {
  const fetch = fakeFetch({ 'https://openrouter.ai/api/v1/key': { status: 401, body: {} } });
  await assert.rejects(keyRemainingUsd('sk-secret-123', { fetch }), (e) => /401/.test(e.message) && !e.message.includes('sk-secret-123'));
});
