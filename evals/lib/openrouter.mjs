// OpenRouter spend checks: the key's remaining limit and a pre-round cost
// estimate. `fetch` is injectable; tests never touch the network. Errors
// name the endpoint and status, never the key.

import { expandRuns } from './plan.mjs';

const API = 'https://openrouter.ai/api/v1';

// A per-run token guess, priced at the model's live OpenRouter rate.
export const GUESS = { input: 300_000, output: 20_000 };

async function getJson(path, { fetch = globalThis.fetch, key } = {}) {
  const headers = key ? { Authorization: `Bearer ${key}` } : {};
  const res = await fetch(`${API}${path}`, { headers });
  if (!res.ok) throw new Error(`openrouter: GET ${API}${path} returned ${res.status}`);
  return res.json();
}

/** The key's `limit - usage` in USD, or null when the key has no limit. */
export async function keyRemainingUsd(key, { fetch } = {}) {
  const { data } = await getJson('/key', { fetch, key });
  if (data?.limit == null) return null;
  return data.limit - (data.usage ?? 0);
}

/** Live prices: Map of model id -> {prompt, completion} in USD per token. */
export async function modelPrices({ fetch } = {}) {
  const { data } = await getJson('/models', { fetch });
  return new Map(
    (data ?? []).map((m) => [m.id, { prompt: Number(m.pricing?.prompt), completion: Number(m.pricing?.completion) }]),
  );
}

/**
 * Estimated OpenRouter spend for `runs` (default: all of the plan's): each
 * non-Claude run costs GUESS tokens at its model's price; Claude runs go
 * through a subscription and count as 0. Throws for a model with no price.
 * @param {object} plan
 * @param {{prices: Map<string, {prompt: number, completion: number}>, runs?: {agent: string, model: string}[]}} opts
 */
export function estimateUsd(plan, { prices, runs = expandRuns(plan) }) {
  let usd = 0;
  for (const r of runs) {
    if (r.agent === 'claude') continue;
    const p = prices.get(r.model);
    if (!p || !Number.isFinite(p.prompt) || !Number.isFinite(p.completion)) {
      throw new Error(`openrouter: no price for ${r.model}`);
    }
    usd += GUESS.input * p.prompt + GUESS.output * p.completion;
  }
  return Math.round(usd * 1e6) / 1e6;
}
