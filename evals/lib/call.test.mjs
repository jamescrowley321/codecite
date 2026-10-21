import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { callRound } from './call.mjs';
import { emptyUsage } from './telemetry.mjs';

const FAKE_AGENT = fileURLToPath(new URL('./testdata/fake-agent.mjs', import.meta.url));
const KEY = 'sk-or-v1-FAKEKEY0123456789abcdef';
const OAUTH = 'sk-ant-oat01-FAKEOAUTH9876543210';
let tmp, taskDir, n = 0;

before(() => {
  tmp = mkdtempSync(join(tmpdir(), 'evals-call-'));
  taskDir = join(tmp, 'task');
  mkdirSync(join(taskDir, 'project', 'app'), { recursive: true });
  writeFileSync(join(taskDir, 'project', 'app', 'main.py'), 'app = None\n');
});
after(() => rmSync(tmp, { recursive: true, force: true }));

function fakeAdapter(name) {
  return {
    name,
    command(run, dirs, ctx) {
      const mode = run.model.split('/')[1];
      return {
        cmd: mode === 'missing' ? join(tmp, 'no-such-agent') : process.execPath,
        args: [FAKE_AGENT, mode],
        env: {
          PATH: dirs.path,
          HOME: dirs.home,
          FAKE_KEY: ctx.secrets.openrouter,
          FAKE_OAUTH: ctx.secrets.claudeOauth,
          FAKE_PROMPT: ctx.prompt,
          FAKE_COUNTER: join(ctx.round.dir, '..', `${ctx.round.plan.roundId}.count`),
          FAKE_PIDFILE: join(ctx.round.dir, '..', `${ctx.round.plan.roundId}.pid`),
        },
        cwd: dirs.project,
      };
    },
    earlyReject(line) {
      return line.includes('"earlyReject"') ? 'user config seen at init' : null;
    },
    parse(stdout) {
      const last = stdout
        .split('\n')
        .map((l) => {
          try {
            return JSON.parse(l);
          } catch {
            return null;
          }
        })
        .filter((o) => o?.type === 'result')
        .at(-1);
      return {
        usage: emptyUsage(),
        turns: last?.turns ?? null,
        toolCalls: null,
        codeciteCalls: null,
        transcript: stdout,
        providerStatus: last?.providerStatus ?? null,
        init: last?.init ?? null,
        rejectReason: last?.rejectReason ?? null,
        shellCommands: last?.shell ?? [],
        readPaths: last?.reads ?? [],
      };
    },
  };
}

/** A round on disk with a baseline arm only, for `runs` reps of each agent. */
function makeRound(agents, { runs = 1 } = {}) {
  const roundId = `r${++n}`;
  const dir = join(tmp, 'rounds', roundId);
  const exportDir = join(dir, 'identity-model');
  const xdg = join(dir, 'xdg');
  const binCodecite = join(dir, 'bin-codecite');
  for (const d of [exportDir, xdg, binCodecite]) mkdirSync(d, { recursive: true });
  const plan = {
    task: 'fake',
    runs,
    seed: 7,
    timeoutSec: 60,
    repos: { 'identity-model': join(tmp, 'real-repo') },
    agents,
    arms: [{ name: 'baseline' }],
    roundId,
    taskDef: { id: 'fake', dir: taskDir, project: 'project', setup: [], promptText: 'Do the task.\n' },
  };
  const round = {
    dir,
    export: exportDir,
    manifest: join(dir, 'corpus.toml'),
    xdg,
    binCodecite,
    taintPaths: [join(tmp, 'real-repo'), join(dir, 'plan.json'), join(dir, 'round.json'), join(dir, 'corpus.toml'), join(dir, 'runs')],
  };
  writeFileSync(join(dir, 'plan.json'), JSON.stringify(plan));
  writeFileSync(join(dir, 'round.json'), JSON.stringify(round));
  return { dir, roundId, counter: join(tmp, 'rounds', `${roundId}.count`), pidfile: join(tmp, 'rounds', `${roundId}.pid`) };
}

function deps(over = {}) {
  return {
    secrets: { openrouter: KEY, claudeOauth: OAUTH },
    loadAdapter: async (name) => fakeAdapter(name),
    keyRemainingUsd: async () => 100,
    modelPrices: async () => ({ get: () => ({ prompt: 0, completion: 0 }) }),
    log: () => {},
    ...over,
  };
}

const count = (file) => (existsSync(file) ? readFileSync(file, 'utf8').trim().split('\n').length : 0);
const results = (dir) =>
  existsSync(join(dir, 'runs'))
    ? readdirSync(join(dir, 'runs'))
        .filter((id) => existsSync(join(dir, 'runs', id, 'result.json')))
        .map((id) => JSON.parse(readFileSync(join(dir, 'runs', id, 'result.json'), 'utf8')))
    : [];

function filesUnder(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    return e.isDirectory() ? filesUnder(p) : e.isFile() ? [p] : [];
  });
}

