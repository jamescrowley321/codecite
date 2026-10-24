import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { bootstrapCi, comparePair, comparePlan, iqr, loadRecords, median, renderScorecard, roundVerdict, usableRuns } from './scorecard.mjs';
import { mulberry32 } from './rng.mjs';

/** n synthetic usable runs: every one passing, 100k tokens, 60 s. */
function runs(n, over = {}) {
  return Array.from({ length: n }, (_, i) => ({
    id: `r${i}`,
    status: 'ok',
    tainted: [],
    attempts: 1,
    passed: true,
    totalTokens: 100_000,
    wallMs: 60_000,
    costUsd: 0.5,
    toolCalls: 10,
    codeciteCalls: 0,
    ...(typeof over === 'function' ? over(i) : over),
  }));
}

test('median and iqr', () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
  assert.equal(median([]), null);
  assert.equal(median([1, null, 3]), null, 'unknown never becomes known');
  assert.deepEqual(iqr([1, 2, 3, 4, 5]), [2, 4]);
  assert.equal(iqr([1, null]), null);
});

test('pass when equal', () => {
  const r = comparePair(runs(20), runs(20), 20);
  assert.equal(r.verdict, 'PASS');
  assert.equal(r.correctness.loss, false);
  assert.equal(r.correctness.deltaPts, 0);
  assert.deepEqual(r.correctness.ci, [0, 0]);
  assert.equal(r.tokens.loss, false);
  assert.equal(r.tokens.deltaPct, 0);
  assert.deepEqual(r.tokens.ci, [0, 0]);
  assert.equal(r.time.loss, false);
});

/** n values around `mid`, spread +-`spread` (a fraction), from a fixed seed. */
function noisy(n, mid, spread, seed = 11) {
  const rand = mulberry32(seed);
  return Array.from({ length: n }, () => Math.round(mid * (1 + spread * (2 * rand() - 1))));
}

test('identical noisy samples PASS', () => {
  const tokens = noisy(20, 100_000, 0.5);
  const times = noisy(20, 60_000, 0.5, 12);
  const arm = () => runs(20, (i) => ({ totalTokens: tokens[i], wallMs: times[i], passed: i % 3 !== 0 }));
  const r = comparePair(arm(), arm(), 20, { seed: 1 });
  assert.equal(r.verdict, 'PASS', r.reasons.join('; '));
  assert.equal(r.tokens.deltaPct, 0);
  assert.ok(r.tokens.ci[0] < 0 && r.tokens.ci[1] > 0, `tokens CI ${r.tokens.ci}`);
  assert.ok(r.correctness.ci[0] < 0 && r.correctness.ci[1] > 0, `correctness CI ${r.correctness.ci}`);
});

test('a clear 20% token increase with low noise is a STOP', () => {
  const b = noisy(20, 100_000, 0.02);
  const c = noisy(20, 120_000, 0.02, 13);
  const r = comparePair(runs(20, (i) => ({ totalTokens: b[i] })), runs(20, (i) => ({ totalTokens: c[i] })), 20, { seed: 1 });
  assert.ok(r.tokens.ci[0] > 5, `lower bound ${r.tokens.ci[0]}`);
  assert.equal(r.tokens.loss, true);
  assert.equal(r.verdict, 'STOP');
});

test('a 6% median increase inside wide noise is not a loss (the plain comparison said STOP)', () => {
  const b = noisy(20, 100_000, 0.5);
  const c = b.map((x) => Math.round(x * 1.06));
  const r = comparePair(runs(20, (i) => ({ totalTokens: b[i] })), runs(20, (i) => ({ totalTokens: c[i] })), 20, { seed: 1 });
  assert.ok(Math.abs(r.tokens.deltaPct - 6) < 0.01, `point estimate ${r.tokens.deltaPct}`);
  assert.ok(r.tokens.ci[0] < 5, `lower bound ${r.tokens.ci[0]}`);
  assert.equal(r.tokens.loss, false);
  assert.equal(r.verdict, 'PASS');
});

