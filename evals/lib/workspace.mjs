// Rounds and runs on disk: the identity-model export, the prebuilt corpus,
// and a fresh, isolated directory set for each run.

import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { basename, delimiter, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertOutsideGit } from './plan.mjs';
import { findBwrap, findOnPath, IN_SANDBOX, roundSandbox } from './sandbox.mjs';

export const REPO_ROOT = fileURLToPath(new URL('../../', import.meta.url)).replace(/\/$/, '');
const PLACEHOLDER = '@IDENTITY_MODEL@';

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, ...opts });
  if (r.error) throw new Error(`${cmd}: ${r.error.message}`);
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(' ')} exited ${r.status}: ${(r.stderr || '').trim().slice(-2000)}`);
  return r;
}

const expandHome = (p) => (p === '~' || p.startsWith('~/') ? join(homedir(), p.slice(1)) : p);
const isUnder = (p, dir) => {
  const rel = relative(dir, p);
  return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel));
};

/** Build the release codecite in this repo and return the binary's path. */
export function buildCodecite() {
  run('cargo', ['build', '--release', '-p', 'codecite'], { cwd: REPO_ROOT, stdio: ['ignore', 'inherit', 'inherit'] });
  return join(REPO_ROOT, 'target', 'release', 'codecite');
}

// What prepareRound creates in a round dir; a failure removes exactly these.
const ROUND_PARTS = ['src-repo', 'identity-model', 'corpus.toml', 'bin-codecite', 'xdg', 'xdg-cache', 'round.json'];

/**
 * Prepare a round once, before any run. identity-model's pinned commit is
 * fetched into a throwaway repo; its checkout, without .git,
 * becomes the read-only export, and the same repo is indexed as corpus
 * `eval`. The throwaway repo and codecite's clone cache are then deleted, so
 * nothing the runs can see names the real repo. `codeciteBin` skips the
 * cargo build and uses that binary instead.
 */
export function prepareRound(plan, roundDir, { codeciteBin } = {}) {
  const dir = resolve(roundDir);
  assertOutsideGit(dir);
  const { repo, commit } = plan.taskDef.corpus;
  const repoPath = plan.repos[repo];
  if (!repoPath) throw new Error(`round: plan.repos has no path for "${repo}"`);
  for (const part of ROUND_PARTS) {
    if (existsSync(join(dir, part))) throw new Error(`round: ${join(dir, part)} already exists`);
  }
  mkdirSync(dir, { recursive: true });
  try {
    return buildRound(plan, dir, { repo, commit, repoPath: expandHome(repoPath), codeciteBin });
  } catch (e) {
    spawnSync('chmod', ['-R', 'u+w', dir]);
    for (const part of ROUND_PARTS) rmSync(join(dir, part), { recursive: true, force: true });
    throw e;
  }
}

function buildRound(plan, dir, { repo, commit, repoPath, codeciteBin }) {
  // Not --depth=1: codecite's `clone --filter=blob:none` of a shallow local
  // repo loops forever (git 2.34). The history goes when this repo does.
  const srcRepo = join(dir, 'src-repo');
  run('git', ['init', '-q', srcRepo]);
  run('git', ['-C', srcRepo, 'fetch', '-q', `file://${repoPath}`, commit]);
  run('git', ['-C', srcRepo, '-c', 'advice.detachedHead=false', 'checkout', '-q', '--detach', 'FETCH_HEAD']);

  // The full checked-out tree (export-ignore doesn't apply), so the export
  // is exactly what codecite indexes.
  const exportDir = join(dir, 'identity-model');
  cpSync(srcRepo, exportDir, {
    recursive: true,
    verbatimSymlinks: true,
    filter: (src) => src !== join(srcRepo, '.git'),
  });
  run('chmod', ['-R', 'a-w', exportDir]);

  // codecite refuses a `local` source without .git, so the corpus is a `git`
  // source at the throwaway repo: no network, citations `<repo>@<commit>`.
  const manifest = join(dir, 'corpus.toml');
  writeFileSync(
    manifest,
    [
      'name = "eval"',
      '',
      '[[source]]',
      'kind = "git"',
      `name = ${JSON.stringify(repo)}`,
      `url = ${JSON.stringify(srcRepo)}`,
      `commit = ${JSON.stringify(commit)}`,
      'license = "Apache-2.0"',
      'use = "adapt"',
      '',
    ].join('\n'),
  );

  const binCodecite = join(dir, 'bin-codecite');
  mkdirSync(binCodecite, { recursive: true });
  const bin = join(binCodecite, 'codecite');
  copyFileSync(codeciteBin ?? buildCodecite(), bin);
  chmodSync(bin, 0o755);

  const xdg = join(dir, 'xdg');
  const cache = join(dir, 'xdg-cache');
  run(bin, ['index', 'eval', '--manifest', manifest], {
    env: { ...process.env, XDG_DATA_HOME: xdg, XDG_CACHE_HOME: cache },
  });
  rmSync(cache, { recursive: true, force: true });
  rmSync(srcRepo, { recursive: true, force: true });

  // Paths a run must not touch (Task 4 flags them): the real repos, this
  // codecite checkout (the hidden tests), the round's frozen task (hidden
  // tests, reference), and the round's own records and other runs.
  const taintPaths = [
    ...Object.values(plan.repos).map((p) => resolve(expandHome(p))),
    REPO_ROOT,
    join(dir, 'task'),
    join(dir, 'plan.json'),
    join(dir, 'round.json'),
    manifest,
    join(dir, 'runs'),
  ];
  const round = { dir, export: exportDir, manifest, xdg, binCodecite, taintPaths };
  writeFileSync(join(dir, 'round.json'), `${JSON.stringify(round, null, 2)}\n`);
  return { ...round, plan };
}

