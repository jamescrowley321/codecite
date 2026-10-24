// The Claude Code adapter: runs `claude -p` with a clean start (an empty
// HOME and config dir, no settings files, no MCP servers) and reads its
// stream-json output. Auth is the owner's subscription token: from the evals
// env file when set, else the owner's Claude Code login. ANTHROPIC_API_KEY is
// never passed.

import { accessSync, constants, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, isAbsolute, join } from 'node:path';
import { emptyAgentResult } from '../lib/agent.mjs';
import { SECRETS_PATH } from '../lib/secrets.mjs';

/** The owner's Claude Code login, read (never written) when evals.env has no token. */
export const CREDENTIALS_PATH = join(homedir(), '.claude', '.credentials.json');

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

/** The first executable `name` in the absolute dirs of `path`, else null. */
export function findOnPath(name, path) {
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

/** Margin on top of a run's timeout that a login must still be valid for. */
export const EXPIRY_MARGIN_MS = 60_000;

/**
 * The subscription token: `secrets.claudeOauth` if set, else
 * `claudeAiOauth.accessToken` from `credentialsPath`, read fresh each call so
 * a login Claude Code refreshed mid-round is picked up. Errors name the
 * sources, never a value.
 */
export function claudeToken(secrets, credentialsPath) {
  return claudeLogin(secrets, credentialsPath).token;
}

/**
 * claudeToken plus, from the login file, its `refreshToken` (to redact,
 * never to pass) and `expiresAt` (ms since the epoch), each null when
 * absent or when the token came from evals.env.
 */
export function claudeLogin(secrets, credentialsPath) {
  if (secrets?.claudeOauth) return { token: secrets.claudeOauth, refreshToken: null, expiresAt: null };
  const none = `claude: CLAUDE_CODE_OAUTH_TOKEN is not set in ${SECRETS_PATH}, and`;
  let text;
  try {
    text = readFileSync(credentialsPath, 'utf8');
  } catch (e) {
    throw new Error(`${none} ${credentialsPath} is missing or unreadable (${e.code ?? 'error'}); run \`claude\` and log in`);
  }
  let creds;
  try {
    creds = JSON.parse(text);
  } catch {
    throw new Error(`${none} ${credentialsPath} is not valid JSON`);
  }
  const token = creds?.claudeAiOauth?.accessToken;
  if (typeof token !== 'string' || !token) {
    throw new Error(`${none} ${credentialsPath} has no claudeAiOauth.accessToken; run \`claude\` and log in`);
  }
  const refresh = creds.claudeAiOauth.refreshToken;
  const expiresAt = creds.claudeAiOauth.expiresAt;
  return {
    token,
    refreshToken: typeof refresh === 'string' && refresh ? refresh : null,
    expiresAt: typeof expiresAt === 'number' && Number.isFinite(expiresAt) ? expiresAt : null,
  };
}

const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);

/**
 * Usage summed over the assistant messages, for a run with no result event
 * (killed at timeout). stream-json repeats a message's usage on each of its
 * content blocks, so each message id counts once (its last report). Cost is
 * only in the result event: null.
 */
function usageFromMessages(events) {
  const byId = new Map();
  let anon = 0;
  for (const e of events) {
    if (e.type !== 'assistant' || !e.message?.usage) continue;
    byId.set(e.message.id ?? `anon-${anon++}`, e.message.usage);
  }
  if (byId.size === 0) return null;
  const sum = (f) => {
    let total = 0;
    for (const u of byId.values()) {
      const n = num(u[f]);
      if (n == null) return null;
      total += n;
    }
    return total;
  };
  const usage = { input: sum('input_tokens'), output: sum('output_tokens'), cacheRead: sum('cache_read_input_tokens'), cacheWrite: sum('cache_creation_input_tokens'), totalTokens: null, costUsd: null };
  const parts = [usage.input, usage.output, usage.cacheRead, usage.cacheWrite];
  if (parts.every((n) => n != null)) usage.totalTokens = parts.reduce((a, b) => a + b, 0);
  return usage;
}

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

/** The adapter, reading the fallback login from `credentialsPath`. */
export function createClaude({ credentialsPath = CREDENTIALS_PATH } = {}) {
  return {
    name: 'claude',

    command(run, dirs, ctx) {
      const { token, refreshToken, expiresAt } = claudeLogin(ctx.secrets, credentialsPath);
      // The token is never refreshed here; one that would expire mid-run is
      // a 401 (fatal) partway through, so stop before the run instead.
      const needMs = (ctx.plan?.timeoutSec ?? 0) * 1000 + EXPIRY_MARGIN_MS;
      if (expiresAt != null && expiresAt < Date.now() + needMs) {
        const when = expiresAt < Date.now() ? 'expired' : 'expires before this run could finish';
        throw new Error(`claude: the Claude Code login in ${credentialsPath} ${when} (at ${new Date(expiresAt).toISOString()}); log in to Claude Code to refresh it (run \`claude\`), then run call again to resume`);
      }
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
      // `secrets`: the harness redacts the token even when it came from the
      // login file rather than ctx.secrets, and the login's refresh token.
      return { cmd, args: ['-p', ctx.prompt, '--model', run.model, ...FLAGS], env, cwd: dirs.project, secrets: [token, refreshToken].filter(Boolean) };
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
      let limitStatus = null;

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
        } else if (e.type === 'rate_limit_event') {
          limitStatus = e.rate_limit_info?.status ?? limitStatus;
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
        // An error result (a 429 usage limit, a 5xx) ended the run: before
        // any turn that is infra (classify), after one a failed run.
        r.endedOnError = result.is_error === true;
        r.providerError = r.endedOnError && typeof result.result === 'string' ? result.result.slice(0, 1000) : null;
      } else {
        r.usage = usageFromMessages(events) ?? r.usage;
        r.providerStatus = lastStatus;
      }
      // Ended on a rate or usage limit: a 429, a rejected rate_limit_event,
      // or the limit message in the error result.
      const text = typeof result?.result === 'string' ? result.result : '';
      r.rateLimited =
        (r.endedOnError || !result) &&
        (r.providerStatus === 429 || limitStatus === 'rejected' || (r.endedOnError && /usage limit|hit your (usage )?limit|rate.?limit/i.test(text)));
      return r;
    },
  };
}

export default createClaude();