test('the bootstrap is reproducible from its seed', () => {
  const b = noisy(20, 100_000, 0.5);
  const c = noisy(20, 104_000, 0.5, 14);
  const pair = (seed) => comparePair(runs(20, (i) => ({ totalTokens: b[i] })), runs(20, (i) => ({ totalTokens: c[i] })), 20, { seed });
  assert.deepEqual(pair(7).tokens.ci, pair(7).tokens.ci);
  assert.notDeepEqual(pair(7).tokens.ci, pair(8).tokens.ci);
  const stat = (x, y) => median(y) - median(x);
  assert.deepEqual(bootstrapCi(b, c, stat, { seed: 3 }), bootstrapCi(b, c, stat, { seed: 3 }));
});

test('comparePlan seeds each pair from the plan seed', () => {
  const recs = (arm) => noisy(20, 100_000, 0.5, arm === 'baseline' ? 21 : 22).map((t, i) => ({ ...runs(1)[0], id: `${arm}-${i}`, agent: 'pi', model: 'm', arm, totalTokens: t }));
  const plan = (seed) => ({ seed, runs: 20, agents: [{ agent: 'pi', model: 'm' }], arms: [{ name: 'baseline' }, { name: 'codecite' }] });
  const records = [...recs('baseline'), ...recs('codecite')];
  assert.deepEqual(comparePlan(plan(5), records)[0].tokens.ci, comparePlan(plan(5), records)[0].tokens.ci);
  assert.notDeepEqual(comparePlan(plan(5), records)[0].tokens.ci, comparePlan(plan(6), records)[0].tokens.ci);
});

test('bootstrapCi with an unknown value is null, so the measure can not pass', () => {
  assert.equal(bootstrapCi([1, null, 3], [1, 2, 3], (x, y) => median(y) - median(x), { seed: 1 }), null);
  assert.equal(bootstrapCi([], [1], (x, y) => 0, { seed: 1 }), null);
  const r = comparePair(runs(20), runs(20, (i) => ({ wallMs: i === 3 ? null : 60_000 })), 20);
  assert.equal(r.time.ci, null);
  assert.equal(r.time.loss, null);
  assert.equal(r.verdict, 'INCOMPLETE');
  assert.match(r.reasons.join(' '), /time unknown/);
});

test('stop on 6% more tokens', () => {
  const r = comparePair(runs(20), runs(20, { totalTokens: 106_000 }), 20);
  assert.equal(r.tokens.deltaPct, 6);
  assert.equal(r.tokens.loss, true);
  assert.equal(r.verdict, 'STOP');
});

test('exactly 5% more is not a loss', () => {
  const r = comparePair(runs(20), runs(20, { totalTokens: 105_000, wallMs: 63_000 }), 20);
  assert.equal(r.tokens.loss, false);
  assert.equal(r.time.loss, false);
  assert.equal(r.verdict, 'PASS');
});

test('pass on 4% more time', () => {
  const r = comparePair(runs(20), runs(20, { wallMs: 62_400 }), 20);
  assert.equal(r.time.deltaPct, 4);
  assert.equal(r.time.loss, false);
  assert.equal(r.verdict, 'PASS');
});

test('10 points lower pass rate inside the noise of 20 runs is not a loss', () => {
  // 18 of 20 pass: 90% against 100%; resampling reaches 0 points often.
  const r = comparePair(runs(20), runs(20, (i) => ({ passed: i >= 2 })), 20);
  assert.equal(r.correctness.baseline, 100);
  assert.equal(r.correctness.codecite, 90);
  assert.equal(r.correctness.deltaPts, -10);
  assert.ok(r.correctness.ci[1] >= -5, `upper bound ${r.correctness.ci[1]}`);
  assert.equal(r.correctness.loss, false);
});