/**
 * The developer's PATH with every way to reach a codecite removed: relative
 * entries, ~/.cargo/bin, this repo's target/, and any directory holding a
 * `codecite`. Both arms start from this, so they differ only in bin-codecite.
 */
export function cleanPath(basePath = process.env.PATH ?? '') {
  const cargoBin = join(homedir(), '.cargo', 'bin');
  const target = join(REPO_ROOT, 'target');
  const seen = new Set();
  return basePath
    .split(delimiter)
    .filter((e) => {
      if (!e || !isAbsolute(e) || seen.has(e)) return false;
      seen.add(e);
      return !isUnder(e, cargoBin) && !isUnder(e, target) && !existsSync(join(e, 'codecite'));
    })
    .join(delimiter);
}

function substitute(dir, value) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) substitute(p, value);
    else if (entry.isFile()) {
      const text = readFileSync(p, 'utf8');
      if (text.includes(PLACEHOLDER)) writeFileSync(p, text.replaceAll(PLACEHOLDER, value));
    }
  }
}

/**
 * The interpreter `uv` would pick for `project` on the host and the install
 * it lives in (sys.base_prefix), so a sandboxed `uv sync` can be given both:
 * the owner's uv-managed Pythons live under the hidden home. Null when
 * there is none.
 */
function hostPython(uv, project, env) {
  const find = spawnSync(uv, ['python', 'find'], { cwd: project, encoding: 'utf8', env });
  const python = find.status === 0 ? find.stdout.trim() : '';
  if (!python || !isAbsolute(python)) return null;
  const prefix = spawnSync(python, ['-c', 'import sys; print(sys.base_prefix)'], { encoding: 'utf8', env: { PATH: env.PATH } });
  const root = prefix.status === 0 ? prefix.stdout.trim() : '';
  return { python, root: root && isAbsolute(root) ? root : null };
}

/**
 * Run the task's setup command (e.g. `uv sync`) in the run's sandbox, at
 * the same in-sandbox paths the agent gets, so the environment it builds
 * (the venv's scripts, the path deps on the export) is valid there. A `uv`
 * setup gets the host's Python bound in, and a per-round uv cache
 * (<round>/uv-cache) bound only here, never for the agent.
 */
