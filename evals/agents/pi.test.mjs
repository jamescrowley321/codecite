import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import pi, { TOOLS, configLeak } from './pi.mjs';

const fixture = (name) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8');
const STREAM = fixture('pi-stream.jsonl');
const LEAKED = fixture('pi-leaked.jsonl');

const line = (obj) => JSON.stringify(obj);
const usage = (input, output, cost) => ({
  input,
  output,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: input + output,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: cost },
});
const assistantEnd = (message) =>
  line({ type: 'message_end', message: { role: 'assistant', content: [], usage: usage(0, 0, 0), stopReason: 'stop', ...message } });
const toolStart = (toolName, args) => line({ type: 'tool_execution_start', toolCallId: `c${Math.random()}`, toolName, args });

/** A fake run with a stub `pi` on its PATH, as prepareRun would lay it out. */
function fakeDirs({ codecite = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'pi-adapter-'));
  const sys = join(root, 'sysbin');
  mkdirSync(sys);
  writeFileSync(join(sys, 'pi'), '#!/bin/sh\n');
  chmodSync(join(sys, 'pi'), 0o755);
  const bin = codecite ? join(root, 'bin-codecite') : null;
  if (bin) {
    mkdirSync(bin);
    writeFileSync(join(bin, 'codecite'), '#!/bin/sh\n');
    chmodSync(join(bin, 'codecite'), 0o755);
  }
  const dirs = { out: root, project: join(root, 'project'), home: join(root, 'home'), xdg: join(root, 'xdg'), bin };
  for (const d of [dirs.project, dirs.home, dirs.xdg]) mkdirSync(d);
  dirs.path = bin ? [bin, sys].join(delimiter) : sys;
  return dirs;
}

const RUN = { id: 'r1', agent: 'pi', model: 'deepseek/deepseek-v4.1-flash', arm: 'baseline', rep: 1, seq: 1 };
const CTX = { prompt: 'Fix the bug.', secrets: { openrouter: 'or-test-123', claudeOauth: null }, plan: {}, round: {} };

test('sums usage across assistant messages', () => {
  const r = pi.parse(STREAM, '');
  // The fixture's two assistant messages: a tool-use turn, then the reply.
  assert.equal(r.usage.input, 2308 + 248);
  assert.equal(r.usage.output, 130 + 3);
  assert.equal(r.usage.cacheRead, 0 + 2304);
  assert.equal(r.usage.cacheWrite, 0);
  assert.equal(r.usage.totalTokens, 2438 + 2555);
  assert.ok(Math.abs(r.usage.costUsd - (0.00042419999999999996 + 0.000045912)) < 1e-12);
  assert.equal(r.turns, 2);
  assert.equal(r.toolCalls, 3);
  assert.equal(r.codeciteCalls, 0);
  assert.equal(r.providerStatus, null);
  assert.equal(r.rejectReason, null);
  assert.equal(r.endedOnError, false);
  assert.equal(r.init.model, 'deepseek/deepseek-v4.1-flash');
  assert.deepEqual(r.init.tools, TOOLS);
  assert.deepEqual(r.shellCommands, ['echo hi']);
  assert.deepEqual(r.readPaths, ['hello.txt', '.']);
  assert.match(r.transcript, /\$ echo hi/);
  assert.match(r.transcript, /ok/);
});

test('tolerates a truncated last line', () => {
  const lines = STREAM.trim().split('\n');
  const last = lines.pop();
  const r = pi.parse([...lines, last.slice(0, Math.floor(last.length / 2))].join('\n'), '');
  assert.equal(r.usage.totalTokens, 2438 + 2555);
  assert.equal(r.turns, 2);
});

test('no assistant message leaves usage and turns unknown', () => {
  const head = STREAM.split('\n').filter((l) => l && !l.includes('"role":"assistant"') && !l.includes('tool_execution'));
  const r = pi.parse(head.join('\n'), '');
  assert.deepEqual(r.usage, { input: null, output: null, cacheRead: null, cacheWrite: null, totalTokens: null, costUsd: null });
  assert.equal(r.turns, null);
});

test('a usage field missing from one message is unknown in the sum', () => {
  const partial = usage(10, 2, 0.001);
  delete partial.cost;
  const r = pi.parse([assistantEnd({ usage: usage(5, 1, 0.002) }), assistantEnd({ usage: partial })].join('\n'), '');
  assert.equal(r.usage.input, 15);
  assert.equal(r.usage.costUsd, null);
});

