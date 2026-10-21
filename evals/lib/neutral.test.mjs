// Nothing an agent can see names its arm, its run, its round or the evals
// state dir: both arms get the same cwd, env, prompt (but for the codecite
// note) and in-sandbox paths, and differ only by the codecite bin dir on PATH.

import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import claude from '../agents/claude.mjs';
import pi from '../agents/pi.mjs';
import { renderPrompt } from './plan.mjs';
import { findBwrap, roundSandbox, spawnSandbox } from './sandbox.mjs';
import { prepareRun } from './workspace.mjs';

const BWRAP = findBwrap(process.env.PATH);
const ROUND_ID = 'round-zq7';
const NOTE = 'The `codecite` command is on your PATH. Its corpus `eval` indexes the source at `{identity_model}`.';
let tmp, round, sysbin;
let rep = 0;

before(() => {
  tmp = mkdtempSync(join(tmpdir(), 'evals-neutral-'));
  // Laid out like a real round: <state>/codecite-evals/<round>/...
  const dir = join(tmp, 'codecite-evals', ROUND_ID);
  const taskDir = join(dir, 'task');
  mkdirSync(join(taskDir, 'project', 'app'), { recursive: true });
  writeFileSync(join(taskDir, 'project', 'app', 'main.py'), 'app = None\n');
  sysbin = join(tmp, 'sysbin');
  mkdirSync(sysbin);
  for (const name of ['claude', 'pi']) {
    writeFileSync(join(sysbin, name), '#!/bin/sh\n');
    chmodSync(join(sysbin, name), 0o755);
  }
  const exportDir = join(dir, 'identity-model');
  const xdg = join(dir, 'xdg');
  const binCodecite = join(dir, 'bin-codecite');
  for (const d of [exportDir, join(xdg, 'codecite'), binCodecite]) mkdirSync(d, { recursive: true });
  writeFileSync(join(exportDir, 'lib.py'), 'x = 1\n');
  writeFileSync(join(binCodecite, 'codecite'), '#!/bin/sh\n');
  chmodSync(join(binCodecite, 'codecite'), 0o755);
  const plan = {
    roundId: ROUND_ID,
    runs: 1,
    seed: 1,
    timeoutSec: 600,
    agents: [],
    arms: [{ name: 'baseline' }, { name: 'codecite', codecite: true, note: NOTE }],
    taskDef: { id: 'fake', dir: taskDir, project: 'project', setup: [], promptText: 'Add auth. The source is at `{identity_model}`.\n' },
  };
  round = { dir, export: exportDir, xdg, binCodecite, taintPaths: [join(dir, 'plan.json'), join(dir, 'runs')], plan };
});
after(() => rmSync(tmp, { recursive: true, force: true }));

const SECRETS = { openrouter: 'sk-or-v1-NEUTRALTEST', claudeOauth: 'sk-ant-oat01-NEUTRALTEST' };

/** One run of `agent` in `arm`: its dirs, spec, sandbox and everything the agent can see. */
function build(agent, arm) {
  rep++;
  const run = { id: `${agent}-m-${arm}-${rep}`, agent, model: 'm', arm, rep, seq: rep };
  const dirs = prepareRun(run, round, { basePath: [sysbin, '/usr/bin', '/bin'].join(delimiter), bwrap: BWRAP });
  const armDef = round.plan.arms.find((a) => a.name === arm);
  const prompt = renderPrompt(round.plan.taskDef, armDef);
  const adapter = agent === 'claude' ? claude : pi;
  const spec = adapter.command(run, dirs, { prompt, secrets: SECRETS, plan: round.plan, round });
  const sb = roundSandbox(spec, dirs, round, { bwrap: BWRAP ?? '/usr/bin/bwrap' });
  // In-sandbox paths: every bind's and symlink's destination, the tmpfs
  // mounts, the chdir; plus the command line, the env and the files the
  // adapter wrote into the agent's home.
  const dests = [];
  for (let i = 0; i < sb.args.length; i++) {
    const a = sb.args[i];
    if (a === '--') break;
    if (['--bind', '--ro-bind', '--symlink'].includes(a)) dests.push(sb.args[i + 2]);
    if (['--tmpfs', '--chdir'].includes(a)) dests.push(sb.args[i + 1]);
  }
  const files = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else files.push(readFileSync(p, 'utf8'));
    }
  };
  walk(dirs.host.home);
  const command = sb.args.slice(sb.args.indexOf('--') + 1);
  const data = (spec.dataFiles ?? []).map((f) => `${f.path}\n${f.data}`);
  return { run, dirs, spec, sb, prompt, visible: [...dests, ...command, ...Object.values(sb.env), ...files, ...data] };
}

