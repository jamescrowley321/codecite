// Taint: did a run reach outside its own sandbox? Only the shell commands an
// agent ran and the paths its file tools touched are scanned, never the
// contents of what it read, so a README mentioning github.com is fine.

import { isAbsolute, relative, resolve } from 'node:path';

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

// Path-ish words of a shell command: split on whitespace, quotes and shell
// punctuation; keep words that are absolute, start with ./ or ../, or hold a /.
function pathWords(cmd) {
  return cmd
    .split(/[\s"'`;&|()<>=]+/)
    .filter((w) => w && (w.includes('/') || w === '..' || w === '.'));
}

function tainting(p, { taintPaths, ownRunDir, cwd }) {
  if (!isAbsolute(p) && !cwd) return null;
  const abs = resolve(cwd ?? '/', p);
  if (ownRunDir && isUnder(abs, ownRunDir)) return null;
  return taintPaths.find((t) => isUnder(abs, t)) ?? null;
}

/**
 * The reasons a run is tainted; empty when it is clean. One entry per
 * offending command or path, naming the first rule it broke.
 * @param {{shellCommands: string[], readPaths: string[]}} activity
 * @param {{taintPaths: string[], ownRunDir: string, cwd?: string}} ctx
 *   `taintPaths` are absolute (round.json); anything under `ownRunDir`
 *   (<round>/runs/<run.id>) is the run's own. Relative paths resolve
 *   against `cwd`, the agent's working directory.
 * @returns {string[]}
 */
export function taintHits({ shellCommands = [], readPaths = [] }, ctx) {
  const hits = [];
  for (const cmd of shellCommands) {
    const net = NETWORK.find(([, re]) => re.test(cmd));
    const hidden = HIDDEN.find(([, re]) => re.test(cmd));
    const path = pathWords(cmd).map((w) => tainting(w, ctx)).find(Boolean);
    const why = net?.[0] ?? hidden?.[0] ?? (path && `touches ${path}`);
    if (why) hits.push(`shell (${why}): ${cmd.slice(0, 300)}`);
  }
  for (const p of readPaths) {
    const hidden = HIDDEN.find(([, re]) => re.test(p));
    const path = tainting(p, ctx);
    const why = hidden?.[0] ?? (path && `touches ${path}`);
    if (why) hits.push(`read (${why}): ${p}`);
  }
  return hits;
}
