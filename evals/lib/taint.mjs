// Taint: did a run reach outside its own sandbox? Only the shell commands an
// agent ran and the paths its file tools touched are scanned, never the
// contents of what it read, so a README mentioning github.com is fine.
// Paths are the agent's own, in-sandbox ones (its cwd is /work/project);
// the round dir and the real repos are hidden by the sandbox, so a path
// taints only when it names something inside a taint path (an attempt that
// found an empty dir), never for merely walking up to `/`.

import { basename, isAbsolute, relative, resolve } from 'node:path';

// Fetching code or packages from outside: shell commands only. curl and
// wget are checked per segment (see fetchesOutside): loopback is fine.
// `pip install` matches `uv pip install` and `python -m pip install` too:
// the spec lists it, and the prompt says "no new packages".
const NETWORK = [
  ['github.com', /github\.com/],
  ['githubusercontent.com', /githubusercontent\.com/],
  ['git clone', /\bgit\s+clone\b/],
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

// A command's segments: split on &&, ||, ;, |, & and newlines.
const segments = (cmd) => cmd.split(/&&|\|\||[;|&\n]/);
// A segment's words: split on whitespace, quotes and shell punctuation.
const words = (seg) => seg.split(/[\s"'`()<>=]+/).filter(Boolean);

const LOOPBACK_HOST = /^(localhost|127\.\d{1,3}\.\d{1,3}\.\d{1,3}|\[::1\]|::1)$/i;
// A host with an optional port and path, no scheme: localhost:8000/x.
const BARE_LOOPBACK = /^(localhost|127\.\d{1,3}\.\d{1,3}\.\d{1,3}|\[::1\])(:\d+)?(\/|$)/i;
// A bare dotted name that reads as a host (example.org/x), not a file (out.json).
const BARE_HOST = /^[a-z0-9-]+(\.[a-z0-9-]+)*\.[a-z]{2,}(:\d+)?(\/\S*)?$/i;
const FILE_EXT = /\.(json|txt|html?|py|log|xml|md|sh|js|ya?ml|toml|csv|out|err|cfg|ini)$/i;
const OUTPUT_FLAGS = new Set(['-o', '--output', '-O', '--output-document', '-D', '--dump-header', '-c', '--cookie-jar', '-b', '--cookie', '-K', '--config']);

function hostOf(url) {
  try {
    return new URL(url).hostname;
  } catch {
    return null;
  }
}

/**
 * True when a segment that runs curl or wget fetches from anything but
 * loopback (127.0.0.0/8, localhost, ::1). A fetch naming no host it can
 * read (`curl $URL`) counts as outside.
 */
function fetchesOutside(seg) {
  const w = words(seg);
  const at = w.findIndex((x) => ['curl', 'wget'].includes(basename(x)));
  if (at < 0) return false;
  let targets = 0;
  for (let i = at + 1; i < w.length; i++) {
    const x = w[i];
    if (OUTPUT_FLAGS.has(w[i - 1])) continue; // a file name, not a host
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(x)) {
      targets++;
      const host = hostOf(x);
      if (!host || !LOOPBACK_HOST.test(host)) return true;
    } else if (BARE_LOOPBACK.test(x)) {
      targets++;
    } else if (!x.startsWith('-') && BARE_HOST.test(x) && !FILE_EXT.test(x.split('/')[0])) {
      return true;
    }
  }
  return targets === 0;
}

/** Why the resolved path `abs` taints, or null: it is inside a taint path (and not the run's own). */
function reaches(abs, { taintPaths, ownRunDir }) {
  if (ownRunDir && isUnder(abs, ownRunDir)) return null;
  const inside = taintPaths.find((t) => isUnder(abs, t));
  return inside ? `touches ${inside}` : null;
}

// Every word of each segment resolves against a cwd that follows `cd`.
// The cwd starts at ctx.cwd for each command.
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
    for (const x of w) {
      const why = reaches(resolve(cwd, x), ctx);
      if (why) return why;
    }
  }
  return null;
}

/**
 * The reasons a run is tainted; empty when it is clean. One entry per
 * offending command or path, naming the first rule it broke.
 * @param {{shellCommands: string[], readPaths: string[]}} activity
 * @param {{taintPaths: string[], ownRunDir?: string, cwd?: string}} ctx
 *   `taintPaths` are absolute (round.json); anything under `ownRunDir` (the
 *   sandbox's /work) is the run's own. Relative paths resolve against
 *   `cwd`, the agent's in-sandbox working directory.
 * @returns {string[]}
 */
export function taintHits({ shellCommands = [], readPaths = [] }, ctx) {
  const c = { ...ctx, taintPaths: ctx.taintPaths ?? [] };
  const hits = [];
  for (const cmd of shellCommands) {
    const net = NETWORK.find(([, re]) => re.test(cmd))?.[0] ?? (segments(cmd).some(fetchesOutside) ? 'curl/wget outside loopback' : null);
    const hidden = HIDDEN.find(([, re]) => re.test(cmd))?.[0];
    const why = net ?? hidden ?? shellReach(cmd, c);
    if (why) hits.push(`shell (${why}): ${cmd.slice(0, 300)}`);
  }
  for (const p of readPaths) {
    const hidden = HIDDEN.find(([, re]) => re.test(p));
    const why = hidden?.[0] ?? (isAbsolute(p) || c.cwd ? reaches(resolve(c.cwd ?? '/', p), c) : null);
    if (why) hits.push(`read (${why}): ${p}`);
  }
  return hits;
}
