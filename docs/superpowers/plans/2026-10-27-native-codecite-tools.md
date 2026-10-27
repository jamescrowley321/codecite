# Native codecite tools in the eval, and arms that never used them

Issue: #91. The codecite arm reaches codecite only through `bash`, and some
models never call it, so their pair measures nothing.

## Steps

1. **Report, test-first.** In `evals/tests/test_report.py`, build logs whose
   samples carry model-call events and test:
   - `codecite_calls` counts a native call (`mcp__codecite__search`,
     `mcp__codecite__read`), a `bash` call that runs `codecite` (bare, by
     path, after `&&`, behind `VAR=value`, inside `$( )`), and not prose that
     names codecite, `grep codecite` or `ls /opt/codecite`;
   - the scorecard's `used codecite` column, per arm, as runs with a call
     out of usable runs;
   - a pair at exactly half adoption keeps its verdict; one just below gets
     `not measured`; errored samples don't count as usable;
   - calls survive a write and read of a log file.

   Then in `evals/src/codecite_evals/report.py`: `codecite_calls(sample)`
   reads the tool calls from each `ModelEvent`'s output, the row tuple gains
   "used codecite", and the verdict after the completeness check is
   `not measured (codecite used in k of n runs)` when k < n / 2.

2. **Task, test-first.** In `evals/tests/test_task.py`, test that only the
   codecite arm writes pi's `mcp.json` (`codecite mcp`, exposure `direct`)
   and lists the two codecite tools in `--tools`; that the baseline arm
   keeps `--no-mcp` and deletes codecite; that the commands differ only in
   those flags; and that the prompts differ only by the note.

   Then in `evals/src/codecite_evals/task.py`: `pi_command(arm, prompt)`;
   the codecite arm adds `-e builtin:mcp` (since `--no-extensions` drops
   built-in extensions) and `mcp__codecite__search,mcp__codecite__read` to
   `--tools` (an MCP tool no `--tools` entry names is never declared); the
   agent writes `/root/.pi/agent/mcp.json` before pi starts. The note names
   the tools and the CLI.

3. **Check pi for real, without a model key.** Run pi 1.2.0 with both arms'
   command lines against a fake OpenAI-compatible server on
   `localhost:13131` that logs each request's tool list and answers the
   first call with `mcp__codecite__search`. The codecite arm's requests list
   both tools with codecite's descriptions and get search results back; the
   baseline's list neither.

4. **Docs.** `docs/design.md` (Evaluation plan: Arms, Isolation, Measures,
   Success criteria) and `evals/README.md` (How a sample runs).

5. **PR** with the `eval` label, so the CI eval runs the quick tier with the
   native tools and posts a scorecard with `used codecite` for every model.