for (const agent of ['claude', 'pi']) {
  test(`${agent}: nothing visible to the agent names its arm, run, round or the evals dir`, () => {
    for (const arm of ['baseline', 'codecite']) {
      const { run, visible } = build(agent, arm);
      const forbidden = ['baseline', 'codecite-evals', ROUND_ID, run.id, round.dir];
      for (const v of visible) {
        // The codecite note necessarily says codecite: it is the one exempt text.
        const text = String(v).replace(NOTE.replace('{identity_model}', '/work/identity-model'), '<note>');
        for (const f of forbidden) assert.ok(!text.includes(f), `${arm}: "${f}" visible in ${JSON.stringify(text).slice(0, 200)}`);
        assert.ok(!/codecite/.test(text) || arm === 'codecite', `baseline sees "codecite" in ${JSON.stringify(text).slice(0, 200)}`);
      }
    }
  });

  test(`${agent}: the arms differ only by the note and the codecite bin dir`, () => {
    const b = build(agent, 'baseline');
    const c = build(agent, 'codecite');
    assert.equal(b.spec.cwd, '/work/project');
    assert.equal(c.spec.cwd, b.spec.cwd);
    const noteText = NOTE.replace('{identity_model}', '/work/identity-model');
    assert.equal(c.prompt, `${b.prompt}\n${noteText}\n`);
    const env = (s) => ({ ...s.sb.env, PATH: s.sb.env.PATH.split(delimiter).filter((p) => p !== '/work/bin').join(delimiter) });
    // pi's X-Title is per arm (opaque); every other value is the same.
    assert.deepEqual(env(c), env(b));
    assert.equal(c.sb.env.PATH, `/work/bin${delimiter}${b.sb.env.PATH}`);
    const strip = (s) => s.sb.args.join('\0').replace(`\0--ro-bind\0${round.binCodecite}\0/work/bin`, '').replace(c.prompt, b.prompt).split('\0');
    // Host sources differ by run dir; compare the in-sandbox side only.
    const dests = (s) => strip(s).filter((a) => !a.startsWith(round.dir));
    assert.deepEqual(dests(c), dests(b));
  });
}

/** Run a sandbox the way the call loop does (binds on a pipe); resolves {code, stdout, stderr}. */
function runIt(sb, cwd) {
  return new Promise((res) => {
    const child = spawnSandbox(sb, { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    const out = [];
    const err = [];
    child.stdout.on('data', (c) => out.push(c));
    child.stderr.on('data', (c) => err.push(c));
    child.on('close', (code) => res({ code, stdout: Buffer.concat(out).toString(), stderr: Buffer.concat(err).toString() }));
  });
}

// What an agent's shell can see of itself and its sandbox. The mount table
// (mountinfo) is left out: it names the host dirs, and reading it taints.
const PROBE = [
  'pwd',
  'env',
  'ls -A /work',
  'ps -eo pid,args',
  'tr "\\0" " " </proc/1/cmdline; echo',
  'for p in /proc/[0-9]*/cmdline; do tr "\\0" " " <"$p"; echo; done',
  'tr "\\0" "\\n" </proc/self/environ',
  'tr "\\0" "\\n" </proc/1/environ',
  'cat /proc/self/mounts',
  'printf "%s" "$1"',
].join('; ');

for (const agent of ['claude', 'pi']) {
  test(`${agent}: inside the sandbox, pwd, env, ps, /proc and the prompt name no arm, run or round`, { skip: BWRAP ? false : 'bwrap is not on PATH' }, async () => {
    for (const arm of ['baseline', 'codecite']) {
      const { run, spec, dirs, prompt } = build(agent, arm);
      // The run's own sandbox and env, with a probe for the agent.
      const probe = { ...spec, cmd: '/bin/sh', args: ['-c', PROBE, 'probe', prompt] };
      const sb = roundSandbox(probe, dirs, round, { bwrap: BWRAP });
      const r = await runIt(sb, dirs.host.project);
      assert.equal(r.code, 0, r.stderr);
      assert.match(r.stdout, /^\/work\/project\n/);
      assert.match(r.stdout, /bwrap --args 3 -- \/bin\/sh/, 'pid 1 is bwrap with its binds on a pipe');
      const seen = r.stdout.replace(NOTE.replace('{identity_model}', '/work/identity-model'), '<note>');
      for (const f of ['baseline', 'codecite-evals', ROUND_ID, run.id, round.dir]) assert.ok(!seen.includes(f), `${arm}: ${f} in ${seen}`);
      const work = seen.split('\n').filter((l) => ['project', 'home', 'xdg', 'bin', 'identity-model'].includes(l)).sort();
      assert.deepEqual(work, arm === 'codecite' ? ['bin', 'home', 'identity-model', 'project', 'xdg'] : ['home', 'identity-model', 'project', 'xdg']);
    }
  });
}
