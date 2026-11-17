# SCIP provider, references, callers and callees — plan

Issue #234. Design: `docs/superpowers/specs/2026-11-16-scip-graph-design.md`.
Each step is test-first: write the failing test, make it pass, run
`cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D
warnings` and `cargo test --workspace --locked` before pushing.

## 1. Schema (codecite-core)

- Test: a fresh store has `occurrence` and `symbol.provider`/`local_id`; a
  version 9 corpus upgrades to 10 keeping its symbols as `treesitter`; every
  foreign key's index test lists `occurrence_document`.
- Add `migrations/0014_scip.sql` and register it.
- `clear_document` deletes the document's occurrences with its symbols.

## 2. Writing a SCIP layer (codecite-core `store/scip.rs`)

- Types: `ScipDocument { path, definitions, occurrences }`,
  `ScipDefinition { symbol, display_name, kind, line, start_line, end_line }`,
  `ScipOccurrence { symbol, start_line, end_line, roles }`, `ScipSummary`.
- `Store::write_scip(source_id, &[ScipDocument])` and
  `Store::clear_scip(source_id)`, one transaction each.
- Tests: a definition replaces the innermost same-named tree-sitter symbol
  holding its line and the chunk follows; an unmatched one is inserted; a
  second write after a changed index gives the same rows as one write;
  `clear_scip` restores tree-sitter IDs; references give `calls` edges to
  methods and `references` edges otherwise, enclosing by innermost range;
  missing documents and lines past the end are counted, not written.

## 3. Decoding (codecite-index `scip.rs`)

- `prost` messages for `Index`, `Document`, `Occurrence`, `SymbolInformation`
  and the typed ranges; `decode(bytes) -> Vec<ScipDocument>`-shaped owned
  data; 256 MiB cap; `local` symbols skipped; kind from
  `SymbolInformation.kind`, else the descriptor's suffix.
- Tests: the committed rust-analyzer fixture
  (`tests/fixtures/scip/app/index.scip`) decodes to the expected definitions
  and references; typed ranges win over deprecated ones; a three-element
  range is one line; a truncated file is an error.

## 4. Manifest and pipeline (codecite-index)

- `scip` on `GitSource` and `LocalSource`; `Manifest::load` resolves it
  against the manifest's directory; `~` expands.
- `index_corpus` reads and decodes every SCIP file before writing, then
  after each tree source's documents calls `write_scip` or `clear_scip`;
  `IndexSummary` reports per-source SCIP counts.
- Tests: a local source with the fixture indexes and replaces symbols; a
  missing file stops the run with the store untouched; removing `scip`
  restores tree-sitter IDs; a reindex equals a fresh index.

## 5. Queries (codecite-core `store/graph.rs`, `definition.rs`)

- `Store::references(symbol, min_confidence, allow_stale)` and
  `Store::graph(symbol, Direction, depth, min_confidence, allow_stale)` with a
  recursive CTE; errors `NoGraph` and `AmbiguousSymbol`.
- `definition`: providers and confidence from `symbol.provider`/`local_id`;
  `names_in_file` reads SCIP symbol strings.
- Tests over a store written with `write_scip`: references with citations
  and enclosing; callers and callees at depth 1 and 2 with shortest
  distance and `via`; cycles terminate; ambiguity, no graph, unknown symbol,
  stale index.

## 6. CLI and MCP

- `codecite refs` and `codecite graph callers|callees`, `--json` printing the
  tool JSON, human forms one line per result; help with examples.
- MCP tools `references`, `callers`, `callees` with schema-checked inputs
  (`depth` 1–10, `min_confidence` 0–3).
- Tests: CLI integration over an indexed fixture corpus; MCP results equal
  the store's and fit their output schemas.

## 7. Docs

- `docs/design.md`: data model (migration 14, occurrence), provider model
  (SCIP reconciliation), interfaces (commands, tools, JSON), decisions
  (move the reconciliation question into the table).
- `docs/usage.md`: the `scip` manifest key, producing an index, `refs` and
  `graph`, the MCP tools.
