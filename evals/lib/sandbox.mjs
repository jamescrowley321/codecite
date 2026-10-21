// The bubblewrap sandbox every agent runs in. The host is visible read-only,
// except the places secrets, sockets and other runs live, which are hidden
// under empty tmpfs mounts: /home (and $HOME), /root, /tmp, /var/tmp, /run
// and the evals state dir, each at its path and its real path. Container
// daemon sockets elsewhere are covered with /dev/null. Into that, only this
// run's own dirs are bound back read-write, at the neutral IN_SANDBOX paths
// (/work/project, ...), and the agent's install, the task's tools and the
// codecite bin dir read-only. The environment is exactly the adapter's (plus
// PWD, which bwrap's --chdir sets), handed to bwrap as its own env. Network
// is shared: agents need their provider's API.

import { spawn, spawnSync } from 'node:child_process';
import { accessSync, closeSync, constants, existsSync, lstatSync, openSync, readdirSync, readFileSync, readlinkSync, readSync, realpathSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, delimiter, dirname, isAbsolute, join, relative, resolve } from 'node:path';

/**
 * Where a run's dirs appear inside the sandbox: fixed, neutral paths, the
 * same for every arm, run and round, so nothing the agent sees (its cwd,
 * env, prompt, PATH) names the arm, the run, the round or the evals dir.
 * `bin` (codecite) is bound only in the codecite arm. `hidden`, `score`
 * and `plugin` are scoring's; `uvCache` is the run setup's.
 */
export const IN_SANDBOX = Object.freeze({
  root: '/work',
  project: '/work/project',
  home: '/work/home',
  xdg: '/work/xdg',
  bin: '/work/bin',
  export: '/work/identity-model',
  hidden: '/work/hidden',
  score: '/work/score',
  plugin: '/work/score-plugin',
  uvCache: '/work/uv-cache',
});

/** Hidden in every sandbox, when they exist. */
// /run whole, not just /run/user: it holds the system bus, systemd's private
// socket, snapd's and any container daemon's socket that appears later. The
// resolver config (/etc/resolv.conf -> /run/systemd/resolve/...) is bound
// back; nothing else in /run is needed.
export const ALWAYS_HIDDEN = ['/home', '/root', '/tmp', '/var/tmp', '/run'];

/** Files bound back from under the hidden roots in every sandbox. */
export const ALWAYS_BOUND = ['/etc/resolv.conf'];

/**
 * Container daemon sockets: reaching one is root for a member of the docker,
 * lxd or incus group. Those under /run are hidden with it; any that exists
 * elsewhere is covered with /dev/null.
 */
export const DAEMON_SOCKETS = [
  '/run/docker.sock',
  '/var/run/docker.sock',
  '/run/containerd/containerd.sock',
  '/run/podman/podman.sock',
  '/var/snap/lxd/common/lxd/unix.socket',
  '/var/lib/lxd/unix.socket',
  '/run/incus/unix.socket',
];

/** The evals state dir: $CODECITE_EVALS_DIR, else ~/.local/state/codecite-evals. */
export function evalsStateDir(home = homedir()) {
  return process.env.CODECITE_EVALS_DIR || join(home, '.local', 'state', 'codecite-evals');
}

const isUnder = (p, dir) => {
  const rel = relative(dir, p);
  return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel));
};
const underAny = (p, dirs) => dirs.some((d) => isUnder(p, d));

/** The first executable `name` in the absolute dirs of `path`, else null. */
export function findOnPath(name, path) {
  if (name.includes('/')) return isAbsolute(name) ? name : null;
  for (const dir of (path ?? '').split(delimiter)) {
    if (!dir || !isAbsolute(dir)) continue;
    const p = join(dir, name);
    try {
      accessSync(p, constants.X_OK);
      return p;
    } catch {
      // not here
    }
  }
  return null;
}

/** bwrap on `path`, else null. */
export const findBwrap = (path = process.env.PATH) => findOnPath('bwrap', path);

/**
 * Run the sandbox's namespace setup once with nothing in it. Returns null
 * when it works, else bwrap's error.
 */
export function probeBwrap(bwrap) {
  const r = spawnSync(bwrap, [...NAMESPACES, '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc', '--', '/bin/true'], {
    encoding: 'utf8',
    env: {},
    timeout: 10_000,
  });
  if (r.error) return r.error.message;
  if (r.status !== 0) return (r.stderr || `exited ${r.status ?? r.signal}`).trim();
  return null;
}

const NAMESPACES = ['--die-with-parent', '--unshare-pid', '--unshare-ipc', '--unshare-uts', '--new-session'];

