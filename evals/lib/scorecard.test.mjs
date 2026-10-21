import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { comparePair, iqr, loadRecords, median, renderScorecard, roundVerdict, usableRuns } from './scorecard.mjs';

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
  assert.equal(r.tokens.loss, false);
  assert.equal(r.tokens.deltaPct, 0);
  assert.equal(r.time.loss, false);
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

test('stop on 10 points lower pass rate', () => {
  // 18 of 20 pass: 90% against 100%.
  const r = comparePair(runs(20), runs(20, (i) => ({ passed: i >= 2 })), 20);
  assert.equal(r.correctness.baseline, 100);
  assert.equal(r.correctness.codecite, 90);
  assert.equal(r.correctness.deltaPts, -10);
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

test('null tokens never pass', () => {
  const r = comparePair(runs(20), runs(20, (i) => ({ totalTokens: i === 3 ? null : 100_000 })), 20);
  assert.equal(r.tokens.codecite, null);
  assert.equal(r.tokens.deltaPct, null);
  assert.equal(r.tokens.loss, null);
  assert.notEqual(r.verdict, 'PASS');
  assert.equal(r.verdict, 'INCOMPLETE');
  assert.match(r.reasons.join(' '), /tokens/);
});

test('a null median and a loss is still STOP', () => {
  const r = comparePair(runs(20), runs(20, { totalTokens: null, wallMs: 90_000 }), 20);
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
  ];
  assert.deepEqual(
    usableRuns(recs).map((r) => r.status),
    ['ok', 'timeout', 'agent_error'],
  );
});

test('failed and timed-out runs count in the medians', () => {
  const codecite = runs(20, (i) => (i < 10 ? { status: 'timeout', passed: false, totalTokens: 300_000 } : {}));
  const r = comparePair(runs(20), codecite, 20);
  assert.equal(r.tokens.codecite, 200_000);
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
  ]);
  try {
    const records = loadRecords(dir);
    assert.equal(records.length, 6);
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
