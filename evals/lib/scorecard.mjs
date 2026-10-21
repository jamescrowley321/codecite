// The no-loss scorecard: per agent/model pair, the codecite arm against the
// baseline on correctness, tokens and time, and the round's verdict. A loss
// is codecite's pass rate more than 5 points below baseline's, or its median
// total tokens or median wall time more than 5% above. Unknown (null) never
// passes a measure.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { loadRound } from './call.mjs';
import { SCORED } from './score.mjs';

export const MARGIN_PCT = 5;
export const MARGIN_PTS = 5;
// Float slack, so exactly 5% is not a loss.
const EPS = 1e-9;

/**
 * @typedef {object} RunRecord
 * @property {string} id
 * @property {string} status  ok|timeout|agent_error|infra_error|fatal
 * @property {string[]} tainted
 * @property {number} attempts
 * @property {boolean|null} passed  null when never scored, or unscored (a scoring fault)
 * @property {string|null} unscored  why scoring gave no result
 * @property {number|null} totalTokens
 * @property {number|null} wallMs
 * @property {number|null} costUsd
 * @property {number|null} toolCalls
 * @property {number|null} codeciteCalls
 */

const known = (xs) => xs.length > 0 && xs.every((x) => typeof x === 'number' && Number.isFinite(x));

/** Linear-interpolated quantile of sorted numbers. */
function quantile(sorted, q) {
  const pos = (sorted.length - 1) * q;
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo);
}

/** The median, or null when `xs` is empty or holds an unknown. */
export function median(xs) {
  if (!known(xs)) return null;
  return quantile([...xs].sort((a, b) => a - b), 0.5);
}

/** [q1, q3], or null when `xs` is empty or holds an unknown. */
export function iqr(xs) {
  if (!known(xs)) return null;
  const s = [...xs].sort((a, b) => a - b);
  return [quantile(s, 0.25), quantile(s, 0.75)];
}

const sum = (xs) => (known(xs) ? xs.reduce((a, b) => a + b, 0) : null);

/**
 * The runs that count: scored, with status ok, timeout or agent_error, and
 * not tainted. infra_error and fatal never count.
 */
export function usableRuns(records) {
  return records.filter((r) => SCORED.has(r.status) && typeof r.passed === 'boolean' && !(r.tainted?.length > 0));
}

const passRate = (rs) => (rs.length === 0 ? null : (rs.filter((r) => r.passed).length * 100) / rs.length);

function pctMeasure(field, ub, uc) {
  const b = median(ub.map((r) => r[field]));
  const c = median(uc.map((r) => r[field]));
  let deltaPct = null;
  let loss = null;
  if (b != null && c != null) {
    if (b === 0) {
      deltaPct = c === 0 ? 0 : null;
      loss = c > 0;
    } else {
      deltaPct = ((c - b) * 100) / b;
      loss = deltaPct > MARGIN_PCT + EPS;
    }
  }
  const passing = (rs) => median(rs.filter((r) => r.passed).map((r) => r[field]));
  return {
    baseline: b,
    codecite: c,
    deltaPct,
    loss,
    iqrBaseline: iqr(ub.map((r) => r[field])),
    iqrCodecite: iqr(uc.map((r) => r[field])),
    passingBaseline: passing(ub),
    passingCodecite: passing(uc),
  };
}

function callRate(rs) {
  const calls = rs.map((r) => r.codeciteCalls);
  if (!known(calls)) return null;
  return (calls.filter((n) => n > 0).length * 100) / calls.length;
}

/**
 * Compare one pair's codecite arm to its baseline.
 *
 * @param {RunRecord[]} baseline
 * @param {RunRecord[]} codecite
 * @param {number} plannedRuns
 * @returns {{verdict: "PASS"|"STOP"|"INCOMPLETE", reasons: string[], runs: {baseline: number, codecite: number},
 *   correctness: object, tokens: object, time: object, cost: object, toolCalls: object, codeciteCallRate: object}}
 */
