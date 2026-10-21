// The no-loss scorecard: per agent/model pair, the codecite arm against the
// baseline on correctness, tokens and time, and the round's verdict. Each
// measure is a difference between the arms: pass rate in points
// (codecite - baseline), and median total tokens and median wall time as a
// relative difference (codecite / baseline - 1). A 95% percentile bootstrap
// interval over the usable runs (each arm resampled on its own, seeded from
// the plan's seed) judges it: a loss only when even the interval's
// favourable bound is past the margin, i.e. the pass rate's upper bound is
// more than 5 points below 0, or the tokens' or time's lower bound more than
// 5% above. This replaces the spec's plain comparison of the numbers, by
// the owner's decision. Unknown (null) never passes a measure.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { loadRound } from './call.mjs';
import { armLabel } from './plan.mjs';
import { mulberry32 } from './rng.mjs';
import { SCORED } from './score.mjs';

export const MARGIN_PCT = 5;
export const MARGIN_PTS = 5;
/** Bootstrap resamples per measure. */
export const RESAMPLES = 10_000;
/** The interval's coverage. */
export const CONFIDENCE = 0.95;
// Float slack, so a bound of exactly 5% is not a loss.
const EPS = 1e-9;

/**
 * @typedef {object} RunRecord
 * @property {string} id
 * @property {string} status  ok|timeout|agent_error|infra_error|fatal
 * @property {string[]} tainted
 * @property {number} attempts
 * @property {boolean|null} passed  null when never scored, or unscored (a scoring fault)
 * @property {string|null} unscored  why scoring gave no result
 * @property {boolean} scoreFatal  unscored after every allowed attempt: a fatal scoring error
 * @property {number|null} totalTokens
 * @property {number|null} wallMs
 * @property {number|null} costUsd
 * @property {number|null} toolCalls
 * @property {number|null} codeciteCalls
 * @property {boolean} superseded  a run dir a resume kept aside (`<id>.tainted-N`, `<id>.unknown-N`)
 * @property {boolean} tokensUnknown  scored-status run whose total tokens are unknown: redone on resume
 * @property {boolean} secretInTranscript  a secret was redacted from this run's output
 */

const known = (xs) => xs.length > 0 && xs.every((x) => typeof x === 'number' && Number.isFinite(x));

/** Linear-interpolated quantile of sorted numbers. */
function quantile(sorted, q) {
  const pos = (sorted.length - 1) * q;
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  // Equal ends (Infinity included) need no interpolation.
  if (sorted[lo] === sorted[hi]) return sorted[lo];
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo);
}

const medianOf = (xs) => quantile([...xs].sort((a, b) => a - b), 0.5);

