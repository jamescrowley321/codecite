import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createServer } from 'node:net';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { dirname, join, relative } from 'node:path';
import { DAEMON_SOCKETS, findBwrap, hiddenRoots, opensslFiles, sandboxArgs } from './sandbox.mjs';

const BWRAP = findBwrap(process.env.PATH);
// The defaults minus /tmp: these tests' dirs live under /tmp, so with it
// hidden every planted path would be hidden anyway and prove nothing.
const NOT_TMP = ['/home', '/root', '/var/tmp', '/run'];
const skip = BWRAP ? false : 'bwrap is not on PATH; install bubblewrap to run the sandbox tests';

let base, home, evalsDir, round, dirs, opts;

before(() => {
  base = mkdtempSync(join(tmpdir(), 'evals-sandbox-'));
  // A planted owner home holding every secret the real one does.
  home = join(base, 'owner');
  const plant = (rel, text) => {
    const p = join(home, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, text);
    return p;
  };
  plant('.claude/.credentials.json', '{"claudeAiOauth":{"accessToken":"planted"}}');
  plant('.config/codecite/evals.env', 'OPENROUTER_EVAL_API_KEY=planted\n');
  plant('.pi/agent/auth.json', '{"planted":true}');
  plant('repos/codecite-worktrees/x/evals/tasks/t/hidden/test_hidden.py', 'assert True\n');
  plant('.local/share/other/secret.txt', 'planted\n');

  // An agent install under the owner's home, reached through a symlink, as
  // claude's is: ~/.local/bin/agent -> ../share/agent/v1/agent.sh.
  const install = join(home, '.local', 'share', 'agent', 'v1');
  mkdirSync(install, { recursive: true });
  writeFileSync(join(install, 'agent.sh'), '#!/bin/sh\necho agent-ran "$@"\n');
  chmodSync(join(install, 'agent.sh'), 0o755);
  mkdirSync(join(home, '.local', 'bin'), { recursive: true });
  symlinkSync('../share/agent/v1/agent.sh', join(home, '.local', 'bin', 'agent'));

  // A node package reached through a bin symlink with an `env node` shebang,
  // as pi's is.
  const pkg = join(home, '.local', 'lib', 'node_modules', 'nodeagent');
  mkdirSync(join(pkg, 'dist'), { recursive: true });
  mkdirSync(join(pkg, 'lib'), { recursive: true });
  writeFileSync(join(pkg, 'package.json'), '{"name":"nodeagent","type":"module"}\n');
  writeFileSync(join(pkg, 'lib', 'msg.js'), 'export const msg = "nodeagent-ran";\n');
  writeFileSync(join(pkg, 'dist', 'cli.js'), '#!/usr/bin/env node\nimport { msg } from "../lib/msg.js";\nconsole.log(msg);\n');
  chmodSync(join(pkg, 'dist', 'cli.js'), 0o755);
  symlinkSync('../lib/node_modules/nodeagent/dist/cli.js', join(home, '.local', 'bin', 'nodeagent'));

  // A round with this run, a sibling run and the round's records.
  evalsDir = join(home, '.local', 'state', 'codecite-evals');
  round = join(evalsDir, 'round-1');
  writeFileSync(join(mkdirp(round), 'plan.json'), '{"planted":true}\n');
  writeFileSync(join(mkdirp(join(round, 'runs', 'sibling', 'project')), 'answer.py'), 'planted\n');
  const out = join(round, 'runs', 'me');
  dirs = { out, project: mkdirp(join(out, 'project')), home: mkdirp(join(out, 'home')), xdg: mkdirp(join(out, 'xdg')), bin: null };
  writeFileSync(join(dirs.project, 'main.py'), 'app = None\n');
  writeFileSync(join(base, 'host-tmp-marker'), 'planted\n');
  opts = { bwrap: BWRAP, home, evalsDir };
});
after(() => rmSync(base, { recursive: true, force: true }));

