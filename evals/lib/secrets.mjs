// Reads the eval keys from the owner's env file. Errors name the file, line
// or variable, never a value.

import { readFileSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

export const SECRETS_PATH = join(homedir(), '.config', 'codecite', 'evals.env');

const VARS = { openrouter: 'OPENROUTER_EVAL_API_KEY', claudeOauth: 'CLAUDE_CODE_OAUTH_TOKEN' };

/** Returns {openrouter, claudeOauth}, each a string or null when unset. */
export function readSecrets({ path = SECRETS_PATH } = {}) {
  let st;
  try {
    st = statSync(path);
  } catch {
    throw new Error(`secrets: ${path} is missing; create it with mode 0600`);
  }
  if (st.mode & 0o077) {
    throw new Error(`secrets: ${path} is readable by group or others; chmod 600 it`);
  }
  const env = {};
  readFileSync(path, 'utf8')
    .split('\n')
    .forEach((raw, i) => {
      const line = raw.trim();
      if (line === '' || line.startsWith('#')) return;
      const m = /^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=(.*)$/.exec(line);
      if (!m) throw new Error(`secrets: ${path} line ${i + 1} is not NAME=value`);
      let value = m[2].trim();
      if (/^(["']).*\1$/.test(value) && value.length >= 2) value = value.slice(1, -1);
      env[m[1]] = value;
    });
  return Object.fromEntries(Object.entries(VARS).map(([k, v]) => [k, env[v] || null]));
}

/** The secret `key` from readSecrets(), or an error naming its variable. */
export function requireSecret(secrets, key) {
  const value = secrets[key];
  if (!value) throw new Error(`secrets: ${VARS[key] ?? key} is not set in ${SECRETS_PATH}`);
  return value;
}
