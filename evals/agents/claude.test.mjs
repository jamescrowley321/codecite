import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import claude, { configLeak, createClaude } from './claude.mjs';
import { classify } from '../lib/guards.mjs';

const fixture = (name) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8');
const STREAM = fixture('claude-stream.jsonl');
const TOOLS = fixture('claude-tools.jsonl');
const LEAKED = fixture('claude-leaked.jsonl');
const AUTH_FAILED = fixture('claude-auth-failed.jsonl');
const USAGE_LIMIT = fixture('claude-usage-limit.jsonl');

const line = (obj) => JSON.stringify(obj);
const assistant = (...content) => line({ type: 'assistant', message: { id: `msg_${Math.random()}`, content } });
const bash = (command) => ({ type: 'tool_use', id: `t${Math.random()}`, name: 'Bash', input: { command } });

/** A fake run with a stub `claude` on its PATH, as prepareRun would lay it out. */
function fakeDirs({ codecite = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'claude-adapter-'));
  const sys = join(root, 'sysbin');
  mkdirSync(sys);
  writeFileSync(join(sys, 'claude'), '#!/bin/sh\n');
  chmodSync(join(sys, 'claude'), 0o755);
  const bin = codecite ? join(root, 'bin-codecite') : null;
  if (bin) {
    mkdirSync(bin);
    writeFileSync(join(bin, 'codecite'), '#!/bin/sh\n');
    chmodSync(join(bin, 'codecite'), 0o755);
  }
  const dirs = { out: root, project: join(root, 'project'), home: join(root, 'home'), xdg: join(root, 'xdg'), bin };
  dirs.path = bin ? [bin, sys].join(delimiter) : sys;
  return dirs;
}

const RUN = { id: 'r1', agent: 'claude', model: 'claude-sonnet-5-5', arm: 'baseline', rep: 1, seq: 1 };
const CTX = { prompt: 'Fix the bug.', secrets: { openrouter: null, claudeOauth: 'tok-test-123' }, plan: {}, round: {} };

test('parses usage from the result event', () => {
  const r = claude.parse(STREAM, '');
  assert.deepEqual(r.usage, {
    input: 2,
    output: 4,
    cacheRead: 11228,
    cacheWrite: 5623,
    totalTokens: 2 + 4 + 11228 + 5623,
    costUsd: 0.0236588,
  });
  assert.equal(r.turns, 1);
  assert.equal(r.toolCalls, 0);
  assert.equal(r.codeciteCalls, 0);
  assert.equal(r.providerStatus, null);
  assert.equal(r.rejectReason, null);
  assert.equal(r.init.model, 'claude-sonnet-5-5');
  assert.match(r.transcript, /ok/);
});

test('a missing result event (a run killed at timeout) sums the per-message usage', () => {
  const truncated = STREAM.trim().split('\n').filter((l) => JSON.parse(l).type !== 'result').join('\n');
  const r = claude.parse(truncated, '');
  assert.deepEqual(r.usage, { input: 2, output: 4, cacheRead: 11228, cacheWrite: 5623, totalTokens: 2 + 4 + 11228 + 5623, costUsd: null });
  assert.equal(r.turns, null);
  assert.equal(r.init.model, 'claude-sonnet-5-5');
});

test('per-message usage counts each message once, though it repeats per content block', () => {
  const events = TOOLS.trim().split('\n').map((l) => JSON.parse(l));
  const truncated = events.filter((e) => e.type !== 'result');
  const seen = new Map();
  for (const e of truncated) if (e.type === 'assistant') seen.set(e.message.id, e.message.usage);
  assert.ok(seen.size < truncated.filter((e) => e.type === 'assistant').length, 'the fixture repeats a message id');
  const want = [...seen.values()].reduce((a, u) => a + u.input_tokens + u.output_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens, 0);
  const r = claude.parse(truncated.map(line).join('\n'), '');
  assert.equal(r.usage.totalTokens, want);
  assert.equal(r.usage.costUsd, null, 'cost is only in the result event');
});

test('no result and no message usage leaves the tokens unknown', () => {
  const r = claude.parse(STREAM.trim().split('\n').filter((l) => !['result', 'assistant'].includes(JSON.parse(l).type)).join('\n'), '');
  assert.equal(r.usage.totalTokens, null);
});

test('a missing token field makes totalTokens null, not a partial sum', () => {
  const lines = STREAM.trim().split('\n').map((l) => JSON.parse(l));
  delete lines.find((e) => e.type === 'result').usage.cache_read_input_tokens;
  const r = claude.parse(lines.map(line).join('\n'), '');
  assert.equal(r.usage.cacheRead, null);
  assert.equal(r.usage.totalTokens, null);
  assert.equal(r.usage.input, 2);
});