/**
 * The dirs hidden under tmpfs: ALWAYS_HIDDEN plus `home`, `evalsDir` and each
 * of `hide` that isn't already under one, keeping only those that exist.
 */
export function hiddenRoots({
  home = homedir(),
  evalsDir = evalsStateDir(home),
  hide = [],
  exists = existsSync,
  realpath = realpathSync,
  alwaysHidden = ALWAYS_HIDDEN,
} = {}) {
  const roots = [];
  const add = (r) => {
    if (underAny(r, roots)) return;
    // A later, wider root replaces the narrower ones it covers.
    for (let i = roots.length - 1; i >= 0; i--) if (isUnder(roots[i], r)) roots.splice(i, 1);
    roots.push(r);
  };
  for (const p of [...alwaysHidden, home, evalsDir, ...hide]) {
    if (!p || !isAbsolute(p)) continue;
    const r = resolve(p);
    if (!exists(r)) continue;
    // Both the path and where it really is: a $HOME that is a symlink to
    // /data/owner must hide /data/owner, and the binds must see real paths
    // as hidden. The real path first, so a wider one replaces the narrower.
    let real;
    try {
      real = realpath(r);
    } catch {
      real = r;
    }
    add(real);
    add(r);
  }
  return roots;
}

/**
 * Resolve `p` one component at a time. Returns its real path and every
 * symlink met on the way as [target, linkPath], so the same path can be
 * rebuilt inside the sandbox. Null when the path doesn't exist.
 */
export function resolveChain(p) {
  const links = [];
  let todo = resolve(p).split('/').filter(Boolean);
  let cur = '/';
  let hops = 0;
  while (todo.length > 0) {
    const seg = todo.shift();
    if (seg === '.') continue;
    if (seg === '..') {
      cur = dirname(cur);
      continue;
    }
    const next = join(cur, seg);
    let st;
    try {
      st = lstatSync(next);
    } catch {
      return null;
    }
    if (!st.isSymbolicLink()) {
      cur = next;
      continue;
    }
    if (++hops > 40) throw new Error(`sandbox: too many symlinks resolving ${p}`);
    const target = readlinkSync(next);
    links.push([target, next]);
    todo = [...target.split('/').filter(Boolean), ...todo];
    if (isAbsolute(target)) cur = '/';
  }
  return { real: cur, links };
}

function readHead(p, n) {
  const fd = openSync(p, 'r');
  try {
    const buf = Buffer.alloc(n);
    const got = readSync(fd, buf, 0, n, 0);
    return buf.subarray(0, got);
  } finally {
    closeSync(fd);
  }
}

/** A 64-bit little-endian ELF's PT_INTERP, '' for an ELF without one, null for a non-ELF. */
export function elfInterp(p) {
  const h = readHead(p, 64);
  if (h.length < 64 || h.readUInt32BE(0) !== 0x7f454c46) return null;
  if (h[4] !== 2 || h[5] !== 1) return '';
  const phoff = Number(h.readBigUInt64LE(32));
  const size = h.readUInt16LE(54);
  const count = h.readUInt16LE(56);
  const fd = openSync(p, 'r');
  try {
    const ph = Buffer.alloc(size * count);
    readSync(fd, ph, 0, ph.length, phoff);
    for (let i = 0; i < count; i++) {
      if (ph.readUInt32LE(i * size) !== 3) continue;
      const off = Number(ph.readBigUInt64LE(i * size + 8));
      const len = Number(ph.readBigUInt64LE(i * size + 32));
      const s = Buffer.alloc(len);
      readSync(fd, s, 0, len, off);
      return s.toString('latin1').replace(/\0.*$/s, '');
    }
    return '';
  } finally {
    closeSync(fd);
  }
}

/** The shared libraries `elf` loads, as its own dynamic loader lists them. */
function elfLibs(elf, interp) {
  if (!interp) return [];
  const r = spawnSync(interp, ['--list', elf], { encoding: 'utf8', env: {}, timeout: 10_000 });
  if (r.status !== 0) return [];
  const libs = [];
  for (const line of r.stdout.split('\n')) {
    const m = /=>\s*(\/\S+)/.exec(line) ?? /^\s*(\/\S+)\s+\(0x/.exec(line);
    if (m) libs.push(resolve(m[1]));
  }
  return libs;
}

/**
 * The OpenSSL config and CA files a libcrypto reads, from the OPENSSLDIR
 * compiled into it (Homebrew's points under /home/linuxbrew), else []. Not
 * the whole dir: its `private/` is for keys.
 */
export function opensslFiles(lib) {
  if (!/^libcrypto\.so/.test(basename(lib))) return [];
  let text;
  try {
    text = readFileSync(lib, 'latin1');
  } catch {
    return [];
  }
  const dir = /OPENSSLDIR: "([^"]+)"/.exec(text)?.[1];
  return dir && isAbsolute(dir) ? ['openssl.cnf', 'cert.pem', 'certs'].map((f) => join(dir, f)) : [];
}