/** A seed for one pair's measure: FNV-1a over the parts, mixed with the plan's seed. */
export function measureSeed(planSeed, ...parts) {
  let h = 0x811c9dc5 ^ (Number(planSeed) >>> 0);
  for (const ch of parts.join('|')) {
    h ^= ch.codePointAt(0);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

/**
 * A percentile bootstrap interval for `stat(baseline, codecite)`: each arm
 * resampled with replacement on its own, `resamples` times, from a
 * mulberry32 stream seeded with `seed`. Returns [lower, upper] at
 * CONFIDENCE, or null when either arm is empty or holds an unknown.
 */
export function bootstrapCi(b, c, stat, { seed, resamples = RESAMPLES } = {}) {
  if (!known(b) || !known(c)) return null;
  const rand = mulberry32(seed ?? 0);
  const sb = new Array(b.length);
  const sc = new Array(c.length);
  const out = new Float64Array(resamples);
  for (let k = 0; k < resamples; k++) {
    for (let i = 0; i < b.length; i++) sb[i] = b[Math.floor(rand() * b.length)];
    for (let i = 0; i < c.length; i++) sc[i] = c[Math.floor(rand() * c.length)];
    out[k] = stat(sb, sc);
  }
  out.sort();
  const tail = (1 - CONFIDENCE) / 2;
  return [quantile(out, tail), quantile(out, 1 - tail)];
}

/** Codecite's median relative to baseline's, in percent; 0 vs 0 is 0, anything vs 0 is +Infinity. */
const relPct = (mb, mc) => (mb === 0 ? (mc === 0 ? 0 : Infinity) : ((mc - mb) * 100) / mb);
const rateOf = (xs) => (xs.reduce((a, x) => a + x, 0) * 100) / xs.length;

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
  return records.filter(
    (r) => SCORED.has(r.status) && typeof r.passed === 'boolean' && !(r.tainted?.length > 0) && !r.superseded && typeof r.totalTokens === 'number' && Number.isFinite(r.totalTokens),
  );
}

const passRate = (rs) => (rs.length === 0 ? null : (rs.filter((r) => r.passed).length * 100) / rs.length);

function pctMeasure(field, ub, uc, seed, resamples) {
  const xb = ub.map((r) => r[field]);
  const xc = uc.map((r) => r[field]);
  const b = median(xb);
  const c = median(xc);
  const deltaPct = b != null && c != null ? relPct(b, c) : null;
  const ci = bootstrapCi(xb, xc, (sb, sc) => relPct(medianOf(sb), medianOf(sc)), { seed, resamples });
  const loss = ci == null ? null : ci[0] > MARGIN_PCT + EPS;
  const passing = (rs) => median(rs.filter((r) => r.passed).map((r) => r[field]));
  return {
    baseline: b,
    codecite: c,
    deltaPct,
    ci,
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
 * @param {{seed?: number, resamples?: number}} [opts]  `seed` (default 0) seeds every
 *   measure's bootstrap (comparePlan derives it from the plan's seed and the pair).
 * @returns {{verdict: "PASS"|"STOP"|"INCOMPLETE", reasons: string[], runs: {baseline: number, codecite: number},
 *   correctness: object, tokens: object, time: object, cost: object, toolCalls: object, codeciteCallRate: object}}
 */
export function comparePair(baseline, codecite, plannedRuns, { seed = 0, resamples = RESAMPLES } = {}) {
  const ub = usableRuns(baseline);
  const uc = usableRuns(codecite);
  const rb = passRate(ub);
  const rc = passRate(uc);
  const deltaPts = rb == null || rc == null ? null : rc - rb;
  const passes = (rs) => rs.map((r) => (r.passed ? 1 : 0));
  const cci = bootstrapCi(passes(ub), passes(uc), (sb, sc) => rateOf(sc) - rateOf(sb), { seed: measureSeed(seed, 'correctness'), resamples });
  const correctness = { baseline: rb, codecite: rc, deltaPts, ci: cci, loss: cci == null ? null : cci[1] < -MARGIN_PTS - EPS };
  const tokens = pctMeasure('totalTokens', ub, uc, measureSeed(seed, 'tokens'), resamples);
  const time = pctMeasure('wallMs', ub, uc, measureSeed(seed, 'time'), resamples);

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
    testsRate: { baseline: rateWhere(ub, (r) => r.testsPassed), codecite: rateWhere(uc, (r) => r.testsPassed) },
    checkRates: checkRates(ub, uc),
    judge: judgeMeasures(ub, uc),
  };
}

// Reported, never part of the verdict: hidden tests alone, each
// deterministic check, and the judge's median scores.
const rateWhere = (rs, f) => (rs.length === 0 ? null : (rs.filter(f).length * 100) / rs.length);

function checkRates(ub, uc) {
  const names = [...new Set([...ub, ...uc].flatMap((r) => (r.checks ?? []).map((c) => c.name)))];
  const rate = (rs, n) => {
    const known = rs.filter((r) => r.checks);
    return known.length === 0 ? null : (known.filter((r) => r.checks.find((c) => c.name === n)?.passed).length * 100) / known.length;
  };
  return names.map((name) => ({ name, baseline: rate(ub, name), codecite: rate(uc, name) }));
}

export const JUDGE_CRITERIA = ['instructions', 'library', 'hygiene'];

function judgeMeasures(ub, uc) {
  const of = (rs, c) => rs.map((r) => r.judge?.[c]).filter((x) => typeof x === 'number');
  return JUDGE_CRITERIA.map((c) => {
    const b = of(ub, c);
    const k = of(uc, c);
    return { name: c, baseline: median(b), codecite: median(k), meanBaseline: b.length ? b.reduce((x, y) => x + y, 0) / b.length : null, meanCodecite: k.length ? k.reduce((x, y) => x + y, 0) / k.length : null, n: [b.length, k.length] };
  });
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
    const judge = readJson(join(runsDir, id, 'judge.json'));
    const run = result.run ?? {};
    const totalTokens = num(result.usage?.totalTokens);
    records.push({
      // The dir name: a kept-aside run (`<id>.tainted-1`) shares its run's id.
      id,
      superseded: run.id != null && id !== run.id,
      tokensUnknown: SCORED.has(result.status) && totalTokens == null,
      secretInTranscript: result.secretInTranscript === true,
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
      testsPassed: typeof score?.testsPassed === 'boolean' ? score.testsPassed : typeof score?.passed === 'boolean' ? score.passed : null,
      checks: Array.isArray(score?.checks) ? score.checks : null,
      judge: judge && !judge.error ? Object.fromEntries(JUDGE_CRITERIA.map((c) => [c, num(judge[c]?.score)])) : null,
      failures: Array.isArray(score?.failures) ? score.failures : [],
      unscored: typeof score?.unscored === 'string' ? score.unscored : null,
      scoreFatal: score?.passed === null && score?.fatal === true,
      scoreAttempts: num(score?.scoreAttempts),
      totalTokens,
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
      const seed = measureSeed(plan.seed ?? 0, agent, model, arm.name);
      pairs.push({ agent, model, arm: arm.name, ...comparePair(of('baseline'), of(arm.name), plan.runs, { seed }) });
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
const judgeCell = (m, mean) => (m == null ? UNKNOWN : `${m} / ${mean.toFixed(2)}`);
const pct = (n) => (n == null ? UNKNOWN : `${n.toFixed(1)}%`);
const signed = (n, unit) => (n == null ? UNKNOWN : `${n > 0 ? '+' : ''}${n.toFixed(1)}${unit}`);
const lossText = (l) => (l === true ? '**loss**' : l === false ? 'no' : `${UNKNOWN} (can't pass)`);
const range = (r, f) => (r == null ? UNKNOWN : `${f(r[0])} – ${f(r[1])}`);
const bound = (n, unit) => (n === Infinity ? '+inf' : n === -Infinity ? '-inf' : signed(n, unit));
/** A point estimate with its interval: `+6.0% (95% CI -3.1% to +15.2%)`. */
const withCi = (n, ci, unit) => (n == null ? UNKNOWN : `${bound(n, unit)} (95% CI ${ci == null ? UNKNOWN : `${bound(ci[0], unit)} to ${bound(ci[1], unit)}`})`);

/** A fenced block whose fence is longer than any backtick run in `text`. */
function fenced(text, lang = '') {
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  const fence = '`'.repeat(longest + 1);
  return `${fence}${lang}\n${text.replace(/\n$/, '')}\n${fence}`;
}

function pairSection(p) {
  const rows = [
    ['Usable runs', p.runs.baseline, p.runs.codecite, '', ''],
    ['Pass rate (tests and checks)', pct(p.correctness.baseline), pct(p.correctness.codecite), withCi(p.correctness.deltaPts, p.correctness.ci, ' pts'), lossText(p.correctness.loss)],
    ['Median total tokens', int(p.tokens.baseline), int(p.tokens.codecite), withCi(p.tokens.deltaPct, p.tokens.ci, '%'), lossText(p.tokens.loss)],
    ['Total tokens IQR', range(p.tokens.iqrBaseline, int), range(p.tokens.iqrCodecite, int), '', ''],
    ['Median total tokens, passing runs', int(p.tokens.passingBaseline), int(p.tokens.passingCodecite), '', ''],
    ['Median wall time', secs(p.time.baseline), secs(p.time.codecite), withCi(p.time.deltaPct, p.time.ci, '%'), lossText(p.time.loss)],
    ['Wall time IQR', range(p.time.iqrBaseline, secs), range(p.time.iqrCodecite, secs), '', ''],
    ['Median wall time, passing runs', secs(p.time.passingBaseline), secs(p.time.passingCodecite), '', ''],
    ['Median cost per run', usd(p.cost.baseline), usd(p.cost.codecite), '', ''],
    ['Total cost', usd(p.cost.totalBaseline), usd(p.cost.totalCodecite), '', ''],
    ['Median tool calls', int(p.toolCalls.baseline), int(p.toolCalls.codecite), '', ''],
    ['Runs that called codecite', pct(p.codeciteCallRate.baseline), pct(p.codeciteCallRate.codecite), '', ''],
    ...(p.checkRates?.length ? [['Hidden tests pass', pct(p.testsRate.baseline), pct(p.testsRate.codecite), '', '']] : []),
    ...(p.checkRates ?? []).map((c) => [`Check ${c.name}`, pct(c.baseline), pct(c.codecite), '', '']),
    ...(p.judge ?? []).filter((j) => j.n[0] + j.n[1] > 0).map((j) => [`Judge ${j.name} (1-5, median / mean)`, judgeCell(j.baseline, j.meanBaseline), judgeCell(j.codecite, j.meanCodecite), '', '']),
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
  const scoreErrors = records.filter((r) => r.scoreFatal);
  if (scoreErrors.length > 0) {
    out.push(
      `**Scoring errors:** ${scoreErrors.length} run${scoreErrors.length === 1 ? '' : 's'} could not be scored (${scoreErrors.map((r) => `\`${r.id}\``).join(', ')}); see Excluded runs. Fix the cause and run \`score --rescore\`.`,
      '',
    );
  }
  out.push(
    `The rule: per agent/model pair, the codecite arm must not lose to the baseline. Each measure is a difference (codecite - baseline): pass rate in points, median total tokens and median wall time in percent of baseline's median. Each carries a ${Math.round(CONFIDENCE * 100)}% percentile bootstrap interval (${RESAMPLES.toLocaleString('en-US')} resamples of each arm's usable runs, seeded from the plan's seed). A loss is an interval that is past the margin even at its favourable end: a pass rate whose upper bound is more than ${MARGIN_PTS} points below baseline's, or tokens or time whose lower bound is more than ${MARGIN_PCT}% above. This replaces a plain comparison of the numbers, which stops on noise. Any loss is STOP. A pair with fewer than ${plan.runs} usable runs in an arm, or an unknown measure, is INCOMPLETE. Usable runs are scored runs with status ok, timeout or agent_error, with known tokens, that are not tainted; medians cover all of them, failed runs included. A tainted run or one with unknown tokens is redone when \`call\` resumes, and the old one stays listed below.`,
    '',
  );
  const dirty = plan.codeciteDirty ? ' (dirty tree)' : '';
  out.push(
    `Task \`${plan.task ?? plan.taskDef?.id}\`; planned runs per agent/model/arm: ${plan.runs}; codecite \`${plan.codeciteCommit ?? UNKNOWN}\`${dirty}; identity-model \`${plan.identityModelCommit ?? UNKNOWN}\`; frozen ${plan.frozenAt ?? UNKNOWN}.`,
    '',
  );
  if ((plan.agents ?? []).some((a) => a.agent === 'pi')) {
    const labels = plan.arms.map((a) => `\`${a.name}\` = \`${armLabel(plan.roundId, a.name)}\``).join(', ');
    out.push(`OpenRouter attribution (pi's X-Title, opaque so the agent can't read its arm from it): ${labels}.`, '');
  }
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
    const fatalNote = r.scoreFatal ? `, a fatal scoring error after ${r.scoreAttempts ?? '?'} attempts` : '';
    const redo = r.superseded ? 'redone' : 'redone when call resumes';
    const why = !SCORED.has(r.status)
      ? r.status
      : r.tokensUnknown
        ? `token usage unknown (${r.status}; ${redo})`
        : r.unscored
          ? `unscored (${cell(r.unscored)})${fatalNote}`
          : 'not scored';
    out.push(`- \`${r.id}\`: ${why}${r.error ? `: ${cell(r.error)}` : ''} (attempts ${r.attempts})`);
  }
  out.push('');

  const retried = records.filter((r) => r.attempts > 1);
  out.push('## Retried runs', '');
  if (retried.length === 0) out.push('None.');
  for (const r of retried) out.push(`- \`${r.id}\`: ${r.attempts} attempts, ended ${r.status}`);
  out.push('');

  const tainted = records.filter((r) => r.tainted.length > 0);
  out.push('## Tainted runs (left out of the verdict, redone when call resumes)', '');
  if (tainted.length === 0) out.push('None.');
  for (const r of tainted) out.push(`- \`${r.id}\`${r.superseded ? ' (kept aside; redone)' : ''}: ${r.tainted.map((h) => `\`${cell(typeof h === 'string' ? h : JSON.stringify(h))}\``).join(', ')}`);
  out.push('');

  const leaked = records.filter((r) => r.secretInTranscript);
  out.push('## Secrets in transcripts', '');
  if (leaked.length === 0) out.push('None.');
  else {
    out.push(
      `**${leaked.length} run${leaked.length === 1 ? '' : 's'} printed a secret** (redacted on disk, but the agent's output went to its model provider): rotate the OpenRouter key and log in to Claude Code again.`,
      '',
    );
    for (const r of leaked) out.push(`- \`${r.id}\``);
  }
  out.push('');

  out.push('## Codecite note', '');
  const noted = plan.arms.filter((a) => a.note);
  if (noted.length === 0) out.push('None.');
  for (const a of noted) out.push(`Arm \`${a.name}\`:`, '', fenced(a.note), '');
  out.push('');

  out.push('## Frozen plan', '', '<details><summary>plan.json</summary>', '', fenced(JSON.stringify(plan, null, 2), 'json'), '', '</details>', '');
  return out.join('\n');
}