export function comparePair(baseline, codecite, plannedRuns) {
  const ub = usableRuns(baseline);
  const uc = usableRuns(codecite);
  const rb = passRate(ub);
  const rc = passRate(uc);
  const deltaPts = rb == null || rc == null ? null : rc - rb;
  const correctness = { baseline: rb, codecite: rc, deltaPts, loss: deltaPts == null ? null : deltaPts < -MARGIN_PTS - EPS };
  const tokens = pctMeasure('totalTokens', ub, uc);
  const time = pctMeasure('wallMs', ub, uc);

  const reasons = [];
  let verdict;
  const short = [
    ['baseline', ub.length],
    ['codecite', uc.length],
  ].filter(([, n]) => n < plannedRuns);
  const measures = { correctness, tokens, time };
  const losses = Object.entries(measures).filter(([, m]) => m.loss === true);
  const unknown = Object.entries(measures).filter(([, m]) => m.loss === null);
  if (short.length > 0) {
    verdict = 'INCOMPLETE';
    for (const [arm, n] of short) reasons.push(`${arm} has ${n} of ${plannedRuns} usable runs`);
  } else if (losses.length > 0) {
    verdict = 'STOP';
  } else if (unknown.length > 0) {
    verdict = 'INCOMPLETE';
  } else {
    verdict = 'PASS';
  }
  for (const [name] of losses) reasons.push(`loss on ${name}`);
  for (const [name] of unknown) reasons.push(`${name} unknown, so it can't pass`);

  return {
    verdict,
    reasons,
    runs: { baseline: ub.length, codecite: uc.length },
    correctness,
    tokens,
    time,
    cost: {
      baseline: median(ub.map((r) => r.costUsd)),
      codecite: median(uc.map((r) => r.costUsd)),
      totalBaseline: sum(ub.map((r) => r.costUsd)),
      totalCodecite: sum(uc.map((r) => r.costUsd)),
    },
    toolCalls: { baseline: median(ub.map((r) => r.toolCalls)), codecite: median(uc.map((r) => r.toolCalls)) },
    codeciteCallRate: { baseline: callRate(ub), codecite: callRate(uc) },
  };
}

/** STOP if any pair STOPs, INCOMPLETE if any is incomplete (or there are none), else PASS. */
export function roundVerdict(pairs) {
  if (pairs.length === 0) return 'INCOMPLETE';
  if (pairs.some((p) => p.verdict === 'STOP')) return 'STOP';
  if (pairs.some((p) => p.verdict !== 'PASS')) return 'INCOMPLETE';
  return 'PASS';
}

const readJson = (p) => {
  try {
    return JSON.parse(readFileSync(p, 'utf8'));
  } catch {
    return null;
  }
};
const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);

/** Every run of a round that has a result.json, joined with its score.json. */
export function loadRecords(roundDir) {
  const runsDir = join(roundDir, 'runs');
  if (!existsSync(runsDir)) return [];
  const records = [];
  for (const id of readdirSync(runsDir).sort()) {
    const result = readJson(join(runsDir, id, 'result.json'));
    if (!result) continue;
    const score = readJson(join(runsDir, id, 'score.json'));
    const run = result.run ?? {};
    records.push({
      id: run.id ?? id,
      agent: run.agent ?? null,
      model: run.model ?? null,
      arm: run.arm ?? null,
      rep: run.rep ?? null,
      seq: run.seq ?? null,
      status: result.status ?? null,
      error: result.error ?? null,
      attempts: num(result.attempts) ?? 1,
      tainted: Array.isArray(result.tainted) ? result.tainted : [],
      passed: typeof score?.passed === 'boolean' ? score.passed : null,
      failures: Array.isArray(score?.failures) ? score.failures : [],
      unscored: typeof score?.unscored === 'string' ? score.unscored : null,
      totalTokens: num(result.usage?.totalTokens),
      costUsd: num(result.usage?.costUsd),
      wallMs: num(result.wallMs),
      toolCalls: num(result.toolCalls),
      codeciteCalls: num(result.codeciteCalls),
    });
  }
  return records;
}

