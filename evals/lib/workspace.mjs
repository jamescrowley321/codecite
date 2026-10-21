// Rounds and runs on disk: the identity-model export, the prebuilt corpus,
// and a fresh, isolated directory set for each run.

import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { delimiter, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertOutsideGit } from './plan.mjs';

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
  // codecite checkout (the hidden tests), and the round's own records and
  // other runs.
  const taintPaths = [
    ...Object.values(plan.repos).map((p) => resolve(expandHome(p))),
    REPO_ROOT,
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
 * Prepare one run's directories under <round>/runs/<run.id>: a fresh project
 * (placeholder substituted, setup run before the clock starts), a clean home,
 * its own XDG_DATA_HOME, and its PATH. Only the codecite arm gets the corpus
 * and bin-codecite. Returns {out, project, home, xdg, bin, path}.
 */
export function prepareRun(runSpec, round, { basePath = process.env.PATH } = {}) {
  const arm = round.plan.arms.find((a) => a.name === runSpec.arm);
  if (!arm) throw new Error(`run ${runSpec.id}: unknown arm "${runSpec.arm}"`);
  const task = round.plan.taskDef;
  const out = join(round.dir, 'runs', runSpec.id);
  if (existsSync(out)) throw new Error(`run ${runSpec.id}: ${out} already exists`);

  const project = join(out, 'project');
  const home = join(out, 'home');
  const xdg = join(out, 'xdg');
  cpSync(join(task.dir, task.project), project, {
    recursive: true,
    filter: (src) => !/(^|\/)(\.venv|__pycache__)$/.test(src),
  });
  substitute(project, round.export);
  mkdirSync(home, { recursive: true });

  const shared = cleanPath(basePath);
  let bin = null;
  if (arm.codecite) {
    cpSync(round.xdg, xdg, { recursive: true });
    bin = round.binCodecite;
  } else {
    mkdirSync(xdg, { recursive: true });
  }
  const path = bin ? [bin, shared].filter(Boolean).join(delimiter) : shared;

  if (task.setup.length > 0) {
    const r = spawnSync(task.setup[0], task.setup.slice(1), {
      cwd: project,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
      env: { ...process.env, PATH: shared },
    });
    writeFileSync(join(out, 'setup.log'), `${r.stdout ?? ''}${r.stderr ?? ''}`);
    if (r.error || r.status !== 0) {
      throw new Error(`run ${runSpec.id}: setup ${task.setup.join(' ')} failed; see ${join(out, 'setup.log')}`);
    }
  }
  return { out, project, home, xdg, bin, path };
}