test('an ok run records stdout, stderr and result.json without the transcript', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/ok' }]);
  process.env.LEAK_SENTINEL = '1';
  let summary;
  try {
    summary = await callRound(r.dir, deps());
  } finally {
    delete process.env.LEAK_SENTINEL;
  }
  assert.equal(summary.stopped, null);
  const [res] = results(r.dir);
  assert.equal(res.status, 'ok');
  assert.equal(res.run.id, 'fake-m_ok-baseline-1');
  assert.equal(typeof res.wallMs, 'number');
  assert.equal(res.turns, 2);
  assert.equal(res.providerStatus, 200);
  assert.equal(res.attempts, 1);
  assert.equal('transcript' in res, false);
  assert.deepEqual(res.tainted, []);
  assert.equal(res.init.leak, false, 'the agent env is exactly the adapter env');
  const out = join(r.dir, 'runs', res.run.id);
  for (const f of ['stdout.jsonl', 'stderr.txt', 'result.json', 'project']) assert.ok(existsSync(join(out, f)), f);
  assert.match(readFileSync(join(out, 'stdout.jsonl'), 'utf8'), /Do the task\./);
});

test('timeout kills the agent and marks timeout', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/hang' }]);
  const t0 = Date.now();
  await callRound(r.dir, deps({ timeoutMs: 400, killGraceMs: 300 }));
  assert.ok(Date.now() - t0 < 10000);
  const [res] = results(r.dir);
  assert.equal(res.status, 'timeout');
  assert.ok(res.wallMs >= 400, `wallMs ${res.wallMs}`);
  const pid = Number(readFileSync(r.pidfile, 'utf8'));
  assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' }, 'SIGTERM-ignoring agent was SIGKILLed');
});

test('fatal stops the round', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/fatal' }], { runs: 3 });
  const summary = await callRound(r.dir, deps());
  assert.equal(summary.stopped, 'fatal');
  assert.match(summary.reason, /401/);
  assert.equal(count(r.counter), 1, 'not retried, no further runs');
  const res = results(r.dir);
  assert.equal(res.length, 1);
  assert.equal(res[0].status, 'fatal');
  assert.match(res[0].error, /provider status 401/);
  assert.equal(res[0].providerStatus, 401);
});

test('a round with a fatal run refuses to resume without retryFatal', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/fatal' }], { runs: 2 });
  await callRound(r.dir, deps());
  await assert.rejects(callRound(r.dir, deps()), /fatal.*--retry-fatal/s);
  assert.equal(count(r.counter), 1);
});

test('retryFatal re-runs the fatal run first', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/ok' }], { runs: 3 });
  await callRound(r.dir, deps());
  // Mark the last run fatal and another one an agent_error.
  const res = results(r.dir).sort((a, b) => a.run.seq - b.run.seq);
  const mark = (x, status) => {
    const f = join(r.dir, 'runs', x.run.id, 'result.json');
    writeFileSync(f, JSON.stringify({ ...x, status }));
  };
  mark(res[2], 'fatal');
  mark(res[0], 'agent_error');
  const order = [];
  const summary = await callRound(r.dir, deps({ retryFatal: true, log: (l) => order.push(l) }));
  assert.equal(summary.ran, 2);
  const ran = order.filter((l) => l.startsWith('[')).map((l) => Number(/^\[(\d+)\//.exec(l)[1]));
  assert.deepEqual(ran, [3, 1]);
});

test('an early reject kills the agent and stops the round', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/earlyreject' }], { runs: 2 });
  const t0 = Date.now();
  const summary = await callRound(r.dir, deps({ killGraceMs: 200 }));
  assert.ok(Date.now() - t0 < 5000);
  assert.equal(summary.stopped, 'fatal');
  assert.match(summary.reason, /user config seen at init/);
  const [res] = results(r.dir);
  assert.equal(res.status, 'fatal');
  assert.equal(res.rejectReason, 'user config seen at init');
  const pid = Number(readFileSync(r.pidfile, 'utf8'));
  assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' });
});

test('timeouts count as failures for the breaker', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/slow' }], { runs: 11 });
  const summary = await callRound(r.dir, deps({ timeoutMs: 100, killGraceMs: 100 }));
  assert.equal(summary.stopped, 'breaker');
  assert.equal(count(r.counter), 10);
  assert.ok(results(r.dir).every((x) => x.status === 'timeout'));
});

test('a rejectReason stops the round', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/reject' }], { runs: 2 });
  const summary = await callRound(r.dir, deps());
  assert.equal(summary.stopped, 'fatal');
  assert.match(summary.reason, /user config leaked/);
  assert.equal(count(r.counter), 1);
});

test('infra failure is retried up to 3 times then recorded', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/infra' }]);
  const summary = await callRound(r.dir, deps());
  assert.equal(summary.stopped, null);
  assert.equal(count(r.counter), 3);
  const [res] = results(r.dir);
  assert.equal(res.status, 'infra_error');
  assert.equal(res.attempts, 3);
});