test('counts tool calls, shell commands and read paths from a recorded run', () => {
  const r = claude.parse(TOOLS, '');
  assert.equal(r.turns, 5);
  assert.equal(r.toolCalls, 4); // Bash, Read, ToolSearch, Bash
  assert.deepEqual(r.shellCommands, ['echo hi', 'grep -n "hello" /home/user/project/a.txt']);
  assert.deepEqual(r.readPaths, ['/home/user/project/a.txt']);
  assert.equal(r.codeciteCalls, 0);
  assert.equal(r.usage.totalTokens, 6 + 685 + 48350 + 9373);
});

test('readPaths covers every file tool and its path fields', () => {
  const out = [
    assistant(
      { type: 'tool_use', id: 'a', name: 'Edit', input: { file_path: 'src/a.py' } },
      { type: 'tool_use', id: 'b', name: 'Write', input: { file_path: '/abs/b.py' } },
      { type: 'tool_use', id: 'c', name: 'Grep', input: { pattern: 'x', path: 'lib' } },
      { type: 'tool_use', id: 'd', name: 'Glob', input: { pattern: '**/*.py', path: '/abs/dir' } },
      { type: 'tool_use', id: 'e', name: 'NotebookEdit', input: { notebook_path: 'n.ipynb' } },
      { type: 'tool_use', id: 'f', name: 'Glob', input: { pattern: '*.md' } },
    ),
  ].join('\n');
  assert.deepEqual(claude.parse(out, '').readPaths, ['src/a.py', '/abs/b.py', 'lib', '/abs/dir', 'n.ipynb']);
});

test('codeciteCalls counts Bash calls that run codecite in any segment', () => {
  const out = [
    assistant(bash('codecite search "token endpoint"')),
    assistant(bash('cd /x && codecite show foo.py:12')),
    assistant(bash('FOO=1 env codecite corpora | head')),
    assistant(bash('/round/bin-codecite/codecite search a; codecite search b')),
    assistant(bash('echo codecite')),
    assistant(bash('grep -r codecite .')),
    assistant(bash('codecitex search')),
  ].join('\n');
  const r = claude.parse(out, '');
  assert.equal(r.codeciteCalls, 4);
  assert.equal(r.toolCalls, 7);
  assert.equal(r.shellCommands.length, 7);
});

test('rejects a run with user MCP servers', () => {
  const r = claude.parse(LEAKED, '');
  assert.equal(r.rejectReason, 'config leaked');
  assert.ok(r.init.mcp_servers.length > 0);
});

test('configLeak flags MCP servers, user plugins, plugin skills, MCP tools and hooks', () => {
  const clean = STREAM.trim().split('\n').map((l) => JSON.parse(l)).find((e) => e.subtype === 'init');
  assert.equal(configLeak(clean), null, 'the recorded isolated init is clean (builtin plugins only)');
  assert.equal(configLeak({ ...clean, mcp_servers: [{ name: 'x', status: 'connected' }] }), 'config leaked');
  assert.equal(configLeak({ ...clean, plugins: [...clean.plugins, { name: 'p', path: '/home/user/.claude/plugins/p' }] }), 'config leaked');
  assert.equal(configLeak({ ...clean, skills: [...clean.skills, 'superpowers:brainstorming'] }), 'config leaked');
  assert.equal(configLeak({ ...clean, slash_commands: [...clean.slash_commands, 'p:cmd'] }), 'config leaked');
  assert.equal(configLeak({ ...clean, tools: [...clean.tools, 'mcp__x__y'] }), 'config leaked');
  assert.equal(configLeak({ type: 'system', subtype: 'hook_started', hook_name: 'SessionStart:startup' }), 'config leaked');
  assert.equal(configLeak({ type: 'assistant' }), null);
});

test('earlyReject stops at the first leaked line and ignores the rest', () => {
  const lines = LEAKED.trim().split('\n');
  assert.equal(claude.earlyReject(lines[0]), 'config leaked', 'hook event from the owner settings');
  assert.equal(claude.earlyReject(lines[1]), 'config leaked', 'init with MCP servers');
  for (const l of STREAM.trim().split('\n')) assert.equal(claude.earlyReject(l), null);
  assert.equal(claude.earlyReject('not json'), null);
  assert.equal(claude.earlyReject(''), null);
});

test('a 401 surfaces as providerStatus with no turns', () => {
  const r = claude.parse(AUTH_FAILED, '');
  assert.equal(r.providerStatus, 401);
  assert.equal(r.turns, 0);
  assert.equal(r.rejectReason, null);
});

