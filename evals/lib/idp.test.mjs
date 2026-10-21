import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { freezeIdp, IDP_DIR, idpDirFor, startIdp } from './idp.mjs';

const ready = existsSync(join(IDP_DIR, 'node_modules')) ? false : 'the IdP fixture has no node_modules (npm ci in evals/fixtures/node-oidc)';

test('freezeIdp copies the fixture, node_modules included, and the round scores from the copy', { skip: ready }, async () => {
  const round = mkdtempSync(join(tmpdir(), 'evals-idp-'));
  try {
    assert.equal(idpDirFor(round), IDP_DIR, 'a round without a frozen IdP (an old one) uses the live fixture');
    const dir = freezeIdp(round);
    assert.equal(dir, join(round, 'idp'));
    assert.equal(idpDirFor(round), dir);
    assert.equal(readFileSync(join(dir, 'provider.js'), 'utf8'), readFileSync(join(IDP_DIR, 'provider.js'), 'utf8'));
    assert.ok(existsSync(join(dir, 'node_modules', 'oidc-provider', 'package.json')));
    const idp = await startIdp({ dir });
    try {
      const disco = await (await fetch(`${idp.url}/.well-known/openid-configuration`)).json();
      assert.ok(disco.token_endpoint);
    } finally {
      await idp.stop();
    }
  } finally {
    rmSync(round, { recursive: true, force: true });
  }
});

test('freezeIdp refuses a fixture without node_modules', () => {
  const round = mkdtempSync(join(tmpdir(), 'evals-idp-'));
  try {
    assert.throws(() => freezeIdp(round, { from: round }), /npm ci/);
  } finally {
    rmSync(round, { recursive: true, force: true });
  }
});
