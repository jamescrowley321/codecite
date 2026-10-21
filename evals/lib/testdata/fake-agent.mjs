// A stand-in coding agent for call.test.mjs: `node fake-agent.mjs <mode>`.
// It prints JSON lines; the last {"type":"result"} line is what the fake
// adapter parses. It echoes FAKE_KEY so tests can prove the harness redacts.
import { appendFileSync, writeFileSync } from 'node:fs';

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
    result({ providerStatus: 429, turns: 2, endedOnError: true }, 0);
    break;
  case 'errstart':
    result({ providerStatus: 429, turns: 0, endedOnError: true }, 0);
    break;
  case 'errnostatus':
    result({ providerStatus: null, turns: 0, endedOnError: true }, 0);
    break;
  case 'taint':
    result({ providerStatus: 200, turns: 3, reads: ['app/main.py'], shell: ['cat ../../../plan.json'] }, 0);
    break;
  default:
    process.stderr.write(`fake-agent: unknown mode ${mode}\n`);
    process.exitCode = 2;
}