/** `#!interp [arg]` of a script, as {interp, arg}, else null. */
function shebang(p) {
  const head = readHead(p, 256).toString('latin1');
  if (!head.startsWith('#!')) return null;
  const [interp, ...rest] = head.slice(2).split('\n')[0].trim().split(/\s+/);
  return { interp, args: rest };
}

/** The nearest dir from `file`'s dir up holding a package.json with a name, else null. */
function packageRoot(file) {
  for (let d = dirname(file); d !== '/'; d = dirname(d)) {
    try {
      if (JSON.parse(readFileSync(join(d, 'package.json'), 'utf8'))?.name) return d;
    } catch {
      // none here
    }
  }
  return null;
}

/**
 * Every host path an executable needs, as paths to expose: the command
 * itself (through its symlinks); for an ELF, its loader and libraries (and a
 * libcrypto's CA files); for a
 * script, its interpreter (an `env` shebang's name is looked up on `path`)
 * and, when it sits in a node package, the whole package.
 */
export function commandNeeds(cmd, path, seen = new Set()) {
  if (seen.has(cmd)) return [];
  seen.add(cmd);
  const chain = resolveChain(cmd);
  if (!chain) return [];
  const needs = [cmd];
  const interp = elfInterp(chain.real);
  if (interp != null) {
    if (interp) {
      const libs = elfLibs(chain.real, interp);
      needs.push(interp, ...libs, ...libs.flatMap(opensslFiles));
    }
    return needs;
  }
  const sb = shebang(chain.real);
  if (!sb) return needs;
  const pkg = packageRoot(chain.real);
  if (pkg) needs.push(pkg);
  let interpreter = sb.interp;
  if (basename(interpreter) === 'env') {
    const name = sb.args.find((a) => !a.startsWith('-'));
    interpreter = name ? findOnPath(name, path) : null;
  }
  if (interpreter) needs.push(...commandNeeds(interpreter, path, seen));
  return needs;
}

/** A venv's base Python install (from pyvenv.cfg's `home`), else null. */
function venvPython(project) {
  try {
    const cfg = readFileSync(join(project, '.venv', 'pyvenv.cfg'), 'utf8');
    const home = /^home\s*=\s*(.+?)\s*$/m.exec(cfg)?.[1];
    return home && isAbsolute(home) ? dirname(home) : null;
  } catch {
    return null;
  }
}

/**
 * bwrap args for `paths` under the hidden roots: each real path bound
 * read-only, each symlink on the way recreated. Paths outside the hidden
 * roots are visible already.
 */
function exposeArgs(paths, hidden) {
  const binds = [];
  const links = new Map();
  for (const p of paths) {
    const chain = resolveChain(p);
    if (!chain) continue;
    for (const [target, link] of chain.links) if (underAny(link, hidden)) links.set(link, target);
    if (underAny(chain.real, hidden)) binds.push(chain.real);
  }
  // Bind each tree once: drop paths inside another bound path.
  const uniq = [...new Set(binds)].sort();
  const kept = uniq.filter((p) => !uniq.some((q) => q !== p && isUnder(p, q)));
  const args = kept.flatMap((p) => ['--ro-bind', p, p]);
  for (const [link, target] of links) {
    if (kept.some((k) => isUnder(link, k))) continue; // already inside a bound tree
    args.push('--symlink', target, link);
  }
  return args;
}

const realOrSelf = (p) => {
  try {
    return realpathSync(p);
  } catch {
    return p;
  }
};

/**
 * `--ro-bind /dev/null <sock>` for each daemon socket that exists outside
 * the hidden roots: connecting to a character device fails.
 */
function socketArgs(sockets, hidden) {
  const args = [];
  const seen = new Set();
  for (const sock of sockets) {
    let real;
    try {
      lstatSync(sock);
      real = realpathSync(sock);
    } catch {
      continue; // not there
    }
    if (underAny(real, hidden) || seen.has(real)) continue;
    seen.add(real);
    args.push('--ro-bind', '/dev/null', real);
  }
  return args;
}

