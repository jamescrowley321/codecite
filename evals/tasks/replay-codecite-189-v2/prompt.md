The repository at `/work/project` is jamescrowley321/codecite. Make the change this issue
asks for. Stop when you're done.

# feat(cli): make the CLI an agent's natural search tool

## What

Compact default output (one line per hit with its citation, `--json` for structure), `--help` for every command written for an agent learning the tool cold, and a short instructions snippet for AGENTS.md or CLAUDE.md in `docs/usage.md`. The MCP instructions lose their spec-specific advice. CLI query time is measured against `rg` on the history benchmark's repositories.

## Done when

The history benchmark doesn't lose on any repository, and output bytes per query and query time are reported next to `rg`'s.

## Scope of this task

Do only the `codecite` CLI's part, in `crates/codecite/src/main.rs`: the
compact human output of `search`, `grep` and `def`, and help for `search`.
Leave the docs snippet, the MCP instructions and the `rg` benchmark alone.
`--json` output doesn't change. The tests run in a debug build
(`cargo test --locked --offline`), which is already warm; a release build
or a workspace-wide `cargo clippy` starts cold and takes minutes.

## What the tests check

The human (non-`--json`) output becomes one line per hit, with no rank
number, no score and no indented snippet line below it. The header always
says how many hits there are, and carries the license class and index time
only when every hit shares them; otherwise each hit's line carries its own.

- `search`: a header `N result(s)` (or `no results`), with
  ` · <license> · indexed <at>` when every hit shares them, then one line per
  hit, best first:
  `<citation> · <kind>[ <symbol>][ · dirty][ · older version pinned][ · <license> · indexed <at>] · <first snippet line>`
- `grep`: a header `N match(es)` (or `no matches`), with
  ` · <license> · indexed <at>` when every match shares them, then one line
  per match, in index order, as `rg` prints `path:line:text`:
  `<citation>[ · dirty][ · <license> · indexed <at>]: <matched line>`
- `def`: a header `N definition(s) of <symbol>[ · ambiguous]`, with
  ` · <license> · indexed <at>` when every definition shares them, then one
  line each:
  `<citation> · <kind> <symbol> · <role>[ · dirty][ · <license> · indexed <at>] · <first snippet line>`
- `codecite search --help` has an `Examples:` section and an `Output:`
  section describing the output above.
