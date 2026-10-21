import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, existsSync, statSync, chmodSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir, homedir } from 'node:os';
import { join, delimiter, relative } from 'node:path';
import { freezePlan, expandRuns, roundsDir } from './plan.mjs';
import { prepareRound, prepareRun, REPO_ROOT } from './workspace.mjs';

const COMMIT_FILE = 'src/lib.py';
let tmp, repo, commit, tasksDir, fakeCodecite, fakeDevBin, round, runs;

function sh(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', ...opts });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(' ')}: ${r.stderr}`);
  return r.stdout.trim();
}

function withEvalsDir(dir, fn) {
  const old = process.env.CODECITE_EVALS_DIR;
  process.env.CODECITE_EVALS_DIR = dir;
  try {
    return fn();
  } finally {
    if (old === undefined) delete process.env.CODECITE_EVALS_DIR;
    else process.env.CODECITE_EVALS_DIR = old;
  }
}

function which(path) {
  return spawnSync('sh', ['-c', 'command -v codecite'], { encoding: 'utf8', env: { PATH: path } });
}

before(() => {
  tmp = mkdtempSync(join(tmpdir(), 'evals-ws-'));

  // A fake identity-model: a git repo with one commit, then a later uncommitted change.
  repo = join(tmp, 'identity-model');
  mkdirSync(join(repo, 'src'), { recursive: true });
  writeFileSync(join(repo, COMMIT_FILE), 'VALUE = 1\n');
  writeFileSync(join(repo, 'ignored.txt'), 'kept\n');
  writeFileSync(join(repo, '.gitattributes'), 'ignored.txt export-ignore\n');
  sh('git', ['init', '-q', repo]);
  sh('git', ['-C', repo, 'add', '.']);
  sh('git', ['-C', repo, '-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-qm', 'init']);
  commit = sh('git', ['-C', repo, 'rev-parse', 'HEAD']);
  writeFileSync(join(repo, COMMIT_FILE), 'VALUE = 2\n');

  // A fake task.
  tasksDir = join(tmp, 'tasks');
  const taskDir = join(tasksDir, 'fake');
  mkdirSync(join(taskDir, 'project', 'app'), { recursive: true });
  writeFileSync(join(taskDir, 'prompt.md'), 'Source at `{identity_model}`.\n');
  writeFileSync(join(taskDir, 'project', 'pyproject.toml'), 'path = "@IDENTITY_MODEL@/py"\n');
  writeFileSync(join(taskDir, 'project', 'app', 'main.py'), 'app = None\n');
  writeFileSync(
    join(taskDir, 'task.json'),
    JSON.stringify({
      id: 'fake',
      project: 'project',
      prompt: 'prompt.md',
      hidden: 'hidden',
      corpus: { repo: 'identity-model', commit },
      setup: ['sh', '-c', 'echo ran > setup-ran'],
      score: ['true'],
      scoreWarm: ['true'],
    }),
  );

  // A fake codecite binary: `index eval --manifest <path>` writes a corpus
  // file holding the manifest, then the files of the source repo it names.
  fakeCodecite = join(tmp, 'fake-codecite');
  writeFileSync(
    fakeCodecite,
    [
      '#!/bin/sh',
      'set -e',
      'db="$XDG_DATA_HOME/codecite/$2.db"',
      'mkdir -p "$XDG_DATA_HOME/codecite" "$XDG_CACHE_HOME/clone"',
      'cp "$4" "$db"',
      'echo FILES >> "$db"',
      'url=$(sed -n \'s/^url = "\\(.*\\)"$/\\1/p\' "$4")',
      'echo "$url" > "$XDG_CACHE_HOME/clone/origin"',
      'git -C "$url" ls-files >> "$db"',
      '',
    ].join('\n'),
  );
  chmodSync(fakeCodecite, 0o755);

  // A developer bin dir with some other codecite on it.
  fakeDevBin = join(tmp, 'devbin');
  mkdirSync(fakeDevBin);
  writeFileSync(join(fakeDevBin, 'codecite'), '#!/bin/sh\necho dev\n');
  chmodSync(join(fakeDevBin, 'codecite'), 0o755);

  const spec = {
    task: 'fake',
    runs: 1,
    seed: 1,
    timeoutSec: 60,
    repos: { 'identity-model': repo },
    agents: [{ agent: 'pi', model: 'x/y' }],
    arms: [{ name: 'baseline' }, { name: 'codecite', codecite: true, note: 'Corpus at `{identity_model}`.' }],
  };
  const rounds = join(tmp, 'rounds');
  const plan = withEvalsDir(rounds, () => freezePlan(spec, { codeciteCommit: 'abc', roundId: 'r1', tasksDir }));
  round = prepareRound(plan, join(rounds, 'r1'), { codeciteBin: fakeCodecite });
  runs = expandRuns(plan);
});

after(() => {
  spawnSync('chmod', ['-R', 'u+w', tmp]);
  rmSync(tmp, { recursive: true, force: true });
});

test('export has no .git', () => {
  assert.equal(readFileSync(join(round.export, COMMIT_FILE), 'utf8'), 'VALUE = 1\n');
  assert.equal(existsSync(join(round.export, '.git')), false);
});