test('a 429 before any turn ends on a provider error, so it is infra, not a failed run', () => {
  const r = claude.parse(USAGE_LIMIT, '');
  assert.equal(r.providerStatus, 429);
  assert.equal(r.turns, 0);
  assert.equal(r.endedOnError, true);
  assert.equal(classify({ ...r, spawnError: null }), 'infra');
});

test('a run that ended on an error after real turns is endedOnError but not infra', () => {
  const events = TOOLS.trim().split('\n').map((l) => JSON.parse(l));
  const res = events.find((e) => e.type === 'result');
  Object.assign(res, { is_error: true, api_error_status: 429 });
  const r = claude.parse(events.map(line).join('\n'), '');
  assert.equal(r.endedOnError, true);
  assert.equal(r.turns, 5);
  assert.equal(classify({ ...r, spawnError: null }), 'agent');
  assert.equal(claude.parse(TOOLS, '').endedOnError, false);
});

test('a retried status that later succeeds is not a provider error', () => {
  const out = [
    line({ type: 'system', subtype: 'api_retry', error_status: 529, error: 'overloaded' }),
    STREAM.trim(),
  ].join('\n');
  assert.equal(claude.parse(out, '').providerStatus, null);
});

test('a run that dies on retries without a result keeps the last status', () => {
  const out = [line({ type: 'system', subtype: 'api_retry', error_status: 503, error: 'x' })].join('\n');
  assert.equal(claude.parse(out, '').providerStatus, 503);
});

test('command runs claude -p with the isolation flags in the project dir', () => {
  const dirs = fakeDirs();
  const c = claude.command(RUN, dirs, CTX);
  assert.equal(c.cmd, join(dirs.path, 'claude'));
  assert.equal(c.cwd, dirs.project);
  assert.deepEqual(c.args, [
    '-p',
    'Fix the bug.',
    '--model',
    'claude-sonnet-5-5',
    '--output-format',
    'stream-json',
    '--verbose',
    '--strict-mcp-config',
    '--mcp-config',
    '{"mcpServers":{}}',
    '--setting-sources',
    '',
    '--disallowedTools',
    'WebFetch,WebSearch',
    '--dangerously-skip-permissions',
    '--no-session-persistence',
  ]);
  assert.equal(c.env.HOME, dirs.home);
  assert.equal(c.env.CLAUDE_CONFIG_DIR, dirs.home);
  assert.equal(c.env.XDG_DATA_HOME, dirs.xdg);
  assert.equal(c.env.PATH, dirs.path);
  assert.equal(c.env.CLAUDE_CODE_OAUTH_TOKEN, 'tok-test-123');
  assert.ok(!c.args.some((a) => a.includes('tok-test-123')), 'the token never goes in args');
});

test('command never sets ANTHROPIC_API_KEY', () => {
  const saved = process.env.ANTHROPIC_API_KEY;
  process.env.ANTHROPIC_API_KEY = 'sk-should-not-leak';
  try {
    const c = claude.command(RUN, fakeDirs(), CTX);
    assert.ok(!('ANTHROPIC_API_KEY' in c.env));
    assert.ok(!Object.values(c.env).includes('sk-should-not-leak'));
    assert.ok(!('ANTHROPIC_AUTH_TOKEN' in c.env));
  } finally {
    if (saved === undefined) delete process.env.ANTHROPIC_API_KEY;
    else process.env.ANTHROPIC_API_KEY = saved;
  }
});

const NO_ENV_TOKEN = { ...CTX, secrets: { openrouter: 'x', claudeOauth: null } };

/** A credentials.json at a temp path; `body` is the parsed JSON or raw text. */
function credentialsFile(body) {
  const p = join(mkdtempSync(join(tmpdir(), 'claude-creds-')), '.credentials.json');
  writeFileSync(p, typeof body === 'string' ? body : JSON.stringify(body), { mode: 0o600 });
  return p;
}

test('uses evals.env token when present', () => {
  const p = credentialsFile({ claudeAiOauth: { accessToken: 'tok-from-login-999' } });
  const c = createClaude({ credentialsPath: p }).command(RUN, fakeDirs(), CTX);
  assert.equal(c.env.CLAUDE_CODE_OAUTH_TOKEN, 'tok-test-123');
  assert.deepEqual(c.secrets, ['tok-test-123']);
});

