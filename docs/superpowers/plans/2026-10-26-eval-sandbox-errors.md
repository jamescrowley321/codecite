# Label sandbox failures apart from agent timeouts (#117)

`pi()` in `evals/inspect/task.py` records every exception from the agent's
`sb.exec` as `timed_out`, so the scorecard's timeouts column mixes agent
timeouts with infrastructure crashes. Both stay scored as failures; only the
label and the report change.

## Steps

1. **Tests first** (`evals/inspect/tests/`, run with `uv run --locked pytest`
   from `evals/`; pytest is in the project's `dev` group, and its config puts
   `inspect/` on the import path):
   - `test_task.py` drives `pi()` with a stub sandbox, store and agent bridge.
     An `exec` that raises `TimeoutError` records `agent_timeout`; one that
     raises anything else records `sandbox_error` with the exception's type
     and message. Neither raises out of the agent.
   - `test_report.py` hands `report.scorecard()` logs built in memory (two models, a mix of
     passes, plain failures, timeouts and sandbox errors) and checks the
     scorecard: separate `timeouts` and `sandbox errors` columns, a `fails`
     column they fit inside, each sandbox error listed under the table with
     its model, arm and epoch, and a flagged verdict for the model that had
     them.
2. **`task.py`:** catch `TimeoutError` apart from other exceptions and store
   `stopped` (`agent_timeout` or `sandbox_error`) and, for a sandbox error,
   `sandbox_error` (`Type: message`). pi's output overflowing the stream
   limit (`OutputLimitExceededError`) or failing to decode is the agent's
   doing, so it gets `agent_output` instead and counts as a plain failure.
   The scorer copies the reason and the error into the score's metadata and
   its explanation; a stopped agent still can't pass.
3. **`report.py`:** read `stopped` (falling back to the old `timed_out` flag
   as `agent_timeout`, since older logs can't tell the two apart). Add
   `fails`, `timeouts` and `sandbox errors` columns, list each sandbox error
   under the table, and when a model has any, append the per-arm counts to
   its verdict so it doesn't read as a clean loss or pass.
4. **Docs:** `evals/README.md` (how a sample runs, how to run the tests) and
   `docs/design.md` (Evaluation plan) describe the two labels.

## Default a reviewer can change

Any sandbox error in a model's runs flags that model's verdict, whether the
errors fall in one arm or both: even balanced errors shrink both arms, and a
handful is enough to move an interval at the panel's epoch counts.
