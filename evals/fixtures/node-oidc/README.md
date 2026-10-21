# node-oidc fixture

A lean [node-oidc-provider](https://github.com/panva/node-oidc-provider) 9.7.1
that acts as the OIDC IdP for the eval's hidden pytest suite. It serves RS256
JWT access tokens over the `client_credentials` grant, bound to 127.0.0.1.

- Issuer: `http://127.0.0.1:9011` (env `PORT`, default `9011`)
- Discovery: `/.well-known/openid-configuration`; JWKS: `/jwks`
- Clients (`client_secret_post`, secret `eval-secret`):
  - `eval-client`: access token TTL 600 s
  - `eval-expiring`: access token TTL 2 s
- Resources: `urn:codecite-eval:api` (the audience the app must enforce, and the
  default) and `urn:codecite-eval:other` (for the wrong-audience test)
- Scope: `api:read`, granted on either resource when requested, so tokens can
  carry a `scope` claim. The hidden tests request it, so a solution that turns
  on identity-model's `require_access_token_marker` accepts valid tokens.

## Start

From Node:

    cd evals/fixtures/node-oidc && npm ci && node provider.js

From Docker:

    docker compose -f evals/fixtures/node-oidc/compose.yaml up -d --wait
    docker compose -f evals/fixtures/node-oidc/compose.yaml down

Ready when `GET /.well-known/openid-configuration` returns 200.

## Test environment

The hidden pytest suite reads:

- `TEST_DISCO_ADDRESS`: the discovery URL
- `TEST_JWKS_ADDRESS`: the JWKS URL
- `TEST_CLIENT_ID`: `eval-client` (or `eval-expiring` for expiry tests)

## Smoke test

    node --test evals/fixtures/node-oidc/smoke.test.mjs