test('export is read-only', () => {
  for (const p of [round.export, join(round.export, 'src'), join(round.export, COMMIT_FILE)]) {
    assert.equal(statSync(p).mode & 0o222, 0, p);
  }
});

test('round has a manifest, an index and a codecite binary', () => {
  const manifest = readFileSync(round.manifest, 'utf8');
  assert.match(manifest, /^name = "eval"$/m);
  assert.match(manifest, /^kind = "git"$/m);
  assert.ok(manifest.includes(`url = ${JSON.stringify(join(round.dir, 'src-repo'))}`));
  assert.ok(!manifest.includes(repo + '"'));
  assert.ok(manifest.includes(`commit = "${commit}"`));
  assert.equal(existsSync(join(round.dir, 'xdg-cache')), false);
  assert.ok(existsSync(join(round.xdg, 'codecite', 'eval.db')));
  assert.ok(statSync(join(round.binCodecite, 'codecite')).mode & 0o100);
});

test('baseline PATH has no codecite', () => {
  const run = runs.find((r) => r.arm === 'baseline');
  const basePath = [fakeDevBin, join(homedir(), '.cargo', 'bin'), join(REPO_ROOT, 'target', 'release'), process.env.PATH].join(delimiter);
  assert.equal(which(basePath).status, 0);
  const dirs = prepareRun(run, round, { basePath });
  assert.notEqual(which(dirs.path).status, 0);
  const entries = dirs.path.split(delimiter);
  assert.ok(!entries.includes(round.binCodecite));
  assert.ok(!entries.includes(join(homedir(), '.cargo', 'bin')));
  assert.ok(!entries.some((e) => e.startsWith(join(REPO_ROOT, 'target'))));
  assert.equal(dirs.bin, null);
  assert.equal(existsSync(join(dirs.xdg, 'codecite')), false);
});

test('codecite PATH finds the round codecite and nothing else differs', () => {
  const basePath = [fakeDevBin, process.env.PATH].join(delimiter);
  const base = prepareRun({ ...runs.find((r) => r.arm === 'baseline'), id: 'b-path' }, round, { basePath });
  const cc = prepareRun({ ...runs.find((r) => r.arm === 'codecite'), id: 'c-path' }, round, { basePath });
  assert.equal(which(cc.path).stdout.trim(), join(round.binCodecite, 'codecite'));
  assert.equal(cc.path, [round.binCodecite, base.path].join(delimiter));
  assert.ok(existsSync(join(cc.xdg, 'codecite', 'eval.db')));
});

test('placeholder replaced', () => {
  const dirs = prepareRun({ ...runs[0], id: 'placeholder' }, round, { basePath: process.env.PATH });
  const toml = readFileSync(join(dirs.project, 'pyproject.toml'), 'utf8');
  assert.equal(toml, `path = "${round.export}/py"\n`);
  assert.equal(readFileSync(join(dirs.project, 'setup-ran'), 'utf8'), 'ran\n');
  assert.ok(existsSync(dirs.home));
  assert.ok(dirs.out.startsWith(round.dir));
});

function walk(dir, base = dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, base, out);
    else out.push(relative(base, p));
  }
  return out;
}

test('the real repo path appears in no round file but plan.json and round.json', () => {
  prepareRun({ ...runs.find((r) => r.arm === 'codecite'), id: 'leak-check' }, round, { basePath: process.env.PATH });
  const leaks = walk(round.dir).filter((f) => readFileSync(join(round.dir, f)).includes(repo));
  assert.deepEqual(leaks.sort(), ['plan.json', 'round.json']);
  assert.ok(readFileSync(join(round.xdg, 'codecite', 'eval.db')).length > 0);
  assert.equal(existsSync(join(round.dir, 'src-repo')), false);
});

test('export == indexed tree, export-ignore notwithstanding', () => {
  const db = readFileSync(join(round.xdg, 'codecite', 'eval.db'), 'utf8');
  const indexed = db.split('FILES\n')[1].trim().split('\n').sort();
  assert.deepEqual(walk(round.export).sort(), indexed);
  assert.ok(indexed.includes('ignored.txt'));
});

test('round records its taint paths', () => {
  const saved = JSON.parse(readFileSync(join(round.dir, 'round.json'), 'utf8'));
  assert.deepEqual(saved.taintPaths, round.taintPaths);
  assert.deepEqual(round.taintPaths, [
    repo,
    REPO_ROOT,
    join(round.dir, 'plan.json'),
    join(round.dir, 'round.json'),
    join(round.dir, 'corpus.toml'),
    join(round.dir, 'runs'),
  ]);
});

test('a failed prepareRound leaves no half-made round', () => {
  const dir = join(tmp, 'rounds', 'broken');
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'plan.json'), '{}\n');
  const bad = { ...round.plan, taskDef: { ...round.plan.taskDef, corpus: { repo: 'identity-model', commit: '0'.repeat(40) } } };
  assert.throws(() => prepareRound(bad, dir, { codeciteBin: fakeCodecite }));
  assert.deepEqual(readdirSync(dir), ['plan.json']);
});

test('round dir inside a git repo is refused', () => {
  assert.throws(() => withEvalsDir(join(repo, 'rounds'), () => roundsDir()), /git/);
  assert.throws(() => prepareRound(round.plan, join(repo, 'round'), { codeciteBin: fakeCodecite }), /git/);
});