/**
 * The host's `/`, read-only, one top-level entry at a time (each dir bound,
 * each symlink recreated) instead of one bind of `/`: the sandbox's own root
 * stays a tmpfs, so bwrap can make mount points that don't exist on the host
 * (the IN_SANDBOX paths under /work). `/` is remounted read-only after the
 * binds. /dev and /proc are the sandbox's own; `skip` names entries not to
 * take from the host.
 */
export function rootArgs(skip = [IN_SANDBOX.root.slice(1)]) {
  const args = [];
  for (const e of readdirSync('/', { withFileTypes: true })) {
    if (['dev', 'proc', ...skip].includes(e.name)) continue;
    const p = `/${e.name}`;
    if (e.isSymbolicLink()) args.push('--symlink', readlinkSync(p), p);
    else if (e.isDirectory()) args.push('--ro-bind', p, p);
    else if (e.isFile()) args.push('--ro-bind', p, p);
  }
  return args;
}

/** A bind spec as [src, dest]: a path is bound at itself. */
const pair = (b) => (Array.isArray(b) ? b : [b, b]);

/**
 * Wrap an adapter's {cmd, args, env, cwd} in bwrap.
 *
 * @param {object} spec  The adapter's AgentCommand; `cwd` is in-sandbox.
 * @param {object} dirs  The run's {project, home, xdg, bin} as the agent sees
 *   them, and in `dirs.host` where they are on the host (prepareRun's
 *   shape); each host dir is bound at its in-sandbox path. Without
 *   `dirs.host`, every dir is bound at its own path.
 * @param {object} opts  `bwrap` (its path; required), `home` (the owner's
 *   home), `evalsDir`, `hide` (more dirs to hide), `ro` (more paths to bind
 *   read-only, each a path or [hostPath, inSandboxPath], e.g. the round's
 *   export), `rw` (the same, read-write), `tools` (command names the task
 *   needs, looked up on the spec's PATH), `daemonSockets` (default
 *   DAEMON_SOCKETS), `alwaysHidden` (default ALWAYS_HIDDEN; tests only).
 * @returns {{cmd: string, args: string[], env: Record<string, string>}}  Spawn
 *   `cmd args` with exactly `env`, from the host dir `dirs.host.project`.
 */
export function sandboxArgs(spec, dirs, opts = {}) {
  const {
    bwrap,
    home = homedir(),
    evalsDir = evalsStateDir(home),
    hide = [],
    ro = [],
    rw = [],
    tools = [],
    alwaysHidden,
    daemonSockets = DAEMON_SOCKETS,
  } = opts;
  if (!bwrap) throw new Error('sandbox: no bwrap path');
  const env = spec.env ?? {};
  const host = dirs.host ?? dirs;
  const cwd = spec.cwd ?? dirs.project;
  const hidden = hiddenRoots({ home, evalsDir, hide, alwaysHidden });

  const needs = [...commandNeeds(spec.cmd, env.PATH)];
  for (const t of tools) {
    const p = findOnPath(t, env.PATH);
    if (p) needs.push(...commandNeeds(p, env.PATH));
  }
  const py = host.project ? venvPython(host.project) : null;
  if (py) needs.push(py, ...commandNeeds(join(host.project, '.venv', 'bin', 'python'), env.PATH));

  // Same-path read-only paths go through exposeArgs (which follows their
  // symlinks); those bound elsewhere are bound as they are.
  const roPairs = ro.filter(Boolean).map(pair);
  const roSame = roPairs.filter(([s, d]) => s === d).map(([s]) => s);
  const roMoved = roPairs.filter(([s, d]) => s !== d);

  const args = [...rootArgs(), '--dev', '/dev', '--proc', '/proc'];
  // A tmpfs on each real path; a symlinked alias is in `hidden` only so the
  // binds below treat paths through it as hidden (bwrap won't mount on a
  // symlink, and the target is covered already).
  for (const h of hidden) if (realOrSelf(h) === h) args.push('--tmpfs', h);
  args.push(...exposeArgs([...ALWAYS_BOUND, ...needs, ...roSame], hidden));
  args.push(...socketArgs(daemonSockets, hidden));
  for (const [s, d] of roMoved) args.push('--ro-bind', s, d);
  if (host.bin) args.push('--ro-bind', host.bin, dirs.bin ?? host.bin);
  for (const k of ['project', 'home', 'xdg']) if (host[k]) args.push('--bind', host[k], dirs[k] ?? host[k]);
  for (const [s, d] of rw.filter(Boolean).map(pair)) args.push('--bind', s, d);
  // Files the agent needs that hold a secret (pi's auth.json): bwrap copies
  // each from a pipe into its own tmpfs and binds it, 0600, so it is never
  // on the host's disk. Fds 4, 5, ... (3 carries the args; see spawnSandbox).
  const dataFiles = Array.isArray(spec.dataFiles) ? spec.dataFiles : [];
  dataFiles.forEach((f, i) => args.push('--perms', '0600', '--bind-data', String(4 + i), f.path));
  args.push('--remount-ro', '/', ...NAMESPACES, '--chdir', cwd, '--', spec.cmd, ...(spec.args ?? []));
  // bwrap gets exactly the adapter's env and passes it on as is. Not
  // --clearenv/--setenv: the values (secrets) would sit in bwrap's argv,
  // which /proc shows every local user.
  const command = [spec.cmd, ...(spec.args ?? [])];
  return {
    cmd: bwrap,
    args,
    options: args.slice(0, args.length - command.length - 1),
    command,
    data: dataFiles.map((f) => Buffer.from(String(f.data))),
    env: Object.fromEntries(Object.entries(env).map(([k, v]) => [k, String(v)])),
  };
}

