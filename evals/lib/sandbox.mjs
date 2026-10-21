// The bubblewrap sandbox every agent runs in. The host is visible read-only,
// except the places secrets and other runs live, which are hidden under empty
// tmpfs mounts: /home (and $HOME), /root, /tmp, /var/tmp, /run/user and the
// evals state dir. Into that, only this run's own dirs are bound back
// read-write, and the agent's install, the task's tools and the codecite bin
// dir read-only. The environment is exactly the adapter's (plus PWD, which
// bwrap's --chdir sets), handed to bwrap as its own env. Network is shared:
// agents need their provider's API.

import { spawnSync } from 'node:child_process';
import { accessSync, closeSync, constants, existsSync, lstatSync, openSync, readdirSync, readFileSync, readlinkSync, readSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, delimiter, dirname, isAbsolute, join, relative, resolve } from 'node:path';

/** Hidden in every sandbox, when they exist. */
export const ALWAYS_HIDDEN = ['/home', '/root', '/tmp', '/var/tmp', '/run/user'];

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
export function hiddenRoots({ home = homedir(), evalsDir = evalsStateDir(home), hide = [], exists = existsSync } = {}) {
  const roots = [];
  for (const p of [...ALWAYS_HIDDEN, home, evalsDir, ...hide]) {
    if (!p || !isAbsolute(p)) continue;
    const r = resolve(p);
    if (!exists(r) || underAny(r, roots)) continue;
    // A later, wider root replaces the narrower ones it covers.
    for (let i = roots.length - 1; i >= 0; i--) if (isUnder(roots[i], r)) roots.splice(i, 1);
    roots.push(r);
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

/**
 * Wrap an adapter's {cmd, args, env, cwd} in bwrap.
 *
 * @param {object} spec  The adapter's AgentCommand.
 * @param {object} dirs  prepareRun's {out, project, home, xdg, bin}.
 * @param {object} opts  `bwrap` (its path; required), `home` (the owner's
 *   home), `evalsDir`, `hide` (more dirs to hide), `ro` (more paths to bind
 *   read-only, e.g. the round's export), `rw` (more paths to bind read-write;
 *   tests only), `tools` (command names the task needs, looked up on the
 *   spec's PATH).
 * @returns {{cmd: string, args: string[], env: Record<string, string>}}  Spawn
 *   `cmd args` with exactly `env`.
 */
export function sandboxArgs(spec, dirs, opts = {}) {
  const { bwrap, home = homedir(), evalsDir = evalsStateDir(home), hide = [], ro = [], rw = [], tools = [] } = opts;
  if (!bwrap) throw new Error('sandbox: no bwrap path');
  const env = spec.env ?? {};
  const cwd = spec.cwd ?? dirs.project;
  const hidden = hiddenRoots({ home, evalsDir, hide });

  const needs = [...commandNeeds(spec.cmd, env.PATH)];
  for (const t of tools) {
    const p = findOnPath(t, env.PATH);
    if (p) needs.push(...commandNeeds(p, env.PATH));
  }
  const py = venvPython(dirs.project);
  if (py) needs.push(py, ...commandNeeds(join(dirs.project, '.venv', 'bin', 'python'), env.PATH));

  const args = ['--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc'];
  for (const h of hidden) args.push('--tmpfs', h);
  args.push(...exposeArgs([...needs, ...ro], hidden));
  if (dirs.bin) args.push('--ro-bind', dirs.bin, dirs.bin);
  for (const d of [dirs.project, dirs.home, dirs.xdg, ...rw]) if (d) args.push('--bind', d, d);
  args.push(...NAMESPACES, '--chdir', cwd, '--', spec.cmd, ...(spec.args ?? []));
  // bwrap gets exactly the adapter's env and passes it on as is. Not
  // --clearenv/--setenv: the values (secrets) would sit in bwrap's argv,
  // which /proc shows every local user.
  return { cmd: bwrap, args, env: Object.fromEntries(Object.entries(env).map(([k, v]) => [k, String(v)])) };
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
