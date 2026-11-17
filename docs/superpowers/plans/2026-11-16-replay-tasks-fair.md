# Make the replay tasks separate the arms (#202)

The quick assist eval on #199 ran the four replay tasks. Three could not show
any difference between arms. This plan finds why from the eval logs, fixes
the miner, and replaces each broken task with a fair one.

## Why each task failed (eval logs of runs 41216757020, 41216758927, 41216760827, 41216762448)

- `replay-terraform-provider-descope-75`: every sample's `tests_tail` is
  `FAIL .../internal/infra [build failed]`. The hidden test is in package
  `infra` and calls `isRetryableError`, `retryWaitDuration` and
  `transientRetryBaseWait`, which the fix adds and the prompt never names.
  A task bug: it measures name-guessing. The scorer kept only stdout, so the
  compiler's `undefined:` lines never reached the log.
- `replay-identity-model-908`: the agents add the allowlist check, but the
  hidden tests assert an exact message (`Algorithm 'RS256' is not in the
  allowed algorithms`), exact `details` (`{"alg", "allowed"}`) and that an
  empty allowlist rejects. The issue states none of these. A task bug.
- `replay-codecite-189`: 28 of 30 samples hit the 600 s agent limit. The
  issue spans the CLI output, every command's help, docs, the MCP
  instructions and a benchmark, and the hidden tests pin exact output lines
  the issue never describes. The transcripts show the agents doing all of
  it: release builds of `codecite` and `codecite-bench` for the `rg`
  benchmark and workspace-wide clippy, both cold (the image warms only the
  debug test build), on top of the edits. The task is too broad, so a
  longer `AGENT_TIMEOUT` would buy time for work the hidden tests never
  check; cutting it to the CLI part removes both the scope and the reason
  for the cold builds.
- `replay-identity-stack-449` discriminates. Its base is `baseRefOid`, one
  unrelated PR behind the merge's parent, which touches none of its fix or
  test files. It stays, frozen.

## Miner and scorer fixes, test-first (`evals/tests/`)

1. Test data under a test directory (`tests`, `test`, `testdata`,
   `fixtures`, snapshots) goes to `hidden/` and task.json `data`, never to
   `fix.patch`; the scorer overlays it with the tests but does not name it
   to the runner.
2. The base is the merge commit's first parent.
3. A Rust `--test` target is only a file directly in `tests/`.
4. `write_task` never removes a task directory: a re-mined PR gets the next
   free `-v<n>` id; `main` skips PRs already mined unless `--pr` names them.
5. The prompt ends with "Names the tests use" when the hidden tests call
   names the fix adds and the base lacks: file, doc comment, definition.
6. `tests_tail` keeps stderr after stdout, so a failed build says why.

## Tasks

- `replay-terraform-provider-descope-75-v2`: re-mined; the prompt now names
  the three helpers.
- `replay-identity-model-908-v2`: re-mined; a hand-written "What the tests
  check" section gives the message, details and empty-list rule.
- `replay-codecite-189-v2`: re-mined, then cut to the CLI part: fix.patch is
  `crates/codecite/src/main.rs` alone, and the prompt scopes the task and
  states the output formats the tests check, from the PR's own help text,
  and says the debug test build is warm and a release build is not.
- The three broken originals stay as they are: they are frozen, and CI's
  `eval-task` matrix (`.github/`, outside this PR) still names two of
  them. Retiring them, with that matrix, is a follow-up.
- Run `check.sh` for every kept task.

## Calibration

Dispatch the quick tier, baseline arm, for each new task:
`gh workflow run evals.yml --ref fix/replay-tasks-fair -f task=<id> -f arms=baseline`.
A task still 0% or 100% for every model after its fair fix is frozen with
its reason in task.json; one in band is frozen before its first A/B run.

## README

`evals/README.md` describes the first-parent base, test data, Rust targets,
the interface section, hand-written additions, `-v<n>` ids and `--pr`, the
600 s limit for replay tasks and the stderr in `tests_tail`.