test('extracts a 404 as providerStatus', () => {
  const err = assistantEnd({
    stopReason: 'error',
    errorMessage: '404: {"error":{"message":"No endpoints found for deepseek/deepseek-v4.1-flash.","code":404}}',
  });
  const r = pi.parse(err, '');
  assert.equal(r.providerStatus, 404);
  assert.equal(r.turns, 0);
});

test('a 429 before any turn ends the run on error', () => {
  const r = pi.parse(assistantEnd({ stopReason: 'error', errorMessage: '429 Rate limit exceeded' }), '');
  assert.equal(r.providerStatus, 429);
  assert.equal(r.turns, 0);
  assert.equal(r.endedOnError, true);
});

test('a statusless error message ends the run on error with no status', () => {
  const r = pi.parse(assistantEnd({ stopReason: 'error', errorMessage: 'Connection error.' }), '');
  assert.equal(r.providerStatus, null);
  assert.equal(r.turns, 0);
  assert.equal(r.endedOnError, true);
});

test('an error after turns ends the run on error', () => {
  const r = pi.parse([assistantEnd({}), assistantEnd({ stopReason: 'error', errorMessage: '502: bad gateway' })].join('\n'), '');
  assert.equal(r.providerStatus, 502);
  assert.equal(r.turns, 1);
  assert.equal(r.endedOnError, true);
});

test('a 401 from the recorded leaked run is its providerStatus', () => {
  const r = pi.parse(LEAKED, '');
  assert.equal(r.providerStatus, 401);
  assert.equal(r.turns, 0);
});

test('a later successful message clears an earlier error status', () => {
  const r = pi.parse([assistantEnd({ stopReason: 'error', errorMessage: '503 overloaded' }), assistantEnd({})].join('\n'), '');
  assert.equal(r.providerStatus, null);
  assert.equal(r.turns, 1);
  assert.equal(r.endedOnError, false);
});

test('maps tool args into shellCommands, readPaths and codeciteCalls', () => {
  const r = pi.parse(
    [
      toolStart('bash', { command: 'codecite search eval "jwt"' }),
      toolStart('bash', { command: 'cd src && codecite read eval "x#L1"' }),
      toolStart('bash', { command: 'echo codecite' }),
      toolStart('read', { path: '/abs/test_hidden/t.py' }),
      toolStart('edit', { path: 'app/main.py', edits: [] }),
      toolStart('write', { path: 'app/new.py', content: 'x' }),
      toolStart('grep', { pattern: 'token', path: '../..' }),
      toolStart('grep', { pattern: 'token' }),
      toolStart('find', { pattern: '**/*.py', path: 'app' }),
      toolStart('ls', { path: '/' }),
      toolStart('ls', {}),
    ].join('\n'),
    '',
  );
  assert.equal(r.toolCalls, 11);
  assert.equal(r.codeciteCalls, 2);
  assert.deepEqual(r.shellCommands, ['codecite search eval "jwt"', 'cd src && codecite read eval "x#L1"', 'echo codecite']);
  assert.deepEqual(r.readPaths, ['/abs/test_hidden/t.py', 'app/main.py', 'app/new.py', '../..', 'app', '**/*.py', '/']);
});

test("rejects the owner's skills, context files and appended prompt", () => {
  const r = pi.parse(LEAKED, '');
  assert.equal(r.rejectReason, 'config leaked');
  const sys = LEAKED.split('\n').find((l) => l.includes('"type":"message_end"') && l.includes('"role":"system"'));
  assert.equal(pi.earlyReject(sys), 'config leaked');
});

test('the clean recorded run is not a leak', () => {
  for (const l of STREAM.split('\n').filter(Boolean)) assert.equal(pi.earlyReject(l), null);
});

test('configLeak flags each leak signal on its own', () => {
  const clean = JSON.parse(STREAM.split('\n').find((l) => l.includes('"type":"message_end"') && l.includes('"role":"system"')));
  const withSection = (k, v) => ({ ...clean, message: { ...clean.message, sections: { ...clean.message.sections, [k]: v } } });
  assert.equal(configLeak(clean), null);
  assert.equal(configLeak({ ...clean, type: 'message_start' }), null);
  assert.equal(configLeak(withSection('skills', '<skills/>')), 'config leaked');
  assert.equal(configLeak(withSection('addendum', 'x')), 'config leaked');
  assert.equal(configLeak(withSection('something_new', 'x')), 'config leaked');
  const extraTool = { ...clean, message: { ...clean.message, toolsAdded: [...clean.message.toolsAdded, { name: 'mcp__x__y' }] } };
  assert.equal(configLeak(extraTool), 'config leaked');
  // Context files inside the run's cwd (its `cwd` section) are the task's
  // own; anywhere else, or with no cwd to check against, is a leak.
  const ctx = (p) => withSection('project_context', `<project_context>\n<project_instructions path="${p}">\nx\n</project_instructions>\n</project_context>`);
  assert.equal(configLeak(ctx('/home/user/run/project/AGENTS.md')), null);
  assert.equal(configLeak(ctx('/home/user/run/project/sub/CLAUDE.md')), null);
  assert.equal(configLeak(ctx('/home/user/run/AGENTS.md')), 'config leaked');
  assert.equal(configLeak(ctx('/home/user/run/project-other/AGENTS.md')), 'config leaked');
  assert.equal(configLeak(withSection('project_context', '<project_context>\nx\n</project_context>')), 'config leaked');
  const noCwd = ctx('/home/user/run/project/AGENTS.md');
  delete noCwd.message.sections.cwd;
  assert.equal(configLeak(noCwd), 'config leaked');
  assert.equal(configLeak({ type: 'tool_execution_start' }), null);
  assert.equal(configLeak({ type: 'message_end', message: { role: 'assistant' } }), null);
});