/**
 * Spawn a sandboxArgs result with bwrap's own options (the binds, which name
 * the round, the run and its arm) on a pipe (`--args 3`), not in its argv:
 * bwrap is pid 1 inside the sandbox, and its argv is any process's
 * /proc/1/cmdline. Its argv is just `bwrap --args 3 -- <command>`. Data files
 * go on fds 4, 5, .... `opts` are spawn's (cwd, detached); `stdio` gives
 * fds 0-2, default all piped.
 */
export function spawnSandbox(sb, { stdio = ['pipe', 'pipe', 'pipe'], ...opts } = {}) {
  const extra = 1 + (sb.data?.length ?? 0);
  const child = spawn(sb.cmd, ['--args', '3', '--', ...sb.command], { ...opts, env: sb.env, stdio: [...stdio, ...Array(extra).fill('pipe')] });
  const feeds = [Buffer.from(sb.options.map((a) => `${a}\0`).join('')), ...(sb.data ?? [])];
  feeds.forEach((buf, i) => {
    const s = child.stdio[3 + i];
    if (!s) return;
    s.on('error', () => {});
    s.end(buf);
  });
  return child;
}



const isDir = (p) => {
  try {
    return statSync(p).isDirectory();
  } catch {
    return false;
  }
};

/**
 * The sandbox a round's run gets: sandboxArgs plus, hidden, the round dir and
 * every directory in its taintPaths (the real repos, this checkout); bound
 * read-only at IN_SANDBOX.export, the round's export; and the task's setup
 * command (its toolchain, e.g. `uv`) bound in like the agent's own install.
 * `sandbox` ({ro, rw, alwaysHidden}) adds binds or narrows the default
 * hides (scoring's hidden tests; tests).
 */
export function roundSandbox(spec, dirs, round, { bwrap, sandbox = {} }) {
  return sandboxArgs(spec, dirs, {
    bwrap,
    hide: [round.dir, ...(round.taintPaths ?? []).filter(isDir)],
    ro: [round.export ? [round.export, IN_SANDBOX.export] : null, ...(sandbox.ro ?? [])].filter(Boolean),
    rw: sandbox.rw ?? [],
    tools: round.plan.taskDef.setup?.slice(0, 1) ?? [],
    alwaysHidden: sandbox.alwaysHidden,
  });
}

/**
 * Host pids of every live descendant of `pid`, from /proc. The agent runs
 * in its own session inside bwrap (--new-session), so a signal to bwrap's
 * process group never reaches it; the call loop signals these instead.
 */
export function descendants(pid) {
  const children = new Map();
  let entries;
  try {
    entries = readdirSync('/proc');
  } catch {
    return [];
  }
  for (const e of entries) {
    if (!/^\d+$/.test(e)) continue;
    let stat;
    try {
      stat = readFileSync(`/proc/${e}/stat`, 'utf8');
    } catch {
      continue;
    }
    // pid (comm) state ppid ...: comm may hold spaces and parens.
    const ppid = Number(stat.slice(stat.lastIndexOf(')') + 2).split(' ')[1]);
    if (!children.has(ppid)) children.set(ppid, []);
    children.get(ppid).push(Number(e));
  }
  const out = [];
  const todo = [...(children.get(pid) ?? [])];
  while (todo.length > 0) {
    const p = todo.shift();
    out.push(p);
    todo.push(...(children.get(p) ?? []));
  }
  return out;
}