test('falls back to credentials.json accessToken', () => {
  const p = credentialsFile({ claudeAiOauth: { accessToken: 'tok-from-login-999', refreshToken: 'tok-refresh-888' } });
  const before = readFileSync(p, 'utf8');
  const adapter = createClaude({ credentialsPath: p });
  const c = adapter.command(RUN, fakeDirs(), NO_ENV_TOKEN);
  assert.equal(c.env.CLAUDE_CODE_OAUTH_TOKEN, 'tok-from-login-999');
  assert.deepEqual(c.secrets, ['tok-from-login-999', 'tok-refresh-888'], 'the harness must redact the tokens it never saw, the refresh token too');
  assert.ok(!Object.values(c.env).includes('tok-refresh-888'), 'the refresh token is never passed');
  assert.ok(!c.args.some((a) => a.includes('tok-from-login-999')));
  assert.equal(readFileSync(p, 'utf8'), before, 'the login file is read-only');
  // Read fresh on every call: a refreshed login is picked up mid-round.
  writeFileSync(p, JSON.stringify({ claudeAiOauth: { accessToken: 'tok-refreshed-777' } }));
  assert.equal(adapter.command(RUN, fakeDirs(), NO_ENV_TOKEN).env.CLAUDE_CODE_OAUTH_TOKEN, 'tok-refreshed-777');
});

test('error names the source, never the value', () => {
  const missing = join(mkdtempSync(join(tmpdir(), 'claude-creds-')), '.credentials.json');
  const cases = [
    [missing, /CLAUDE_CODE_OAUTH_TOKEN.*\.credentials\.json.*missing/s],
    [credentialsFile('{"claudeAiOauth": {"accessToken": "tok-secret-in-bad-json'), /not valid JSON/],
    [credentialsFile({ claudeAiOauth: { refreshToken: 'tok-secret-refresh-555' } }), /claudeAiOauth\.accessToken/],
    [credentialsFile({ claudeAiOauth: { accessToken: '' } }), /claudeAiOauth\.accessToken/],
  ];
  for (const [p, re] of cases) {
    let err;
    try {
      createClaude({ credentialsPath: p }).command(RUN, fakeDirs(), NO_ENV_TOKEN);
    } catch (e) {
      err = e;
    }
    assert.ok(err, `${p} should throw`);
    assert.match(err.message, re);
    assert.ok(err.message.includes(p), 'names the file');
    assert.ok(!/tok-secret/.test(err.message), `no value in: ${err.message}`);
  }
});

test('an expired or soon-expiring login stops the round with a clear message, never the token', () => {
  const plan = { timeoutSec: 600 };
  const ctx = { ...NO_ENV_TOKEN, plan };
  const at = (ms) => credentialsFile({ claudeAiOauth: { accessToken: 'tok-secret-exp-1', refreshToken: 'tok-secret-exp-2', expiresAt: Date.now() + ms } });
  for (const p of [at(-60_000), at(5 * 60_000)]) {
    let err;
    try {
      createClaude({ credentialsPath: p }).command(RUN, fakeDirs(), ctx);
    } catch (e) {
      err = e;
    }
    assert.ok(err, 'throws');
    assert.match(err.message, /log in to Claude Code/);
    assert.ok(!/tok-secret/.test(err.message), err.message);
  }
  // Valid for longer than a run's timeout: fine.
  assert.equal(createClaude({ credentialsPath: at(60 * 60_000) }).command(RUN, fakeDirs(), ctx).env.CLAUDE_CODE_OAUTH_TOKEN, 'tok-secret-exp-1');
  // No expiresAt, or a token from evals.env: not checked.
  assert.ok(createClaude({ credentialsPath: credentialsFile({ claudeAiOauth: { accessToken: 'a' } }) }).command(RUN, fakeDirs(), ctx));
  assert.ok(createClaude({ credentialsPath: at(-60_000) }).command(RUN, fakeDirs(), { ...CTX, plan }));
});

test('command fails clearly when claude is not on the run PATH', () => {
  const dirs = { ...fakeDirs(), path: '/nonexistent-dir' };
  assert.throws(() => claude.command(RUN, dirs, CTX), /claude.*not found on the run's PATH/);
});

test('baseline PATH lacks codecite', () => {
  const base = claude.command(RUN, fakeDirs(), CTX);
  const arm = claude.command({ ...RUN, arm: 'codecite' }, fakeDirs({ codecite: true }), CTX);
  assert.ok(!base.env.PATH.split(delimiter).some((d) => d.endsWith('bin-codecite')));
  assert.ok(arm.env.PATH.split(delimiter)[0].endsWith('bin-codecite'));
  // The arms differ only in PATH (and per-run dirs), never in flags.
  assert.deepEqual(base.args, arm.args);
  assert.deepEqual(Object.keys(base.env).sort(), Object.keys(arm.env).sort());
});