test('stop on a clearly lower pass rate', () => {
  // 10 of 20 pass: 50% against 100%.
  const r = comparePair(runs(20), runs(20, (i) => ({ passed: i >= 10 })), 20);
  assert.equal(r.correctness.deltaPts, -50);
  assert.ok(r.correctness.ci[1] < -5, `upper bound ${r.correctness.ci[1]}`);
  assert.equal(r.correctness.loss, true);
  assert.equal(r.verdict, 'STOP');
});

test('5 points lower pass rate is not a loss', () => {
  const r = comparePair(runs(20), runs(20, (i) => ({ passed: i >= 1 })), 20);
  assert.equal(r.correctness.deltaPts, -5);
  assert.equal(r.correctness.loss, false);
});

test('incomplete when an arm is short', () => {
  const r = comparePair(runs(20), runs(19), 20);
  assert.equal(r.verdict, 'INCOMPLETE');
  assert.equal(r.runs.codecite, 19);
});

test('a run with unknown tokens is excluded, so its arm is short', () => {
  // Redone on resume (call keeps the old one aside); until then INCOMPLETE.
  const r = comparePair(runs(20), runs(20, (i) => ({ totalTokens: i === 3 ? null : 100_000 })), 20);
  assert.equal(r.runs.codecite, 19);
  assert.equal(r.tokens.codecite, 100_000);
  assert.equal(r.verdict, 'INCOMPLETE');
  assert.match(r.reasons.join(' '), /codecite has 19 of 20 usable runs/);
});

test('an unknown measure and a loss is still STOP', () => {
  const r = comparePair(runs(20), runs(20, { wallMs: null, totalTokens: 150_000 }), 20);
  assert.equal(r.time.loss, null);
  assert.equal(r.tokens.loss, true);
  assert.equal(r.verdict, 'STOP');
});

test('tainted runs excluded', () => {
  // Two tainted codecite runs with huge token counts: left out, the arm is short.
  const codecite = [...runs(20), ...runs(2, { tainted: ['curl x'], totalTokens: 9_000_000 })];
  const r = comparePair(runs(20), codecite, 20);
  assert.equal(r.verdict, 'PASS');
  assert.equal(r.runs.codecite, 20);
  assert.equal(r.tokens.codecite, 100_000);
  const short = comparePair(runs(20), [...runs(19), ...runs(1, { tainted: ['curl x'] })], 20);
  assert.equal(short.verdict, 'INCOMPLETE');
});

test('usable runs: ok, timeout and agent_error, scored and untainted', () => {
  const recs = [
    ...runs(1, { status: 'ok' }),
    ...runs(1, { status: 'timeout', passed: false }),
    ...runs(1, { status: 'agent_error', passed: false }),
    ...runs(1, { status: 'infra_error', passed: null }),
    ...runs(1, { status: 'fatal', passed: null }),
    ...runs(1, { status: 'ok', passed: null }), // never scored
    ...runs(1, { status: 'ok', tainted: ['wget y'] }),
    ...runs(1, { status: 'ok', totalTokens: null }), // redone on resume
    ...runs(1, { status: 'ok', superseded: true }), // a kept-aside dir
  ];
  assert.deepEqual(
    usableRuns(recs).map((r) => r.status),
    ['ok', 'timeout', 'agent_error'],
  );
});

test('failed and timed-out runs count in the medians', () => {
  const codecite = runs(20, (i) => (i < 15 ? { status: 'timeout', passed: false, totalTokens: 300_000 } : {}));
  const r = comparePair(runs(20), codecite, 20);
  assert.equal(r.tokens.codecite, 300_000);
  assert.equal(r.tokens.passingCodecite, 100_000, 'the passing-only median is shown beside it');
  assert.equal(r.verdict, 'STOP');
});

