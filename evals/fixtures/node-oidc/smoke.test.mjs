import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
let child;
let base;

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

async function waitForDiscovery(url, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error("provider exited early");
    try {
      const res = await fetch(url);
      if (res.status === 200) return;
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("provider did not become ready");
}

async function token(clientId, resource) {
  const res = await fetch(`${base}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "client_credentials",
      client_id: clientId,
      client_secret: "eval-secret",
      ...(resource ? { resource } : {}),
    }),
  });
  assert.equal(res.status, 200, await res.clone().text());
  const { access_token } = await res.json();
  return JSON.parse(
    Buffer.from(access_token.split(".")[1], "base64url").toString(),
  );
}

before(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}`;
  child = spawn(process.execPath, [join(here, "provider.js")], {
    env: { ...process.env, PORT: String(port) },
    stdio: "inherit",
  });
  await waitForDiscovery(`${base}/.well-known/openid-configuration`);
});

after(() => {
  child?.kill("SIGTERM");
});

test("eval-client gets a 600 s JWT for the api resource", async () => {
  const p = await token("eval-client", "urn:codecite-eval:api");
  assert.equal(p.aud, "urn:codecite-eval:api");
  assert.equal(p.client_id, "eval-client");
  assert.equal(p.exp - p.iat, 600);
});

test("the other resource yields its own audience", async () => {
  const p = await token("eval-client", "urn:codecite-eval:other");
  assert.equal(p.aud, "urn:codecite-eval:other");
});

test("default resource is the api audience", async () => {
  const p = await token("eval-client");
  assert.equal(p.aud, "urn:codecite-eval:api");
});

test("eval-expiring gets a 2 s token", async () => {
  const p = await token("eval-expiring", "urn:codecite-eval:api");
  assert.equal(p.exp - p.iat, 2);
});
