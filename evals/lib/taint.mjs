// Taint: did a run reach outside its own sandbox? Only the shell commands an
// agent ran and the paths its file tools touched are scanned, never the
// contents of what it read, so a README mentioning github.com is fine.

import { dirname, isAbsolute, relative, resolve } from 'node:path';

// Fetching code or packages from outside: shell commands only.
const NETWORK = [
  ['github.com', /github\.com/],
  ['git clone', /\bgit\s+clone\b/],
  ['curl', /(^|[\s;&|(`])curl(\s|$)/],
  ['wget', /(^|[\s;&|(`])wget(\s|$)/],
  ['pip install', /\bpip3?\s+install\b/],
  ['uv add', /\buv\s+add\b/],
];

// Where the hidden tests and the reference solution live: shell commands and
// read paths (the hidden tests are reachable by absolute path).
const HIDDEN = [
  ['evals/tasks', /evals\/tasks/],
  ['test_hidden', /test_hidden/],
  ['/reference/', /\/reference(\/|$)/],
  ['codecite-worktrees', /codecite-worktrees/],
];

const isUnder = (p, dir) => {
  const rel = relative(dir, p);
  return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel));
};

// A command's segments: split on &&, ||, ;, | and newlines.
const segments = (cmd) => cmd.split(/&&|\|\||[;|\n]/);
// A segment's words: split on whitespace, quotes and shell punctuation.
const words = (seg) => seg.split(/[\s"'`()<>=]+/).filter(Boolean);

/**
 * Why the resolved path `abs` taints, or null: it is inside a taint path
 * (and not inside the run's own dir), or it equals or contains the round dir
 * or a taint path (a walk that can reach them, like `ls ../../..`).
 */
function reaches(abs, { taintPaths, ownRunDir, roundDir }) {
  if (ownRunDir && isUnder(abs, ownRunDir)) return null;
  const inside = taintPaths.find((t) => isUnder(abs, t));
  if (inside) return `touches ${inside}`;
  const guarded = roundDir ? [roundDir, ...taintPaths] : taintPaths;
  const above = guarded.find((t) => isUnder(t, abs));
  return above ? `reaches ${above}` : null;
}

// Every word of each segment resolves against a cwd that follows `cd`; a
// segment other than `cd` also reads its cwd. `cd` targets themselves only
// move the cwd. The cwd starts at ctx.cwd for each command.
function shellReach(cmd, ctx) {
  if (!ctx.cwd) return null;
  let cwd = ctx.cwd;
  for (const seg of segments(cmd)) {
    const w = words(seg);
    if (w.length === 0) continue;
    if (w[0] === 'cd') {
      if (w[1] && w[1] !== '-') cwd = resolve(cwd, w[1]);
      continue;
    }
    for (const p of [cwd, ...w.map((x) => resolve(cwd, x))]) {
      const why = reaches(p, ctx);
      if (why) return why;
    }
  }
  return null;
}

/**
 * The reasons a run is tainted; empty when it is clean. One entry per
 * offending command or path, naming the first rule it broke.
 * @param {{shellCommands: string[], readPaths: string[]}} activity
 * @param {{taintPaths: string[], ownRunDir: string, cwd?: string, roundDir?: string}} ctx
 *   `taintPaths` are absolute (round.json); anything under `ownRunDir`
 *   (<round>/runs/<run.id>) is the run's own. Relative paths resolve
 *   against `cwd`, the agent's working directory. `roundDir` defaults to
 *   two levels above `ownRunDir`.
 * @returns {string[]}
 */
export function taintHits({ shellCommands = [], readPaths = [] }, ctx) {
  const c = { ...ctx, taintPaths: ctx.taintPaths ?? [], roundDir: ctx.roundDir ?? (ctx.ownRunDir && dirname(dirname(ctx.ownRunDir))) };
  const hits = [];
  for (const cmd of shellCommands) {
    const net = NETWORK.find(([, re]) => re.test(cmd));
    const hidden = HIDDEN.find(([, re]) => re.test(cmd));
    const why = net?.[0] ?? hidden?.[0] ?? shellReach(cmd, c);
    if (why) hits.push(`shell (${why}): ${cmd.slice(0, 300)}`);
  }
  for (const p of readPaths) {
    const hidden = HIDDEN.find(([, re]) => re.test(p));
    const why = hidden?.[0] ?? (isAbsolute(p) || c.cwd ? reaches(resolve(c.cwd ?? '/', p), c) : null);
    if (why) hits.push(`read (${why}): ${p}`);
  }
  return hits;
}
