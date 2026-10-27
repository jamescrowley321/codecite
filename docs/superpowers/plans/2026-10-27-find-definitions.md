# Find Definitions with `codecite def` and the `definition` Tool

**Goal:** Implement #33: `codecite def <corpus> <symbol> [--json] [--allow-stale]` and the MCP `definition` tool, answered from the tree-sitter symbols `codecite index` already writes to the `symbol` table. No indexing changes.

**Architecture:** As with `search` and `read`, the logic lives in `codecite-core` so the CLI and the MCP server share it byte for byte.

- `store/definition.rs` adds `Store::definition(&self, symbol, allow_stale) -> Result<Definitions>`. `Definitions { symbol, ambiguous, total, results: Vec<Definition> }` and `Definition` derive `Serialize` and `JsonSchema`.
- One SQL statement (`DEFINITIONS`) looks definitions up by `symbol.display_name = ?1`, joined to their chunk (lines and snippet), document and source. The exact lookup runs it with the name as asked. Only when that is empty and the name is dotted (`Client.callback`), it runs again with the last name, and the rows are kept whose SCIP descriptors inside the file (parsed from `symbol.id`, path segments left out) end with the dotted names. Both lookups are the same statement, so one `EXPLAIN QUERY PLAN` test shows the `symbol_name` index.
- Ranking sorts by role (`source`, `docs`, `example`, `test`), then citation ascending; the first `MAX_SEARCH_LIMIT` (50) are kept and `total` counts all. `ambiguous` is `results.len() > 1`.
- A new `Error::NoSuchSymbol { symbol, candidates }` (`no_such_symbol`) lists defined names with the same `codecite_name_key` (case, `_` and `-` ignored). An empty symbol is `EmptyQuery`; staleness goes through `check_fresh`.
- `codecite-mcp` adds `DefinitionParams { symbol, corpus?, allow_stale }` (`deny_unknown_fields`) and a `definition` tool whose output schema is `Output<Definitions>`.
- The binary adds `Command::Def`, printing JSON through `write_json` or a short human form with control characters escaped.

## Decisions

- `kind` is the symbol's kind (`method`, `struct`, …), not the chunk's (`function`, `class`), since it says what was defined.
- `dirty` is always present: symbols come only from code.
- The result echoes `symbol` as asked; the lookup trims it.
- `NoSuchSymbol` carries candidates, as the design's errors do. Finding them scans `symbol`, but only on the error path.

## Tasks

1. Core: `NoSuchSymbol` with its kind and candidates; `store/definition.rs` with unit tests for a unique, an ambiguous (deterministic order), a dotted and a missing symbol, the 50 cap, staleness, read-back of each citation, descriptor parsing and the index use.
2. MCP: the `definition` tool; the in-process client calls it for unique, ambiguous and missing symbols; tool-list, input-schema, output-schema and bad-argument tests include it.
3. CLI: `def` subcommand, parse and human-form unit tests; `crates/codecite/tests/def.rs` indexes a real checkout and covers unique, ambiguous and missing symbols; `tests/mcp.rs` checks `definition` against `def --json` byte for byte.
4. Docs: `docs/usage.md` "Find a definition" and `definition` under "Connect an agent"; `docs/design.md` `def` and `definition` rows, the ranking rule, and the Errors bullet; README's command table.

Gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