test('unscored runs make the pair INCOMPLETE', () => {
  // Two codecite runs unscored by a PyPI outage: not failures, so no false STOP.
  const codecite = runs(20, (i) => (i < 2 ? { passed: null, unscored: 'no test report (exit 2)' } : {}));
  const r = comparePair(runs(20), codecite, 20);
  assert.equal(r.verdict, 'INCOMPLETE');
  assert.equal(r.runs.codecite, 18);
  assert.equal(r.correctness.codecite, 100);
});

test('renderScorecard uses the pairs it is given', () => {
  const plan = { roundId: 'r', runs: 1, agents: [{ agent: 'a', model: 'm' }], arms: [{ name: 'baseline' }, { name: 'codecite' }] };
  const pairs = [{ agent: 'a', model: 'm', arm: 'codecite', ...comparePair(runs(1), runs(1, { wallMs: 999_000 }), 1) }];
  assert.match(renderScorecard({ plan, records: [], pairs }), /Verdict: \*\*STOP\*\*/);
});

test('round STOP if any pair STOPs', () => {
  const pass = comparePair(runs(20), runs(20), 20);
  const stop = comparePair(runs(20), runs(20, { wallMs: 120_000 }), 20);
  const inc = comparePair(runs(20), runs(3), 20);
  assert.equal(roundVerdict([pass, stop, inc]), 'STOP');
  assert.equal(roundVerdict([pass, inc]), 'INCOMPLETE');
  assert.equal(roundVerdict([pass, pass]), 'PASS');
  assert.equal(roundVerdict([]), 'INCOMPLETE');
});

/** A round on disk: plan.json, round.json and runs with result.json and score.json. */
function roundOnDisk(plannedRuns, recs) {
  const dir = mkdtempSync(join(tmpdir(), 'evals-scorecard-'));
  const plan = {
    roundId: 'r1',
    task: 't',
    runs: plannedRuns,
    seed: 1,
    timeoutSec: 600,
    agents: [{ agent: 'pi', model: 'm/x' }],
    arms: [{ name: 'baseline' }, { name: 'codecite', codecite: true, note: 'Use `codecite search`.' }],
    codeciteCommit: 'abc',
    taskDef: { id: 't', promptText: 'Do it.' },
  };
  writeFileSync(join(dir, 'plan.json'), JSON.stringify(plan));
  writeFileSync(join(dir, 'round.json'), JSON.stringify({ dir }));
  for (const r of recs) {
    const out = join(dir, 'runs', r.id);
    mkdirSync(out, { recursive: true });
    const { passed, unscored, scoreFatal, totalTokens, costUsd, ...rest } = r;
    writeFileSync(join(out, 'result.json'), JSON.stringify({ ...rest, usage: { totalTokens, costUsd } }));
    if (passed != null) writeFileSync(join(out, 'score.json'), JSON.stringify({ passed, failures: passed ? [] : ['t::x'] }));
    if (unscored) writeFileSync(join(out, 'score.json'), JSON.stringify({ passed: null, failures: [], unscored, ...(scoreFatal ? { fatal: true, scoreAttempts: 3 } : {}) }));
  }
  return dir;
}

