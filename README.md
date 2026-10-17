# codecite

A local index of code and specs that answers AI agents with cited results.

codecite is a single Rust binary. It indexes source repositories and reference
specs into one SQLite file per corpus, and answers through a CLI and an MCP
server. Every result cites where it came from — `repo@sha:path#L10-L42` — so an
agent builds on real implementations and spec text instead of recalling them
from memory.

## Status

Pre-alpha. Nothing is usable yet; the design is being turned into milestones.

## Planned

- Code-aware full-text search (BM25 over split identifiers), with optional
  local embeddings.
- A code graph — definitions, references, callers, callees, impact — merged from
  tree-sitter, SCIP, LSP, CodeGraph and Graphify, with every edge tagged by the
  provider that produced it and how far to trust it.
- Pinned corpus manifests that record each source's commit and license class, so
  agents know what they may adapt and what they may only learn from.
- Fully local: no server process, no telemetry, no network access beyond
  fetching pinned sources.

## License

Apache-2.0 (to be added with the first code).
