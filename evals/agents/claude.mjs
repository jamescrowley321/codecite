// The Claude Code adapter: runs `claude -p` with a clean start (an empty
// HOME and config dir, no settings files, no MCP servers) and reads its
// stream-json output. Auth is the owner's subscription token from the evals
// env file; ANTHROPIC_API_KEY is never passed.

import { accessSync, constants } from 'node:fs';
import { delimiter, isAbsolute, join } from 'node:path';
import { emptyAgentResult } from '../lib/agent.mjs';
import { requireSecret } from '../lib/secrets.mjs';

export const CONFIG_LEAKED = 'config leaked';

// Tools whose input names a path the agent touched, and the fields holding it.
const PATH_TOOLS = new Set(['Read', 'Edit', 'Write', 'Grep', 'Glob', 'NotebookEdit']);
const PATH_FIELDS = ['file_path', 'path', 'notebook_path'];

/** The flags every run gets, after `-p <prompt> --model <model>`. */
export const FLAGS = [
  '--output-format',
  'stream-json',
  '--verbose', // stream-json with -p requires it
  '--strict-mcp-config',
  '--mcp-config',
  '{"mcpServers":{}}',
  '--setting-sources',
  '', // load no user, project or local settings
  '--disallowedTools',
  'WebFetch,WebSearch',
  '--dangerously-skip-permissions',
  '--no-session-persistence',
];

function findOnPath(name, path) {
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

/**
 * "config leaked" when a stream event shows the owner's own config loaded,
 * else null: a hook event (hooks only come from settings files), or an init
 * event listing an MCP server, an MCP tool, a non-builtin plugin, or a
 * plugin-namespaced (`plugin:name`) skill or slash command.
 */
export function configLeak(event) {
  if (event?.type !== 'system') return null;
  if (typeof event.subtype === 'string' && event.subtype.startsWith('hook_')) return CONFIG_LEAKED;
  if (event.subtype !== 'init') return null;
  const list = (v) => (Array.isArray(v) ? v : []);
  if (list(event.mcp_servers).length > 0) return CONFIG_LEAKED;
  if (list(event.plugins).some((p) => p?.path !== 'builtin')) return CONFIG_LEAKED;
  if (list(event.tools).some((t) => typeof t === 'string' && t.startsWith('mcp__'))) return CONFIG_LEAKED;
  const namespaced = (n) => typeof n === 'string' && n.includes(':');
  if (list(event.skills).some(namespaced) || list(event.slash_commands).some(namespaced)) return CONFIG_LEAKED;
  return null;
}

/** True when a shell command runs `codecite` at the head of any segment. */
export function runsCodecite(command) {
  return command.split(/&&|\|\||[;|&\n]/).some((segment) => {
    const words = segment.trim().split(/\s+/);
    let i = 0;
    while (i < words.length && (/^[A-Za-z_][A-Za-z0-9_]*=/.test(words[i]) || words[i] === 'env')) i++;
    const head = words[i] ?? '';
    return head === 'codecite' || head.endsWith('/codecite');
  });
}

function parseLines(stdout) {
  const events = [];
  for (const raw of stdout.split('\n')) {
    const l = raw.trim();
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

function usageFrom(result) {
  const u = result?.usage ?? {};
  const usage = {
    input: num(u.input_tokens),
    output: num(u.output_tokens),
    cacheRead: num(u.cache_read_input_tokens),
    cacheWrite: num(u.cache_creation_input_tokens),
    totalTokens: null,
    costUsd: num(result?.total_cost_usd),
  };
  const parts = [usage.input, usage.output, usage.cacheRead, usage.cacheWrite];
  if (parts.every((n) => n != null)) usage.totalTokens = parts.reduce((a, b) => a + b, 0);
  return usage;
}

export default {
  name: 'claude',

  command(run, dirs, ctx) {
    const token = requireSecret(ctx.secrets, 'claudeOauth');
    const cmd = findOnPath('claude', dirs.path);
    if (!cmd) throw new Error(`claude: \`claude\` not found on the run's PATH (${dirs.path})`);
    // The whole environment, built from nothing: no ANTHROPIC_API_KEY, no
    // owner HOME, nothing inherited.
    const env = {
      HOME: dirs.home,
      CLAUDE_CONFIG_DIR: dirs.home,
      XDG_CONFIG_HOME: join(dirs.home, '.config'),
      XDG_CACHE_HOME: join(dirs.home, '.cache'),
      XDG_DATA_HOME: dirs.xdg,
      PATH: dirs.path,
      LANG: 'C.UTF-8',
      DISABLE_AUTOUPDATER: '1',
      CLAUDE_CODE_OAUTH_TOKEN: token,
    };
    delete env.ANTHROPIC_API_KEY;
    return { cmd, args: ['-p', ctx.prompt, '--model', run.model, ...FLAGS], env, cwd: dirs.project };
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
    const events = parseLines(stdout);
    const transcript = [];
    let result = null;
    let lastStatus = null;
    let toolCalls = 0;
    let codeciteCalls = 0;
    let assistantOk = 0;

    for (const e of events) {
      const leak = configLeak(e);
      if (leak && !r.rejectReason) r.rejectReason = leak;
      if (e.type === 'system' && e.subtype === 'init') {
        r.init ??= e;
      } else if (e.type === 'system' && e.subtype === 'api_retry') {
        lastStatus = num(e.error_status) ?? lastStatus;
        transcript.push(`[retry ${e.attempt ?? '?'}: ${e.error_status ?? '?'} ${e.error ?? ''}]`);
      } else if (e.type === 'assistant') {
        if (e.error) lastStatus = num(e.message?.api_error_status ?? e.api_error_status) ?? lastStatus;
        else assistantOk++;
        for (const b of Array.isArray(e.message?.content) ? e.message.content : []) {
          if (b?.type === 'text' && b.text) transcript.push(b.text);
          if (b?.type !== 'tool_use') continue;
          toolCalls++;
          const input = b.input ?? {};
          if (b.name === 'Bash' && typeof input.command === 'string') {
            r.shellCommands.push(input.command);
            if (runsCodecite(input.command)) codeciteCalls++;
            transcript.push(`$ ${input.command}`);
          } else {
            if (PATH_TOOLS.has(b.name)) {
              for (const f of PATH_FIELDS) if (typeof input[f] === 'string' && input[f]) r.readPaths.push(input[f]);
            }
            transcript.push(`[${b.name}] ${JSON.stringify(input)}`);
          }
        }
      } else if (e.type === 'result') {
        result = e;
        if (typeof e.result === 'string') transcript.push(`[result${e.is_error ? ' error' : ''}] ${e.result}`);
      }
    }

    r.toolCalls = toolCalls;
    r.codeciteCalls = codeciteCalls;
    r.transcript = transcript.join('\n');
    if (result) {
      r.usage = usageFrom(result);
      r.turns = num(result.num_turns);
      // A run that only ever got an API error took no real turn.
      if (result.is_error && assistantOk === 0) r.turns = 0;
      // The result reports the status that ended the run; a retry that
      // later succeeded leaves it null.
      r.providerStatus = num(result.api_error_status);
    } else {
      r.providerStatus = lastStatus;
    }
    return r;
  },
};