test('loadRecords and renderScorecard', () => {
  const rec = (arm, rep, over = {}) => ({
    id: `pi-m_x-${arm}-${rep}`,
    run: { id: `pi-m_x-${arm}-${rep}`, agent: 'pi', model: 'm/x', arm, rep },
    status: 'ok',
    tainted: [],
    attempts: 1,
    passed: true,
    totalTokens: 1000,
    wallMs: 5000,
    costUsd: 0.01,
    toolCalls: 3,
    codeciteCalls: arm === 'codecite' ? 1 : 0,
    ...over,
  });
  const dir = roundOnDisk(2, [
    rec('baseline', 1),
    rec('baseline', 2, { attempts: 2 }),
    rec('codecite', 1, { tainted: ['curl https://github.com/x'] }),
    rec('codecite', 2, { totalTokens: null }),
    rec('codecite', 3, { status: 'infra_error', passed: null, attempts: 3 }),
    rec('codecite', 4, { passed: null, unscored: 'warm step failed (exit 2): error: Failed to fetch pytest', scoreFatal: true }),
    // Kept aside by a resume: a tainted and an unknown-token run of rep 1.
    { ...rec('codecite', 1, { tainted: ['shell (wget): wget x'] }), id: 'pi-m_x-codecite-1.tainted-1' },
    { ...rec('codecite', 1, { totalTokens: null }), id: 'pi-m_x-codecite-1.unknown-1' },
    rec('baseline', 3, { secretInTranscript: true }),
    rec('codecite', 5),
  ]);
  try {
    const records = loadRecords(dir);
    assert.equal(records.length, 10);
    const aside = records.find((r) => r.id === 'pi-m_x-codecite-1.tainted-1');
    assert.equal(aside.superseded, true);
    assert.equal(aside.arm, 'codecite');
    assert.equal(records.find((r) => r.id === 'pi-m_x-codecite-1').superseded, false);
    assert.equal(records.find((r) => r.id === 'pi-m_x-codecite-1.unknown-1').tokensUnknown, true);
    const r4 = records.find((r) => r.id === 'pi-m_x-codecite-4');
    assert.equal(r4.passed, null);
    assert.match(r4.unscored, /Failed to fetch/);
    assert.equal(r4.scoreFatal, true);
    const r2 = records.find((r) => r.id === 'pi-m_x-codecite-2');
    assert.equal(r2.totalTokens, null);
    assert.equal(r2.passed, true);
    assert.equal(records.find((r) => r.id === 'pi-m_x-codecite-3').passed, null);

    const md = renderScorecard(dir);
    assert.match(md, /^# /);
    assert.match(md, /Verdict: \*\*INCOMPLETE\*\*/);
    assert.match(md, /pi-m_x-codecite-1/, 'tainted runs are listed');
    assert.match(md, /`pi-m_x-codecite-1\.tainted-1` \(kept aside; redone\): `shell \(wget\): wget x`/, 'a tainted run kept aside is listed as tainted and redone');
    assert.match(md, /`pi-m_x-codecite-1`: `curl https:\/\/github.com\/x`/, 'a live tainted run is listed plainly');
    assert.match(md, /`pi-m_x-codecite-1\.unknown-1`: token usage unknown/, 'an unknown-token run kept aside is listed');
    assert.match(md, /`pi-m_x-codecite-2`: token usage unknown/, 'an unknown-token run is listed as excluded');
    assert.match(md, /## Secrets in transcripts[\s\S]*rotate[\s\S]*`pi-m_x-baseline-3`/, 'runs that echoed a secret are listed with a rotate warning');
    assert.match(md, /95% CI/, 'each measure shows its interval');
    assert.match(md, /`baseline` = `eval-[0-9a-f]{8}`, `codecite` = `eval-[0-9a-f]{8}`/, 'the X-Title of each arm is mapped back');
    assert.match(md, /pi-m_x-baseline-2/, 'retried runs are listed');
    assert.match(md, /pi-m_x-codecite-3.*infra_error/, 'excluded runs are listed with their status');
    assert.match(md, /pi-m_x-codecite-4`: unscored \(warm step failed \(exit 2\): error: Failed to fetch pytest\), a fatal scoring error after 3 attempts/, 'unscored runs are listed with the reason');
    assert.match(md, /\*\*Scoring errors:\*\* 1 run could not be scored \(`pi-m_x-codecite-4`\)/, 'fatal scoring errors are surfaced at the top');
    assert.match(md, /Use `codecite search`\./, 'the codecite note is printed');
    assert.match(md, /"codeciteCommit": "abc"/, 'the frozen plan is printed');
    assert.match(md, /unknown/, 'a null median shows as unknown');
    assert.doesNotMatch(md, /NaN|undefined/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