function mkdirp(d) {
  mkdirSync(d, { recursive: true });
  return d;
}

function spec(cmd, args, env = {}) {
  return { cmd, args, env: { PATH: '/usr/bin:/bin', HOME: dirs.home, ...env }, cwd: dirs.project };
}

function inSandbox(s, o = opts) {
  const sb = sandboxArgs(s, dirs, o);
  assert.equal(sb.cmd, BWRAP);
  return spawnSync(sb.cmd, sb.args, { encoding: 'utf8', env: sb.env });
}

const sh = (script, env) => inSandbox(spec('/bin/sh', ['-c', script], env));

/** The paths of `paths` the sandbox can read (exit codes only; nothing is printed). */
function readable(paths) {
  const script = 'for p in "$@"; do if test -r "$p"; then echo "$p"; fi; done; true';
  const out = inSandbox(spec('/bin/sh', ['-c', script, 'probe', ...paths]));
  assert.equal(out.status, 0, out.stderr);
  return out.stdout.split('\n').filter(Boolean);
}

test("the owner's credentials are unreadable", { skip }, () => {
  const planted = ['.claude/.credentials.json', '.config/codecite/evals.env', '.pi', '.pi/agent/auth.json'].map((p) => join(home, p));
  for (const p of planted) assert.ok(existsSync(p), p);
  assert.deepEqual(readable(planted), []);
  // The real paths too, checked by `test -r` only: never read, never printed.
  const real = ['.claude/.credentials.json', '.config/codecite/evals.env', '.pi'].map((p) => join(homedir(), p)).filter((p) => existsSync(p));
  assert.deepEqual(readable(real), []);
});

test('sibling runs and the round dir are hidden', { skip }, () => {
  const hidden = [
    join(round, 'plan.json'),
    join(round, 'runs', 'sibling', 'project', 'answer.py'),
    join(home, 'repos', 'codecite-worktrees', 'x', 'evals', 'tasks', 't', 'hidden', 'test_hidden.py'),
    join(home, '.local', 'share', 'other', 'secret.txt'),
  ];
  assert.deepEqual(readable(hidden), []);
  const r = sh(`cat main.py && echo new > made.py && echo h > "${dirs.home}/h" && echo x > "${dirs.xdg}/x" && ls -A ..`);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /app = None/);
  assert.equal(readFileSync(join(dirs.project, 'made.py'), 'utf8'), 'new\n', 'the project is read-write');
  assert.ok(existsSync(join(dirs.home, 'h')) && existsSync(join(dirs.xdg, 'x')), 'home and xdg are read-write');
  // The run dir holds only what is bound back: no stdout.jsonl, setup.log, ...
  assert.deepEqual(r.stdout.split('\n').slice(1).filter(Boolean).sort(), ['home', 'project', 'xdg']);
});

test("the env is exactly the adapter's env", { skip }, () => {
  process.env.LEAK_SENTINEL = '1';
  let r;
  try {
    r = inSandbox(spec('/usr/bin/env', [], { FOO: 'bar baz' }));
  } finally {
    delete process.env.LEAK_SENTINEL;
  }
  assert.equal(r.status, 0, r.stderr);
  const lines = r.stdout.split('\n').filter(Boolean);
  // bwrap's --chdir always exports PWD, the cwd: the one key not the adapter's.
  assert.ok(lines.includes(`PWD=${dirs.project}`), 'PWD is the cwd');
  const keys = lines.map((l) => l.split('=')[0]).filter((k) => k !== 'PWD').sort();
  assert.deepEqual(keys, ['FOO', 'HOME', 'PATH']);
  assert.match(r.stdout, /^FOO=bar baz$/m);
});

