// The eval IdP (evals/fixtures/node-oidc) for the score phase: started with
// this Node on a free loopback port, outside the sandbox. Sandboxed scoring
// reaches it over the shared network.

import { spawn } from 'node:child_process';
import { cpSync, existsSync } from 'node:fs';
import { createServer } from 'node:net';
import { join } from 'node:path';
import { REPO_ROOT } from './workspace.mjs';

export const IDP_DIR = join(REPO_ROOT, 'evals', 'fixtures', 'node-oidc');

/**
 * Freeze the IdP with the round, as compose does the task: copy the fixture
 * (provider.js, its lockfile and installed node_modules) into <round>/idp,
 * so scoring never runs the live checkout's. Returns the copy's dir.
 */
export function freezeIdp(roundDir, { from = IDP_DIR } = {}) {
  if (!existsSync(join(from, 'node_modules'))) throw new Error(`idp: no node_modules in ${from}; run npm ci there before compose`);
  const dest = join(roundDir, 'idp');
  cpSync(from, dest, { recursive: true, verbatimSymlinks: true });
  return dest;
}

/**
 * The IdP a round scores against: its frozen copy. A task that names an IdP
 * (`plan.taskDef.idp`) and has none is a setup error, never a fallback to
 * the live fixture; a task without that field uses the live fixture.
 */
export function idpDirFor(roundDir, plan) {
  const frozen = join(roundDir, 'idp');
  if (existsSync(join(frozen, 'provider.js'))) return frozen;
  if (plan?.taskDef?.idp) throw new Error(`idp: no frozen IdP at ${frozen}, though the task names "${plan.taskDef.idp}"; compose freezes it (run npm ci in ${IDP_DIR}, then compose the round again)`);
  return IDP_DIR;
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.once('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

/**
 * Start the IdP and wait for its discovery document. Returns {url, stop}.
 * Throws when its node_modules are missing or it doesn't come up.
 */
export async function startIdp({ dir = IDP_DIR, timeoutMs = 20_000 } = {}) {
  if (!existsSync(join(dir, 'node_modules'))) throw new Error(`idp: no node_modules in ${dir}; run npm ci there`);
  const port = await freePort();
  const url = `http://127.0.0.1:${port}`;
  const log = [];
  const child = spawn(process.execPath, [join(dir, 'provider.js')], {
    cwd: dir,
    env: { PATH: process.env.PATH ?? '', PORT: String(port) },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  for (const s of [child.stdout, child.stderr]) s.on('data', (c) => log.push(c));
  let exited = null;
  child.on('exit', (code, signal) => {
    exited = code ?? signal;
  });
  const stop = () =>
    new Promise((res) => {
      if (exited !== null) return res();
      child.once('exit', () => res());
      child.kill('SIGTERM');
      setTimeout(() => child.kill('SIGKILL'), 3000).unref();
    });
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (exited !== null) throw new Error(`idp: exited early (${exited}): ${Buffer.concat(log).toString('utf8').slice(-1000)}`);
    try {
      const res = await fetch(`${url}/.well-known/openid-configuration`);
      if (res.status === 200) return { url, stop };
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  await stop();
  throw new Error(`idp: not ready at ${url} after ${timeoutMs} ms`);
}
