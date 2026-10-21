// The pi adapter: runs `pi -p --mode json` against OpenRouter with a clean
// start (an empty HOME and agent dir holding only the models.json and
// auth.json we write, no sessions, MCP, extensions, skills, prompt
// templates, themes or project-local .pi resources) and reads its JSON event
// stream. The key is the evals OpenRouter key, in the agent dir's auth.json
// (0600, fed to bwrap from a pipe, never on the host's disk), not in the
// environment, so `env` in the agent's shell doesn't print it; the agent can
// still read the file. Nothing from the owner's
// ~/.pi is read.

import { mkdirSync, writeFileSync } from 'node:fs';
import { isAbsolute, join, relative } from 'node:path';
import { emptyAgentResult } from '../lib/agent.mjs';
import { armLabel } from '../lib/plan.mjs';
import { requireSecret } from '../lib/secrets.mjs';
import { addUsage, emptyUsage } from '../lib/telemetry.mjs';
import { findOnPath, runsCodecite } from './claude.mjs';

export const CONFIG_LEAKED = 'config leaked';

/** The built-in tools both arms get (`pi --help`, "Built-in Tool Names"). */
export const TOOLS = ['read', 'bash', 'edit', 'write', 'grep', 'find', 'ls'];

// pi's tool args that name a path the agent touched. `find`'s pattern is a
// glob that can carry a path, so it is scanned too; grep's is a regex.
const PATH_ARGS = { read: ['path'], edit: ['path'], write: ['path'], grep: ['path'], find: ['path', 'pattern'], ls: ['path'] };

// System-prompt sections a clean start produces. Anything else (`skills`,
// `addendum` from APPEND_SYSTEM.md, or a section a newer pi adds) is treated
// as the owner's config. `project_context` is allowed only for context files
// inside the run's cwd (the task's own AGENTS.md); see configLeak.
const CLEAN_SECTIONS = new Set(['preamble', 'tools', 'rules', 'docs', 'cwd', 'project_context']);

/** The flags every run gets, between `-p` and `--model <model>`. */
export const FLAGS = [
  '--mode',
  'json',
  '--provider',
  'openrouter',
];

/** The isolation flags, after `--model <model>`. */
export const ISOLATION = [
  '--no-session',
  '--no-mcp',
  '--no-extensions',
  '--no-skills',
  '--no-prompt-templates',
  '--no-themes',
  '--no-approve', // ignore project-local .pi resources
  '--tools',
  TOOLS.join(','),
];

const isUnder = (p, dir) => {
  const rel = relative(dir, p);
  return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel));
};

/**
 * "config leaked" when a system message shows the owner's config loaded,
 * else null. pi reports what it loaded in the session's system message
 * (`message_start`/`message_end`, role "system"): `sections` of the system
 * prompt and `toolsAdded`. A leak is a section outside CLEAN_SECTIONS (skills,
 * an appended prompt), a tool outside TOOLS (MCP or extension tools), or a
 * `project_context` naming a context file outside the run's cwd (the agent
 * dir's or a parent directory's AGENTS.md/CLAUDE.md).
 */
export function configLeak(event) {
  if (event?.type !== 'message_start' && event?.type !== 'message_end') return null;
  const m = event.message;
  if (m?.role !== 'system') return null;
  const sections = m.sections && typeof m.sections === 'object' ? m.sections : {};
  if (Object.keys(sections).some((k) => !CLEAN_SECTIONS.has(k))) return CONFIG_LEAKED;
  const tools = Array.isArray(m.toolsAdded) ? m.toolsAdded : [];
  if (tools.some((t) => !TOOLS.includes(t?.name))) return CONFIG_LEAKED;
  if (sections.project_context != null) {
    const cwd = /<cwd>\s*([^\n<]+?)\s*<\/cwd>/.exec(sections.cwd ?? '')?.[1];
    const paths = [...String(sections.project_context).matchAll(/<project_instructions path="([^"]*)"/g)].map((x) => x[1]);
    if (!cwd || !isAbsolute(cwd) || paths.length === 0) return CONFIG_LEAKED;
    if (paths.some((p) => !isAbsolute(p) || !isUnder(p, cwd))) return CONFIG_LEAKED;
  }
  return null;
}

function parseLines(stdout) {
  const events = [];
  for (const raw of stdout.split('\n')) {
    const l = raw.replace(/\r$/, '').trim();
    if (!l.startsWith('{')) continue;
    try {
      events.push(JSON.parse(l));
    } catch {
      // a partial last line from a killed run
    }
  }
  return events;
}

const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);

function usageOf(message) {
  const u = message?.usage ?? {};
  return {
    input: num(u.input),
    output: num(u.output),
    cacheRead: num(u.cacheRead),
    cacheWrite: num(u.cacheWrite),
    totalTokens: num(u.totalTokens),
    costUsd: num(u.cost?.total),
  };
}

/** The HTTP status at the head of pi's errorMessage ("401: {...}", "529 overloaded"), else null. */
export function errorStatus(message) {
  const m = /^\s*(\d{3})\b/.exec(typeof message === 'string' ? message : '');
  return m ? Number(m[1]) : null;
}

function modelsJson(roundId, arm) {
  // OpenRouter attribution, so the dashboard splits spend by arm. pi's own
  // attribution headers are off (PI_TELEMETRY=0), so these are the ones sent.
  // The label is opaque (the scorecard maps it back): the agent can read
  // this file, and nothing it sees may name its arm or round.
  return { providers: { openrouter: { headers: { 'X-Title': armLabel(roundId, arm) } } } };
}