/** Every pair of the plan: each agent/model, each non-baseline arm against the baseline. */
export function comparePlan(plan, records) {
  const pairs = [];
  for (const { agent, model } of plan.agents) {
    const of = (arm) => records.filter((r) => r.agent === agent && r.model === model && r.arm === arm);
    for (const arm of plan.arms.filter((a) => a.name !== 'baseline')) {
      pairs.push({ agent, model, arm: arm.name, ...comparePair(of('baseline'), of(arm.name), plan.runs) });
    }
  }
  return pairs;
}

// --- Markdown ---

const UNKNOWN = 'unknown';
const cell = (s) => String(s).replaceAll('|', '\\|').replaceAll('\n', ' ');
const int = (n) => (n == null ? UNKNOWN : Math.round(n).toLocaleString('en-US'));
const secs = (ms) => (ms == null ? UNKNOWN : `${(ms / 1000).toFixed(1)} s`);
const usd = (n) => (n == null ? UNKNOWN : `$${n.toFixed(Math.abs(n) >= 1 ? 2 : 4)}`);
const pct = (n) => (n == null ? UNKNOWN : `${n.toFixed(1)}%`);
const signed = (n, unit) => (n == null ? UNKNOWN : `${n > 0 ? '+' : ''}${n.toFixed(1)}${unit}`);
const lossText = (l) => (l === true ? '**loss**' : l === false ? 'no' : `${UNKNOWN} (can't pass)`);
const range = (r, f) => (r == null ? UNKNOWN : `${f(r[0])} – ${f(r[1])}`);

/** A fenced block whose fence is longer than any backtick run in `text`. */
function fenced(text, lang = '') {
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  const fence = '`'.repeat(longest + 1);
  return `${fence}${lang}\n${text.replace(/\n$/, '')}\n${fence}`;
}

function pairSection(p) {
  const rows = [
    ['Usable runs', p.runs.baseline, p.runs.codecite, '', ''],
    ['Pass rate', pct(p.correctness.baseline), pct(p.correctness.codecite), signed(p.correctness.deltaPts, ' pts'), lossText(p.correctness.loss)],
    ['Median total tokens', int(p.tokens.baseline), int(p.tokens.codecite), signed(p.tokens.deltaPct, '%'), lossText(p.tokens.loss)],
    ['Total tokens IQR', range(p.tokens.iqrBaseline, int), range(p.tokens.iqrCodecite, int), '', ''],
    ['Median total tokens, passing runs', int(p.tokens.passingBaseline), int(p.tokens.passingCodecite), '', ''],
    ['Median wall time', secs(p.time.baseline), secs(p.time.codecite), signed(p.time.deltaPct, '%'), lossText(p.time.loss)],
    ['Wall time IQR', range(p.time.iqrBaseline, secs), range(p.time.iqrCodecite, secs), '', ''],
    ['Median wall time, passing runs', secs(p.time.passingBaseline), secs(p.time.passingCodecite), '', ''],
    ['Median cost per run', usd(p.cost.baseline), usd(p.cost.codecite), '', ''],
    ['Total cost', usd(p.cost.totalBaseline), usd(p.cost.totalCodecite), '', ''],
    ['Median tool calls', int(p.toolCalls.baseline), int(p.toolCalls.codecite), '', ''],
    ['Runs that called codecite', pct(p.codeciteCallRate.baseline), pct(p.codeciteCallRate.codecite), '', ''],
  ];
  const lines = [
    `## ${p.agent} ${p.model}: ${p.arm} vs baseline — **${p.verdict}**`,
    '',
    ...(p.reasons.length ? [...p.reasons.map((r) => `- ${r}`), ''] : []),
    `| Measure | baseline | ${cell(p.arm)} | Δ | Loss |`,
    '| --- | --- | --- | --- | --- |',
    ...rows.map((r) => `| ${r.map(cell).join(' | ')} |`),
  ];
  return lines.join('\n');
}

/**
 * The round's scorecard as Markdown. `round` is a round dir, or
 * {plan, records, pairs?} (records from loadRecords, pairs from comparePlan).
 */