test('/tmp is private and empty', { skip }, () => {
  const r = sh('ls -A /tmp; echo mine > /tmp/inside-only; cat /tmp/inside-only');
  assert.equal(r.status, 0, r.stderr);
  const lines = r.stdout.split('\n').filter(Boolean);
  assert.equal(lines.pop(), 'mine', '/tmp is writable');
  // The only entry is the skeleton leading to this run's own dirs, when the
  // test's base dir is itself under /tmp.
  const top = relative('/tmp', base).split('/')[0];
  assert.deepEqual(lines, base.startsWith('/tmp/') ? [top] : []);
  assert.equal(existsSync('/tmp/inside-only'), false, "the sandbox's /tmp is not the host's");
  assert.deepEqual(readable([join(base, 'host-tmp-marker')]), []);
});

test("the agent's install is bound through its symlinks; the rest of the owner's home stays hidden", { skip }, () => {
  const r = inSandbox(spec(join(home, '.local', 'bin', 'agent'), ['x']));
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout, 'agent-ran x\n');
  assert.deepEqual(readable([join(home, '.local', 'share', 'other', 'secret.txt'), join(home, '.pi')]), []);
});

test('a node script gets its package and the node runtime from its shebang', { skip }, () => {
  const path = [join(home, '.local', 'bin'), dirname(process.execPath), '/usr/bin', '/bin'].join(':');
  const r = inSandbox(spec(join(home, '.local', 'bin', 'nodeagent'), [], { PATH: path }));
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout, 'nodeagent-ran\n');
});

test('extra read-only binds are readable but not writable', { skip }, () => {
  const ro = mkdirp(join(base, 'export'));
  writeFileSync(join(ro, 'lib.py'), 'x = 1\n');
  const sb = sandboxArgs(spec('/bin/sh', ['-c', `cat "${ro}/lib.py"; echo y > "${ro}/new.py"`]), dirs, { ...opts, ro: [ro] });
  const r = spawnSync(sb.cmd, sb.args, { encoding: 'utf8', env: sb.env });
  assert.match(r.stdout, /x = 1/);
  assert.equal(existsSync(join(ro, 'new.py')), false);
});

test('the codecite bin dir is bound read-only only when the arm has one', { skip }, () => {
  const bin = mkdirp(join(round, 'bin-codecite'));
  writeFileSync(join(bin, 'codecite'), '#!/bin/sh\necho codecite-ran\n');
  chmodSync(join(bin, 'codecite'), 0o755);
  const withBin = sandboxArgs(spec('/bin/sh', ['-c', 'codecite']), { ...dirs, bin }, opts);
  const r = spawnSync(withBin.cmd, withBin.args, { encoding: 'utf8', env: withBin.env });
  assert.equal(r.stdout, '', 'not on PATH unless the adapter puts it there');
  const onPath = sandboxArgs(spec('/bin/sh', ['-c', 'codecite'], { PATH: `${bin}:/usr/bin:/bin` }), { ...dirs, bin }, opts);
  assert.equal(spawnSync(onPath.cmd, onPath.args, { encoding: 'utf8', env: onPath.env }).stdout, 'codecite-ran\n');
  const baseline = sandboxArgs(spec('/bin/sh', ['-c', 'codecite'], { PATH: `${bin}:/usr/bin:/bin` }), dirs, opts);
  assert.notEqual(spawnSync(baseline.cmd, baseline.args, { encoding: 'utf8', env: baseline.env }).status, 0);
  // Apart from the bin bind, the two arms' sandboxes are identical.
  const strip = (args) => args.join('\0').replace(`\0--ro-bind\0${bin}\0${bin}`, '').split('\0');
  assert.deepEqual(strip(onPath.args), baseline.args);
});

