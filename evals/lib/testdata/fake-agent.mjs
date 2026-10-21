// A stand-in coding agent for call.test.mjs: `node fake-agent.mjs <mode>`.
// It prints JSON lines; the last {"type":"result"} line is what the fake
// adapter parses. It echoes FAKE_KEY so tests can prove the harness redacts.
import { accessSync, appendFileSync, constants, readFileSync, writeFileSync } from 'node:fs';

const mode = process.argv[2];
if (process.env.FAKE_COUNTER) appendFileSync(process.env.FAKE_COUNTER, `${mode}\n`);
const say = (o) => process.stdout.write(`${JSON.stringify(o)}\n`);
say({ type: 'init', key: process.env.FAKE_KEY ?? null, oauth: process.env.FAKE_OAUTH ?? null, prompt: process.env.FAKE_PROMPT ?? null });
process.stderr.write(`debug: key=${process.env.FAKE_KEY}\n`);
const init = { leak: process.env.LEAK_SENTINEL !== undefined };

const result = (fields, code) => {
  say({ type: 'result', init, ...fields });
  process.exitCode = code;
};

switch (mode) {
  case 'ok':
    result({ providerStatus: 200, turns: 2, reads: ['app/main.py'], shell: ['uv run pytest -q'] }, 0);
    break;
  case 'hang':
    if (process.env.FAKE_PIDFILE) writeFileSync(process.env.FAKE_PIDFILE, String(process.pid));
    process.on('SIGTERM', () => {});
    setInterval(() => {}, 1000);
    break;
  case 'earlyreject':
    // Pid and the SIGTERM trap first: the harness kills on the next line.
    if (process.env.FAKE_PIDFILE) writeFileSync(process.env.FAKE_PIDFILE, String(process.pid));
    process.on('SIGTERM', () => {});
    say({ type: 'init', earlyReject: true });
    setInterval(() => {}, 1000);
    break;
  case 'term':
    process.on('SIGTERM', () => {
      writeFileSync(process.env.FAKE_TERMFILE, 'yes\n');
      process.exit(0);
    });
    setInterval(() => {}, 1000);
    break;
  case 'peek': {
    // Which of FAKE_PEEK's paths (newline-separated) this agent can read.
    const readable = (process.env.FAKE_PEEK ?? '').split('\n').filter(Boolean).filter((p) => {
      try {
        accessSync(p, constants.R_OK);
        return true;
      } catch {
        return false;
      }
    });
    writeFileSync('peek.json', JSON.stringify(readable));
    result({ providerStatus: 200, turns: 1 }, 0);
    break;
  }
  case 'slow':
    setInterval(() => {}, 1000);
    break;
  case 'fatal':
    result({ providerStatus: 401, turns: 0 }, 1);
    break;
  case 'reject':
    result({ providerStatus: null, turns: 0, rejectReason: 'user config leaked into the run' }, 0);
    break;
  case 'infra':
    result({ providerStatus: 503, turns: 0 }, 1);
    break;
  case 'error':
    result({ providerStatus: 429, turns: 2 }, 1);
    break;
  // Agents like pi exit 0 even when the provider ended the run.
  case 'errend':
    result({ providerStatus: null, turns: 2, endedOnError: true, providerError: 'Provider finish_reason: error' }, 0);
    break;
  case 'errend-once': {
    const first = readFileSync(process.env.FAKE_COUNTER, 'utf8').trim().split('\n').length === 1;
    result(first ? { providerStatus: null, turns: 1, endedOnError: true, providerError: 'Provider returned an empty response' } : { providerStatus: 200, turns: 2 }, 0);
    break;
  }
  case 'errstart':
    result({ providerStatus: 429, turns: 0, endedOnError: true }, 0);
    break;
  case 'errnostatus':
    result({ providerStatus: null, turns: 0, endedOnError: true }, 0);
    break;
  case 'ratelimit':
    result({ providerStatus: 429, turns: 4, endedOnError: true, rateLimited: true }, 1);
    break;
  case 'ratelimit-once': {
    // Rate-limited on its first call only (the counter file has one line).
    const first = readFileSync(process.env.FAKE_COUNTER, 'utf8').trim().split('\n').length === 1;
    result(first ? { providerStatus: 429, turns: 4, endedOnError: true, rateLimited: true } : { providerStatus: 200, turns: 2 }, first ? 1 : 0);
    break;
  }
  case 'notokens':
    result({ providerStatus: 200, turns: 1, tokens: null }, 0);
    break;
  case 'taint':
    // The round's host path (argv[3]): hidden in the sandbox, but the attempt taints.
    result({ providerStatus: 200, turns: 3, reads: ['app/main.py'], shell: [`cat ${process.argv[3]}/plan.json`] }, 0);
    break;
  default:
    process.stderr.write(`fake-agent: unknown mode ${mode}\n`);
    process.exitCode = 2;
}