test('a spawn failure is infra, retried, then recorded', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/missing' }]);
  await callRound(r.dir, deps());
  const [res] = results(r.dir);
  assert.equal(res.status, 'infra_error');
  assert.equal(res.attempts, 3);
  assert.match(res.error, /ENOENT/);
});

test('the breaker stops a round of failing runs after 10', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/error' }], { runs: 12 });
  const summary = await callRound(r.dir, deps());
  assert.equal(summary.stopped, 'breaker');
  assert.equal(count(r.counter), 10);
  assert.ok(results(r.dir).every((x) => x.status === 'agent_error'));
});

test('refuses when the estimate exceeds the key\'s remaining limit', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/ok' }], { runs: 2 });
  const prices = new Map([['m/ok', { prompt: 1e-6, completion: 1e-5 }]]);
  await assert.rejects(
    callRound(r.dir, deps({ keyRemainingUsd: async () => 0.75, modelPrices: async () => prices })),
    (e) => /estimate \$1\.00 exceeds/.test(e.message) && !e.message.includes(KEY),
  );
  assert.equal(count(r.counter), 0);
  assert.equal(existsSync(join(r.dir, 'runs')), false);
});

test('refuses a key with no limit', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/ok' }]);
  await assert.rejects(callRound(r.dir, deps({ keyRemainingUsd: async () => null })), /no spending limit/);
  assert.equal(count(r.counter), 0);
});

test('claude-only rounds skip the OpenRouter check', async () => {
  const r = makeRound([{ agent: 'claude', model: 'claude/ok' }]);
  const boom = async () => {
    throw new Error('network touched');
  };
  const summary = await callRound(r.dir, deps({ keyRemainingUsd: boom, modelPrices: boom }));
  assert.equal(summary.ran, 1);
});

test('result.json never contains the key', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/ok' }, { agent: 'fake', model: 'm/error' }], { runs: 2 });
  await callRound(r.dir, deps());
  const files = filesUnder(r.dir);
  assert.ok(files.some((f) => f.endsWith('stdout.jsonl')));
  for (const f of files) {
    const text = readFileSync(f, 'utf8');
    assert.ok(!text.includes(KEY), `${f} holds the OpenRouter key`);
    assert.ok(!text.includes(OAUTH), `${f} holds the OAuth token`);
  }
  const ok = results(r.dir).find((x) => x.status === 'ok');
  assert.match(readFileSync(join(r.dir, 'runs', ok.run.id, 'stdout.jsonl'), 'utf8'), /\[REDACTED\]/);
  assert.match(readFileSync(join(r.dir, 'runs', ok.run.id, 'stderr.txt'), 'utf8'), /key=\[REDACTED\]/);
});

test('call resumes: ok and timeout runs are skipped, others redone', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/ok' }], { runs: 3 });
  await callRound(r.dir, deps());
  assert.equal(count(r.counter), 3);
  // Turn one finished run into an agent_error; it is the only one redone.
  const id = results(r.dir)[0].run.id;
  const file = join(r.dir, 'runs', id, 'result.json');
  writeFileSync(file, JSON.stringify({ ...JSON.parse(readFileSync(file, 'utf8')), status: 'agent_error' }));
  const summary = await callRound(r.dir, deps());
  assert.equal(summary.skipped, 2);
  assert.equal(summary.ran, 1);
  assert.equal(count(r.counter), 4);
  assert.ok(results(r.dir).every((x) => x.status === 'ok'));
});

test('runs go in seq order', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/ok' }], { runs: 4 });
  const order = [];
  await callRound(r.dir, deps({ log: (line) => order.push(line) }));
  const seqs = order.map((l) => Number(/^\[(\d+)\//.exec(l)?.[1])).filter(Boolean);
  assert.deepEqual(seqs, [1, 2, 3, 4]);
});

test('a read of the round plan taints the run', async () => {
  const r = makeRound([{ agent: 'fake', model: 'm/taint' }]);
  await callRound(r.dir, deps());
  const [res] = results(r.dir);
  assert.equal(res.status, 'ok');
  assert.equal(res.tainted.length, 1);
  assert.match(res.tainted[0], /plan\.json/);
});

test("an adapter's own secrets are redacted like ctx.secrets", async () => {
  const EXTRA = 'sk-ant-oat01-ADAPTERREADTOKEN5555';
  const base = fakeAdapter('fake');
  const adapter = {
    ...base,
    command(run, dirs, ctx) {
      const spec = base.command(run, dirs, ctx);
      return { ...spec, env: { ...spec.env, FAKE_KEY: EXTRA }, secrets: [EXTRA] };
    },
  };
  const r = makeRound([{ agent: 'fake', model: 'm/fatal' }, { agent: 'fake', model: 'm/ok' }]);
  await callRound(r.dir, deps({ loadAdapter: async () => adapter }));
  const files = filesUnder(r.dir);
  assert.ok(files.some((f) => f.endsWith('stdout.jsonl')));
  for (const f of files) assert.ok(!readFileSync(f, 'utf8').includes(EXTRA), `${f} holds the adapter's token`);
});