test('the sandbox args carry the isolation flags and set no unshared network', () => {
  const sb = sandboxArgs(spec('/bin/true', []), dirs, { ...opts, bwrap: '/usr/bin/bwrap' });
  for (const f of ['--die-with-parent', '--unshare-pid', '--unshare-ipc', '--unshare-uts', '--new-session']) {
    assert.ok(sb.args.includes(f), f);
  }
  assert.equal(sb.args.includes('--unshare-net'), false);
  assert.equal(sb.args.includes('--unshare-all'), false);
  assert.deepEqual(sb.args.slice(0, 3), ['--ro-bind', '/', '/']);
  const i = sb.args.indexOf('--chdir');
  assert.equal(sb.args[i + 1], dirs.project);
  // The command and its args come last, after `--`.
  assert.deepEqual(sb.args.slice(-2), ['--', '/bin/true']);
});

test("no env value reaches bwrap's argv, which any local user can read", () => {
  const SECRET = 'sk-or-v1-PLANTEDSECRET0123';
  const sb = sandboxArgs(spec('/bin/true', [], { OPENROUTER_API_KEY: SECRET }), dirs, { ...opts, bwrap: '/usr/bin/bwrap' });
  assert.equal(sb.args.some((a) => a.includes(SECRET)), false);
  assert.equal(sb.args.includes('--setenv'), false);
  assert.deepEqual(sb.env, { PATH: '/usr/bin:/bin', HOME: dirs.home, OPENROUTER_API_KEY: SECRET });
});

test('hiddenRoots hides $HOME and the evals dir when they are outside /home', () => {
  const roots = hiddenRoots({ home: '/srv/owner', evalsDir: '/data/evals', hide: ['/home/x/repo', '/opt/repo'], exists: () => true, realpath: (p) => p });
  for (const r of ['/home', '/root', '/tmp', '/var/tmp', '/run', '/srv/owner', '/data/evals', '/opt/repo']) assert.ok(roots.includes(r), r);
  assert.equal(roots.includes('/home/x/repo'), false, 'already under /home');
  const id = (p) => p;
  const under = hiddenRoots({ home: '/home/owner', evalsDir: '/home/owner/.local/state/codecite-evals', exists: () => true, realpath: id });
  assert.deepEqual(under.sort(), ['/home', '/root', '/run', '/tmp', '/var/tmp']);
  assert.equal(hiddenRoots({ home: '/home/o', evalsDir: '/home/o/e', exists: (p) => p !== '/var/tmp', realpath: id }).includes('/var/tmp'), false);
});

test("a libcrypto's OpenSSL dir contributes its config and CA files, not its private dir", () => {
  const lib = join(base, 'libcrypto.so.3');
  writeFileSync(lib, Buffer.concat([Buffer.from([0x7f, 0x45, 0x4c, 0x46, 0, 1]), Buffer.from('xx OPENSSLDIR: "/opt/brew/etc/openssl@3" yy\0')]));
  assert.deepEqual(opensslFiles(lib), ['/opt/brew/etc/openssl@3/openssl.cnf', '/opt/brew/etc/openssl@3/cert.pem', '/opt/brew/etc/openssl@3/certs']);
  const other = join(base, 'libz.so.1');
  writeFileSync(other, 'no dir here');
  assert.deepEqual(opensslFiles(other), []);
});

/** A fresh owner home reached through a symlink, and run dirs beside it, all outside NOT_TMP. */
function symlinkedHome(name) {
  const root = mkdirp(join(base, name));
  const realHome = mkdirp(join(root, 'real-home'));
  mkdirp(join(realHome, '.claude'));
  writeFileSync(join(realHome, '.claude', '.credentials.json'), '{"planted":true}');
  const link = join(root, 'home-link');
  symlinkSync(realHome, link);
  const run = { project: mkdirp(join(root, 'run', 'project')), home: mkdirp(join(root, 'run', 'home')), xdg: mkdirp(join(root, 'run', 'xdg')), bin: null };
  return { root, realHome, link, run };
}