function runSetup(task, dirs, round, { shared, bwrap, sandbox = {} }) {
  const cmd = findOnPath(task.setup[0], shared);
  if (!cmd) throw new Error(`setup: \`${task.setup[0]}\` not found on PATH (${shared})`);
  const env = { PATH: shared, HOME: IN_SANDBOX.home, LANG: 'C.UTF-8' };
  const ro = [...(sandbox.ro ?? [])];
  const rw = [...(sandbox.rw ?? [])];
  if (basename(cmd) === 'uv') {
    const cache = join(round.dir, 'uv-cache');
    mkdirSync(cache, { recursive: true });
    rw.push([cache, IN_SANDBOX.uvCache]);
    Object.assign(env, { UV_CACHE_DIR: IN_SANDBOX.uvCache, UV_LINK_MODE: 'copy', UV_PYTHON_DOWNLOADS: 'never', UV_NO_PROGRESS: '1' });
    const py = hostPython(cmd, dirs.host.project, { ...process.env, PATH: shared });
    if (py) {
      env.UV_PYTHON = py.python;
      ro.push(py.python, ...(py.root ? [py.root] : []));
    }
  }
  const spec = { cmd, args: task.setup.slice(1), env, cwd: dirs.project };
  const sb = roundSandbox(spec, dirs, round, { bwrap, sandbox: { ...sandbox, ro, rw } });
  return spawnSync(sb.cmd, sb.args, { cwd: dirs.host.project, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, env: sb.env });
}

/**
 * Prepare one run's directories under <round>/runs/<run.id>: a fresh project
 * (placeholder set to the export's in-sandbox path, setup run in the
 * sandbox before the clock starts), a clean home, its own XDG_DATA_HOME,
 * and its PATH. Only the codecite arm gets the corpus and bin-codecite.
 *
 * Returns the dirs as the agent sees them, at the neutral IN_SANDBOX paths
 * (the same for every arm and run): {out, project, home, xdg, bin, path},
 * plus `host`: {project, home, xdg, bin} where they are on the host. `bin`
 * is null in the baseline arm; PATH differs between the arms only by it.
 * `bwrap` defaults to the one on PATH; `sandbox` ({ro, rw, alwaysHidden})
 * is for tests.
 */
export function prepareRun(runSpec, round, { basePath = process.env.PATH, bwrap = findBwrap(process.env.PATH), sandbox } = {}) {
  const arm = round.plan.arms.find((a) => a.name === runSpec.arm);
  if (!arm) throw new Error(`run ${runSpec.id}: unknown arm "${runSpec.arm}"`);
  const task = round.plan.taskDef;
  const out = join(round.dir, 'runs', runSpec.id);
  if (existsSync(out)) throw new Error(`run ${runSpec.id}: ${out} already exists`);

  const host = { project: join(out, 'project'), home: join(out, 'home'), xdg: join(out, 'xdg'), bin: arm.codecite ? round.binCodecite : null };
  cpSync(join(task.dir, task.project), host.project, {
    recursive: true,
    filter: (src) => !/(^|\/)(\.venv|__pycache__)$/.test(src),
  });
  substitute(host.project, IN_SANDBOX.export);
  mkdirSync(host.home, { recursive: true });
  if (arm.codecite) cpSync(round.xdg, host.xdg, { recursive: true });
  else mkdirSync(host.xdg, { recursive: true });

  const shared = cleanPath(basePath);
  const bin = arm.codecite ? IN_SANDBOX.bin : null;
  const path = bin ? [bin, shared].filter(Boolean).join(delimiter) : shared;
  const dirs = { out, project: IN_SANDBOX.project, home: IN_SANDBOX.home, xdg: IN_SANDBOX.xdg, bin, path, host };

  if (task.setup.length > 0) {
    if (!bwrap) throw new Error(`run ${runSpec.id}: setup runs in the sandbox, and bwrap is not on PATH`);
    const r = runSetup(task, dirs, round, { shared, bwrap, sandbox });
    writeFileSync(join(out, 'setup.log'), `${r.stdout ?? ''}${r.stderr ?? ''}`);
    if (r.error || r.status !== 0) {
      throw new Error(`run ${runSpec.id}: setup ${task.setup.join(' ')} failed; see ${join(out, 'setup.log')}`);
    }
  }
  return dirs;
}