export default {
  name: 'pi',

  command(run, dirs, ctx) {
    const key = requireSecret(ctx.secrets, 'openrouter');
    const cmd = findOnPath('pi', dirs.path);
    if (!cmd) throw new Error(`pi: \`pi\` not found on the run's PATH (${dirs.path})`);
    // pi's agent dir: empty but for models.json and auth.json (the evals
    // key, 0600), so no settings, AGENTS.md, skills, extensions or MCP
    // config of the owner's load. models.json is written on the host; pi
    // sees it at the in-sandbox home.
    const hostAgentDir = join(dirs.host?.home ?? dirs.home, 'pi');
    mkdirSync(hostAgentDir, { recursive: true });
    writeFileSync(join(hostAgentDir, 'models.json'), `${JSON.stringify(modelsJson(ctx.plan?.roundId, run.arm), null, 2)}\n`);

    // The whole environment, built from nothing; no key in it.
    const env = {
      HOME: dirs.home,
      PI_CODING_AGENT_DIR: join(dirs.home, 'pi'),
      XDG_CONFIG_HOME: join(dirs.home, '.config'),
      XDG_CACHE_HOME: join(dirs.home, '.cache'),
      XDG_DATA_HOME: dirs.xdg,
      PATH: dirs.path,
      LANG: 'C.UTF-8',
      PI_TELEMETRY: '0', // no install telemetry, no pi attribution headers
      PI_SKIP_VERSION_CHECK: '1',
    };
    // The prompt goes last, after `--`, so it is a message and never a flag.
    const args = ['-p', ...FLAGS, '--model', run.model, ...ISOLATION, '--', ctx.prompt];
    // auth.json with the key reaches the sandbox from a pipe (bwrap's
    // --bind-data, 0600), so it is never on the host's disk; the empty mount
    // point bwrap leaves in the host agent dir is removed after the run.
    const auth = `${JSON.stringify({ openrouter: { type: 'api_key', key } }, null, 2)}\n`;
    return {
      cmd,
      args,
      env,
      cwd: dirs.project,
      dataFiles: [{ path: join(dirs.home, 'pi', 'auth.json'), data: auth }],
      secretFiles: [join(hostAgentDir, 'auth.json')],
    };
  },

  earlyReject(line) {
    const l = line.trim();
    if (!l.startsWith('{')) return null;
    try {
      return configLeak(JSON.parse(l));
    } catch {
      return null;
    }
  },

  parse(stdout) {
    const r = emptyAgentResult();
    const transcript = [];
    let usage = emptyUsage();
    let assistants = 0;
    let turns = 0;
    let lastStatus = null;
    let lastErrored = false;
    let toolCalls = 0;
    let codeciteCalls = 0;
    let system = null;
    let model = null;

    for (const e of parseLines(stdout)) {
      const leak = configLeak(e);
      if (leak && !r.rejectReason) r.rejectReason = leak;
      if (e.type === 'message_end' && e.message?.role === 'system') {
        system ??= e.message;
      } else if (e.type === 'message_end' && e.message?.role === 'assistant') {
        const m = e.message;
        assistants++;
        model ??= m.model ?? null;
        usage = addUsage(usage, usageOf(m));
        lastErrored = m.stopReason === 'error';
        if (lastErrored) {
          lastStatus = errorStatus(m.errorMessage);
          transcript.push(`[error] ${m.errorMessage ?? ''}`);
        } else {
          turns++;
          // A later good message means an earlier error was retried away.
          lastStatus = null;
        }
        for (const b of Array.isArray(m.content) ? m.content : []) if (b?.type === 'text' && b.text) transcript.push(b.text);
      } else if (e.type === 'auto_retry_start') {
        transcript.push(`[retry ${e.attempt ?? '?'}: ${e.errorMessage ?? ''}]`);
      } else if (e.type === 'tool_execution_start') {
        toolCalls++;
        const args = e.args && typeof e.args === 'object' ? e.args : {};
        if (e.toolName === 'bash' && typeof args.command === 'string') {
          r.shellCommands.push(args.command);
          if (runsCodecite(args.command)) codeciteCalls++;
          transcript.push(`$ ${args.command}`);
        } else {
          for (const f of PATH_ARGS[e.toolName] ?? []) if (typeof args[f] === 'string' && args[f]) r.readPaths.push(args[f]);
          transcript.push(`[${e.toolName}] ${JSON.stringify(args)}`);
        }
      }
    }

    if (system || model) {
      r.init = {
        model,
        tools: Array.isArray(system?.toolsAdded) ? system.toolsAdded.map((t) => t?.name) : null,
        sections: system?.sections ? Object.keys(system.sections) : null,
      };
    }
    r.usage = usage;
    r.turns = assistants > 0 ? turns : null;
    r.toolCalls = toolCalls;
    r.codeciteCalls = codeciteCalls;
    r.providerStatus = lastStatus;
    // pi exits 0 even when the provider ended the run; say so for the call loop.
    r.endedOnError = lastErrored;
    // A 429 at the end: the provider's rate limit, not the agent's failure.
    r.rateLimited = lastErrored && lastStatus === 429;
    r.transcript = transcript.join('\n');
    return r;
  },
};
