# Back-Channel Logout Eval Task Plan

**Goal:** Implement steps 1–5 and 7 of #128: an eval task where finding the right code is the expensive part (#92). Step 6, calibrating the task to the 40–80% baseline band and freezing it, needs #111 and #127 and lands with them.

**Architecture:** `evals/tasks/backchannel-logout/` in the shape of `fastapi-middleware`.
- `project/`: a small FastAPI relying-party app. `app/logout.py` hand-rolls logout-token decoding: it checks the signature against the provider's JWKS and the audience, nothing else. `app/main.py` serves `POST /backchannel-logout`, `app/sessions.py` keeps sessions in memory. Its dependencies include `py-identity-model` (path source, substituted per run as in `fastapi-middleware`).
- `prompt.md`: the behaviour only. No identity-model symbol, no spec section, no spec name.
- `hidden/`: the conftest starts a stub provider on loopback serving discovery and JWKS, generates an RSA test key, and mints logout tokens. Tests: three valid tokens (`sub`+`sid`, `sid` only, `sub` only) end the session; ten bad ones get 400 and leave it active.
- `checks.py`: `provider-from-env` (settings read from `OIDC_DISCOVERY_URL` and `OIDC_CLIENT_ID`, no URL literal in `app/` but event-type URIs), `deps-unchanged`, `scope` (outside `app/` only docs may change, and the agent may add its own test modules). None requires identity-model.
- `reference/app/`: calls `py_identity_model.aio.validate_logout_token`. `reference/handwritten/app/`: PyJWT plus the §2.6 rules.
- `planted.sh`: runs the score command and the checks on the references, the start, and planted bad and good solutions, and asserts the exact set of failing tests and checks for each.
- `check.sh`: static checks (task.json's files exist; the prompt names nothing in `py_identity_model.__all__` and no section; the corpus manifest pins identity-model at the task's commit and every document by `sha256`), then `planted.sh`.
- CI: an advisory `eval-task` job in `ci.yml` fetches identity-model at the pinned commit and runs `check.sh`.

**Defaults taken (the issue left them open):**
- `task.json`'s `corpus.documents` names `evals/retrieval/identity-bench.toml` instead of copying its document list; `idp` is `null`.
- Valid tokens carry `exp`: errata set 1 made it required, so a hand-written solution may demand it, while identity-model's claim check does not. No test drops `exp`.
- The prompt adds two lines beyond the issue's wording: where the identity-model source is, as `fastapi-middleware`'s does for both arms, and which interfaces the hidden tests use (`app.main:app`, `app/sessions.py`).
- The harness still runs only `fastapi-middleware`. Choosing a task, and indexing the task's documents into the sandbox corpus, belong with #127's `task` input.

## Steps

- [ ] Start project; confirm the hidden tests fail it and pass both references.
- [ ] `checks.py`, `planted.sh`, `check.sh`; run `check.sh` against a shallow fetch of identity-model, as CI does.
- [ ] `eval-task` job in `ci.yml`; zizmor clean.
- [ ] `evals/README.md` Tasks section and `docs/design.md` Tasks bullet.
- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
