The repository at `/work/project` is jamescrowley321/identity-model. Make the change this issue
asks for. Stop when you're done.

# spec vectors: serve canned HTTP vectors from the node-oidc fixture

Part of identity-model#801.

Serve spec HTTP vectors from the existing `infra/node-oidc-provider` fixture instead of a mock server in each language.

## Scope
- `infra/node-oidc-provider/vectors.js`: Koa middleware mounted with `provider.use()` ahead of the real OP.
  - `{ISSUER}/v/{run}/{capability}/{case_id}/{vector}/{path}` returns `vector.http[path]` (status, headers, `body_fixture` with `https://server.example.com` rewritten to the vector's base URL) and records the request.
  - `{ISSUER}/v/{run}/{capability}/{case_id}/{vector}/_check` returns `{"ok": bool, "diffs": [...]}`, comparing the recorded requests against `expect_request` (method, headers, form; `content-type` compared by media type; expected `""` = absent). Clears the record.
  - `{vector}` = vector name, or index when unnamed. `{run}` = caller-chosen token so concurrent runs don't share records.
  - Anything else falls through to the real OP; its behaviour is unchanged.
- `infra/docker-compose.yml`: mount `../spec:/spec:ro`, `SPEC_DIR=/spec`. `Dockerfile` copies `vectors.js`.

## Acceptance
- The existing node-oidc integration suites (Python, Go, Rust) pass unchanged.
- Against a running fixture: a revocation vector returns its canned status/body; `_check` reports `ok: true` for a matching request and names each mismatching header/form field otherwise; an empty `body_fixture` yields the vector's status (not 204).
- Fixture paths outside `spec/test-fixtures/` and unknown capabilities/cases/vectors are rejected.

No runner changes; the per-language stories consume this.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