function probe(paths, runDirs, o) {
  const script = 'for p in "$@"; do if test -r "$p"; then echo "$p"; fi; done; true';
  const s = { cmd: '/bin/sh', args: ['-c', script, 'probe', ...paths], env: { PATH: '/usr/bin:/bin' }, cwd: runDirs.project };
  const sb = sandboxArgs(s, runDirs, o);
  const r = spawnSync(sb.cmd, sb.args, { encoding: 'utf8', env: sb.env });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout.split('\n').filter(Boolean);
}

test("a symlinked $HOME hides its target, not just the link", { skip }, () => {
  const h = symlinkedHome('symhome');
  const o = { bwrap: BWRAP, home: h.link, evalsDir: join(h.link, 'state'), alwaysHidden: NOT_TMP };
  const creds = join(h.realHome, '.claude', '.credentials.json');
  assert.deepEqual(probe([creds, join(h.link, '.claude', '.credentials.json')], h.run, o), []);
  const roots = hiddenRoots({ home: h.link, evalsDir: join(h.link, 'state'), alwaysHidden: NOT_TMP });
  assert.ok(roots.includes(h.realHome), 'the real path is a hidden root');
});

test('a symlinked hide dir (round dir, repo) hides its target', { skip }, () => {
  const h = symlinkedHome('symhide');
  const realRound = mkdirp(join(h.root, 'real-round'));
  writeFileSync(join(realRound, 'plan.json'), '{}');
  symlinkSync(realRound, join(h.root, 'round-link'));
  const o = { bwrap: BWRAP, home: h.link, evalsDir: join(h.link, 'state'), alwaysHidden: NOT_TMP, hide: [join(h.root, 'round-link')] };
  assert.deepEqual(probe([join(realRound, 'plan.json')], h.run, o), []);
});

test('hiddenRoots adds the real path of every symlinked root', () => {
  const real = { '/home': '/var/home', '/data/evals': '/data/evals' };
  const roots = hiddenRoots({ home: '/home/o', evalsDir: '/data/evals', exists: () => true, realpath: (p) => real[p] ?? p });
  assert.ok(roots.includes('/home') && roots.includes('/var/home'), roots.join(' '));
});

test('daemon sockets outside the hidden roots are covered', { skip }, async () => {
  const h = symlinkedHome('sockets');
  const sock = join(h.root, 'docker.sock');
  const server = createServer();
  await new Promise((res) => server.listen(sock, res));
  try {
    const o = { bwrap: BWRAP, home: h.link, evalsDir: join(h.link, 'state'), alwaysHidden: NOT_TMP, daemonSockets: [sock, join(h.root, 'absent.sock')] };
    const s = { cmd: '/bin/sh', args: ['-c', `if test -S "${sock}"; then echo socket; else echo covered; fi`], env: { PATH: '/usr/bin:/bin' }, cwd: h.run.project };
    const sb = sandboxArgs(s, h.run, o);
    const r = spawnSync(sb.cmd, sb.args, { encoding: 'utf8', env: sb.env });
    assert.equal(r.stdout.trim(), 'covered', r.stderr);
    assert.equal(sb.args.join(' ').includes('absent.sock'), false, 'a socket that does not exist is skipped');
  } finally {
    server.close();
  }
});

test('/run is hidden except the resolver config', { skip }, () => {
  const r = sh('test -r /etc/resolv.conf && echo resolv-ok; for p in /run/dbus/system_bus_socket /run/snapd.socket /run/systemd/private /var/run/docker.sock; do test -e "$p" && echo "VISIBLE $p"; done; true');
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /resolv-ok/);
  assert.doesNotMatch(r.stdout, /VISIBLE/);
});

test('the default daemon socket list names the container daemons', () => {
  for (const p of ['/run/docker.sock', '/var/run/docker.sock', '/run/containerd/containerd.sock', '/run/podman/podman.sock', '/var/snap/lxd/common/lxd/unix.socket', '/var/lib/lxd/unix.socket', '/run/incus/unix.socket']) {
    assert.ok(DAEMON_SOCKETS.includes(p), p);
  }
});