export function renderScorecard(round) {
  const { plan, records, pairs: given } = typeof round === 'string' ? { plan: loadRound(round).plan, records: loadRecords(round) } : round;
  const pairs = given ?? comparePlan(plan, records);
  const verdict = roundVerdict(pairs);
  const out = [];
  out.push(`# Scorecard: ${plan.roundId ?? 'round'}`, '');
  out.push(`Verdict: **${verdict}**`, '');
  out.push(
    `The rule: per agent/model pair, the codecite arm must not lose to the baseline. A loss is a pass rate more than ${MARGIN_PTS} points lower, or median total tokens or median wall time more than ${MARGIN_PCT}% higher. Any loss is STOP. A pair with fewer than ${plan.runs} usable runs in an arm, or an unknown measure, is INCOMPLETE. Usable runs are scored runs with status ok, timeout or agent_error that are not tainted; medians cover all of them, failed runs included.`,
    '',
  );
  const dirty = plan.codeciteDirty ? ' (dirty tree)' : '';
  out.push(
    `Task \`${plan.task ?? plan.taskDef?.id}\`; planned runs per agent/model/arm: ${plan.runs}; codecite \`${plan.codeciteCommit ?? UNKNOWN}\`${dirty}; identity-model \`${plan.identityModelCommit ?? UNKNOWN}\`; frozen ${plan.frozenAt ?? UNKNOWN}.`,
    '',
  );
  for (const p of pairs) out.push(pairSection(p), '');

  out.push('## Runs', '');
  out.push('| Run | Status | Scored | Tokens | Wall time | Cost | Tool calls | codecite calls | Attempts | Tainted |');
  out.push('| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |');
  const bySeq = [...records].sort((a, b) => (a.seq ?? Infinity) - (b.seq ?? Infinity) || a.id.localeCompare(b.id));
  for (const r of bySeq) {
    const scored = r.passed == null ? (r.unscored ? 'unscored' : 'not scored') : r.passed ? 'passed' : `failed (${r.failures.length})`;
    out.push(
      `| ${[`\`${r.id}\``, r.status, scored, int(r.totalTokens), secs(r.wallMs), usd(r.costUsd), int(r.toolCalls), int(r.codeciteCalls), r.attempts, r.tainted.length ? 'yes' : ''].map(cell).join(' | ')} |`,
    );
  }
  out.push('');

  const usable = new Set(usableRuns(records).map((r) => r.id));
  const excluded = records.filter((r) => !usable.has(r.id) && !r.tainted.length);
  out.push('## Excluded runs', '');
  if (excluded.length === 0) out.push('None.');
  for (const r of excluded) {
    const why = !SCORED.has(r.status) ? r.status : r.unscored ? `unscored (${cell(r.unscored)})` : 'not scored';
    out.push(`- \`${r.id}\`: ${why}${r.error ? `: ${cell(r.error)}` : ''} (attempts ${r.attempts})`);
  }
  out.push('');

  const retried = records.filter((r) => r.attempts > 1);
  out.push('## Retried runs', '');
  if (retried.length === 0) out.push('None.');
  for (const r of retried) out.push(`- \`${r.id}\`: ${r.attempts} attempts, ended ${r.status}`);
  out.push('');

  const tainted = records.filter((r) => r.tainted.length > 0);
  out.push('## Tainted runs (left out of the verdict)', '');
  if (tainted.length === 0) out.push('None.');
  for (const r of tainted) out.push(`- \`${r.id}\`: ${r.tainted.map((h) => `\`${cell(typeof h === 'string' ? h : JSON.stringify(h))}\``).join(', ')}`);
  out.push('');

  out.push('## Codecite note', '');
  const noted = plan.arms.filter((a) => a.note);
  if (noted.length === 0) out.push('None.');
  for (const a of noted) out.push(`Arm \`${a.name}\`:`, '', fenced(a.note), '');
  out.push('');

  out.push('## Frozen plan', '', '<details><summary>plan.json</summary>', '', fenced(JSON.stringify(plan, null, 2), 'json'), '', '</details>', '');
  return out.join('\n');
}
