# client-auth-uri Eval Task Implementation Plan

**Goal:** Implement steps 1–4 and 6 of #129: a second A/B eval task whose right answer sits in a spec section the agent has to find and cite. Step 5 (calibrate on the baseline arm, then freeze) needs #111 and #127 and is not part of this plan.

**Architecture:** `evals/tasks/client-auth-uri/` in the shape of `fastapi-middleware`: `task.json`, `prompt.md`, a starting `project/`, `hidden/` tests, `checks.py`, `reference/`, `check.sh` and `planted.sh`. The project's token endpoint is its own, so the hidden tests need no IdP and `planted.sh` can run them for every variant; `check.sh` runs `planted.sh`.

**Spec:** Issue #129; RFC 6749 §2.3.1 (client password authentication) and §5.2 (error response); `docs/design.md`, Agent benchmark.

## Review Focus

- **Overcorrection is the trap.** §2.3.1 says HTTP Basic MUST be supported and the form body MAY be used. The hidden tests must keep both, and the planted solutions that drop either must fail exactly one test.
- **Both ways of fixing pass.** Rejecting a request with URI credentials outright, and ignoring them (so authentication fails), both end in `invalid_client`. `planted.sh` proves both pass.
- **The citation check is exact about the section.** §2.3, §2.3.10, §2.3.1.2 and RFC 6750 §2.3.1 fail; codecite's citation (with or without its pin), the spec URL, `RFC 6749 §2.3.1`, `RFC6749, Section 2.3.1`, `Section 2.3.1 of RFC 6749`, a citation wrapped across comment lines and one in a docstring pass. A citation in a string literal that isn't a docstring fails.

---

### Task 1: The starting project and the prompt

- [ ] `project/app/main.py`: `POST /token` for `client_credentials`; credentials from HTTP Basic, else the form body, else the query string (per parameter, so a split body/query pair also authenticates). Failures are `invalid_client` with 401 and `WWW-Authenticate: Basic`.
- [ ] `project/pyproject.toml`: `fastapi`, `uvicorn`, `python-multipart`; no identity-model dependency.
- [ ] `prompt.md`: the issue's wording, with no RFC or section named.

### Task 2: Hidden tests, test-first against the start

- [ ] `hidden/test_hidden.py`: Basic and form body get a token; query-string credentials, and a query-string secret with a body client id, get `invalid_client` (400 or 401, as §5.2 allows); wrong secrets over Basic (401 with `WWW-Authenticate`) and the body, and an unknown client, get `invalid_client`.
- [ ] Run against the start: exactly the two query tests fail.

### Task 3: The reference and `checks.py`

- [ ] `reference/app/main.py`: reject URI credentials with a comment citing RFC 6749 §2.3.1, and drop the query fallback.
- [ ] `checks.py`: `cites-section` (comments, joined across consecutive lines, and docstrings in `app/`), `deps-unchanged` and `scope`, the last two as in `fastapi-middleware`.

### Task 4: `planted.sh` and `check.sh`

- [ ] Each variant states the hidden tests and the checks it must fail: the reference fails none; the unchanged start fails the two query tests and `cites-section`; dropping the form body or Basic fails one test each; a wrong citation fails only `cites-section`; a new dependency or a file outside `app/` fails its check; the valid citation forms and a fix that ignores URI credentials fail nothing.
- [ ] Run: `evals/tasks/client-auth-uri/check.sh` exits 0.

### Task 5: Docs

- [ ] `evals/README.md`: a Tasks section describing both tasks.
- [ ] `docs/design.md`: the Tasks bullet of the Agent benchmark names the task.

### Task 6: Register the task in the `codecite_evals` package

#131 moved the harness into `evals/src/codecite_evals/`. The task's own files (`checks.py`, `hidden/`, `project/`) stay in `evals/tasks/client-auth-uri/`, as `fastapi-middleware`'s do: they are data the sandbox runs, not harness code.

- [ ] `tests/test_task.py`, first: both tasks are registered; each arm's prompt comes from its own task, and only the codecite arm's note names the specifications, only for a corpus with `corpus.documents`; the scorer copies in its own task's tests, checks and start.
- [ ] `task.py`: `ab_task(task_id)` builds the Inspect task from `config.task_dir`; `fastapi_middleware` and `client_auth_uri` call it; `_registry.py` exports both.
- [ ] `inspect/prepare.sh <task-id>` stages that task's project and a `documents.toml` cut from its `corpus.documents` manifest; the Dockerfile appends it to the `eval` corpus manifest.
- [ ] Run: `uv run --locked pytest`; Inspect loads `codecite_evals/client_auth_uri`; the Dockerfile's corpus step, run on the host over the staged context, indexes identity-model and the 39 documents.
