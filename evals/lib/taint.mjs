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
  // The mount table's source column names the host dirs behind /work (the
  // round, the run, the arm); /proc/mounts and df don't show it.
  ['mountinfo', /mountinfo/],
  ['findmnt', /\bfindmnt\b/],
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

const REDIRECT = Symbol('redirect');

/**
 * A command's simple commands, each as its words, split the way the shell
 * does: on &&, ||, ;, |, & and newlines outside quotes, with quotes removed
 * and backslash escapes kept literal. A redirection (`>`, `2>&1`, `<`) is a
 * REDIRECT marker before its target. Lexical only: no expansion.
 */
export function simpleCommands(cmd) {
  const out = [];
  let words = [];
  let word = null;
  const endWord = () => {
    if (word !== null) words.push(word);
    word = null;
  };
  const endCmd = () => {
    endWord();
    if (words.length) out.push(words);
    words = [];
  };
  for (let i = 0; i < cmd.length; i++) {
    const c = cmd[i];
    if (c === "'") {
      const j = cmd.indexOf("'", i + 1);
      word = (word ?? '') + cmd.slice(i + 1, j < 0 ? cmd.length : j);
      i = j < 0 ? cmd.length : j;
    } else if (c === '"') {
      let j = i + 1;
      let buf = '';
      while (j < cmd.length && cmd[j] !== '"') {
        if (cmd[j] === '\\' && j + 1 < cmd.length) j++;
        buf += cmd[j++];
      }
      word = (word ?? '') + buf;
      i = j;
    } else if (c === '\\' && i + 1 < cmd.length) {
      word = (word ?? '') + cmd[++i];
    } else if (c === ' ' || c === '\t') {
      endWord();
    } else if (c === '\n' || c === ';' || c === '|' || c === '&') {
      endCmd();
      if ((c === '|' || c === '&') && cmd[i + 1] === c) i++;
    } else if (c === '>' || c === '<') {
      // A leading fd number belongs to the redirection, not to the args.
      if (word !== null && /^\d+$/.test(word)) word = null;
      endWord();
      words.push(REDIRECT);
      while (cmd[i + 1] === '>' || cmd[i + 1] === '&' || cmd[i + 1] === '<') i++;
    } else {
      word = (word ?? '') + c;
    }
  }
  endCmd();
  return out;
}

// Commands that run the next word as a command, and their own args to skip.
const WRAPPERS = new Set(['sudo', 'env', 'timeout', 'command', 'nohup', 'time', 'nice', 'stdbuf', 'exec', 'xargs']);
// Options that take a value, per fetcher: their value is never a target.
const VALUED = {
  curl: new Set(['-d', '--data', '--data-raw', '--data-binary', '--data-urlencode', '--data-ascii', '--json', '-H', '--header', '-F', '--form', '--form-string', '-u', '--user', '-A', '--user-agent', '-e', '--referer', '-X', '--request', '-o', '--output', '-b', '--cookie', '-c', '--cookie-jar', '-K', '--config', '-m', '--max-time', '--connect-timeout', '-w', '--write-out', '-T', '--upload-file', '-D', '--dump-header', '--retry', '--retry-delay', '-x', '--proxy', '-r', '--range', '-E', '--cert', '--cacert', '--key']),
  wget: new Set(['-O', '--output-document', '-o', '--output-file', '-a', '--append-output', '-P', '--directory-prefix', '--header', '-U', '--user-agent', '-t', '--tries', '-T', '--timeout', '--post-data', '--post-file', '--body-data', '--body-file', '--method', '-e', '--execute', '--user', '--password']),
};

function hostOf(url) {
  try {
    return new URL(url).hostname;
  } catch {
    return null;
  }
}

/**
 * True when a simple command (its words) runs curl or wget against anything
 * but loopback (127.0.0.0/8, localhost, ::1). curl or wget must be the
 * command: its first word, after any VAR=value and wrappers (sudo, env,
 * timeout, ...), so `which curl` or `grep curl` don't count. Option values
 * (-d, -H, -o, ...) and redirection targets are not targets. A target it
 * can't read (`curl $URL`), or one fed by xargs, counts as outside; no target
 * at all (`curl --version`) is no fetch.
 */
function fetchesOutside(words) {
  let i = 0;
  let viaXargs = false;
  const skipAssign = () => {
    while (i < words.length && typeof words[i] === 'string' && /^[A-Za-z_][A-Za-z0-9_]*=/.test(words[i])) i++;
  };
  skipAssign();
  while (i < words.length && WRAPPERS.has(basename(String(words[i])))) {
    const w = basename(words[i++]);
    if (w === 'xargs') viaXargs = true;
    while (i < words.length && typeof words[i] === 'string' && words[i].startsWith('-')) i++;
    if (w === 'timeout' && /^\d/.test(String(words[i] ?? ''))) i++;
    skipAssign();
  }
  const tool = basename(String(words[i] ?? ''));
  if (tool !== 'curl' && tool !== 'wget') return false;
  if (viaXargs) return true;
  for (let j = i + 1; j < words.length; j++) {
    const x = words[j];
    if (x === REDIRECT) {
      j++;
      continue;
    }
    if (x.startsWith('-')) {
      if (VALUED[tool].has(x)) j++;
      continue;
    }
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(x)) {
      const host = hostOf(x);
      if (!host || !LOOPBACK_HOST.test(host)) return true;
    } else if (!BARE_LOOPBACK.test(x)) {
      return true;
    }
  }
  return false;
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
    const net = NETWORK.find(([, re]) => re.test(cmd))?.[0] ?? (simpleCommands(cmd).some(fetchesOutside) ? 'curl/wget outside loopback' : null);
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