test('command runs pi with the isolation flags and the same tools in both arms', () => {
  const a = pi.command(RUN, fakeDirs(), CTX);
  const b = pi.command({ ...RUN, arm: 'codecite' }, fakeDirs({ codecite: true }), CTX);
  for (const spec of [a, b]) {
    assert.ok(spec.cmd.endsWith('/pi'));
    assert.equal(spec.args[0], '-p');
    // The prompt is a positional message after `--`, so a leading dash can't read as a flag.
    assert.deepEqual(spec.args.slice(-2), ['--', 'Fix the bug.']);
    for (const f of ['--no-session', '--no-mcp', '--no-extensions', '--no-skills']) assert.ok(spec.args.includes(f), f);
    assert.equal(spec.args[spec.args.indexOf('--mode') + 1], 'json');
    assert.equal(spec.args[spec.args.indexOf('--provider') + 1], 'openrouter');
    assert.equal(spec.args[spec.args.indexOf('--model') + 1], 'deepseek/deepseek-v4.1-flash');
    assert.equal(spec.args[spec.args.indexOf('--tools') + 1], 'read,bash,edit,write,grep,find,ls');
  }
  assert.deepEqual(a.args, b.args);
});

test('env is built from nothing, with the key only in env', () => {
  const dirs = fakeDirs();
  const spec = pi.command(RUN, dirs, CTX);
  assert.equal(spec.env.OPENROUTER_API_KEY, 'or-test-123');
  assert.equal(spec.env.HOME, dirs.home);
  assert.equal(spec.env.PI_CODING_AGENT_DIR, join(dirs.home, 'pi'));
  assert.equal(spec.env.PATH, dirs.path);
  assert.equal(spec.cwd, dirs.project);
  for (const k of ['ANTHROPIC_API_KEY', 'OPENAI_API_KEY', 'PI_CODING_AGENT_SESSION_DIR']) assert.equal(spec.env[k], undefined, k);
  assert.ok(!spec.args.some((a) => a.includes('or-test-123')));
});

test('agent dir is empty except models.json', () => {
  for (const arm of ['baseline', 'codecite']) {
    const dirs = fakeDirs({ codecite: arm === 'codecite' });
    const spec = pi.command({ ...RUN, arm }, dirs, CTX);
    const agentDir = spec.env.PI_CODING_AGENT_DIR;
    assert.deepEqual(readdirSync(agentDir), ['models.json']);
    const models = JSON.parse(readFileSync(join(agentDir, 'models.json'), 'utf8'));
    assert.equal(models.providers.openrouter.headers['X-Title'], `codecite-evals-${arm}`);
    assert.deepEqual(Object.keys(models.providers), ['openrouter']);
    assert.ok(!JSON.stringify(models).includes('or-test-123'));
  }
});

test("the owner's ~/.pi is never referenced", () => {
  const spec = pi.command(RUN, fakeDirs(), CTX);
  for (const v of [...Object.values(spec.env), ...spec.args]) assert.ok(!v.includes('/.pi/agent'), v);
  assert.ok(!Object.values(spec.env).some((v) => v === process.env.HOME));
});

test('missing OpenRouter key throws without a value', () => {
  assert.throws(() => pi.command(RUN, fakeDirs(), { ...CTX, secrets: { openrouter: null, claudeOauth: null } }), /OPENROUTER_EVAL_API_KEY/);
});

test('pi missing from PATH throws', () => {
  const dirs = fakeDirs();
  dirs.path = dirs.xdg;
  assert.throws(() => pi.command(RUN, dirs, CTX), /`pi` not found/);
});
