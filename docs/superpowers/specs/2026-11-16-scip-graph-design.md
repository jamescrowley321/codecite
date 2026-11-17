# SCIP provider and graph tools — design

The M6 milestone adds precise symbols from SCIP indexes and the graph tools
`references`, `callers`, `callees`, `impact` and `path`. `docs/design.md`
already decides that symbol IDs are SCIP symbol strings, that tree-sitter
symbols get SCIP-shaped local IDs a SCIP index replaces, and that codecite
reads provider outputs and never links their code. This spec settles what
that leaves open, written while the owner was unavailable, so each decision
carries its reason and the uncertain ones are listed at the end for review.

## Decisions

### 1. A manifest points at a `.scip` file; codecite doesn't run indexers yet

A `git` or `local` source takes an optional `scip = "<path>"`: a SCIP index
someone produced with rust-analyzer, scip-python, scip-typescript, scip-go or
scip-dotnet. A relative path starts at the manifest's directory, as a
guidance `path` does, and `~` expands.

- **Why a file, not a command:** running an indexer runs project code
  (rust-analyzer runs build scripts and proc macros), which `design.md`'s
  security section allows only for trusted sources, with timeouts. Reading a
  file someone else produced keeps the first slice free of process
  management, and it is the format every indexer writes. Running the
  indexer for a trusted source is #236.
- **Why the manifest's directory:** a git source's checkout is cleaned to
  its commit (`clean -ffdx`) on every run, so an index inside it would be
  deleted; the manifest's directory is where guidance already resolves.
- An optional `scip_sha256` pins the file; a source without one is named
  in the run's summary, as unpinned guidance is.
- Every SCIP file is read and decoded before anything is written, as
  documents and guidance are, so a missing, oversized (over 256 MiB) or
  malformed file stops the run naming the source and the file.
- The index's `relative_path`s are taken from the source's root. A SCIP
  document whose path the source didn't index is counted and skipped, and
  so is an occurrence past the end of its file, so an index built from
  another commit degrades instead of writing wrong lines. The run's summary
  reports both counts.

### 2. Decoding: `prost` with hand-written messages

`prost` (Apache-2.0, tokio-rs) derives the decoder from Rust structs whose
fields carry `scip.proto`'s field numbers, so there is no `protoc`, no build
script and no generated code to vendor. Only the fields codecite reads are
declared; protobuf skips the rest. The typed ranges `scip.proto` added
(`single_line_range`, `multi_line_range` and their enclosing forms) win over
the deprecated `repeated int32` ones, as the proto says consumers should.
prost's recursion limit stays on. The `scip` crate was the alternative: it
pulls in `rust-protobuf` and generated code for all of SCIP, where codecite
reads a dozen fields.

### 3. Reconciling tree-sitter and SCIP symbols (the open question)

A SCIP definition occurrence replaces the tree-sitter symbol in the same
file with the same name whose lines hold the occurrence's line, the
innermost if several do. The symbol row takes the SCIP ID, keeps the
tree-sitter lines (its chunk's, doc comments included) and kind, records
`provider = 'scip'` and the tree-sitter ID in `local_id`, and the chunk's
`symbol_id` follows it. A SCIP definition no tree-sitter symbol matches
(a field, a module, a definition tree-sitter doesn't chunk) becomes a
symbol of its own, its lines the occurrence's enclosing range when the
indexer gives one, otherwise the occurrence's line. `local` SCIP symbols are
skipped: they are unique only within a document.

Every index run first undoes the previous run's SCIP layer for each code
source (SCIP-only symbols, occurrences and edges deleted; replaced symbols
given back their `local_id`) and then applies the current index, in one
transaction per source. So a reindex matches a fresh index, an unchanged
file still picks up a changed SCIP index, and a source whose `scip` key is
removed returns to tree-sitter IDs.

- **Why replace rather than alias:** `design.md` already promises the
  replacement, and keeping one ID per definition means `definition`,
  `search` and the graph tools never have to merge two IDs.
- `definition` reports `providers: ["scip", "treesitter"]` and
  `confidence: 3` for a replaced symbol; a dotted lookup (`Client.validate`)
  reads SCIP descriptors as well as codecite's own, so it keeps working
  after replacement. rust-analyzer writes a method as
  ``impl#[Client]validate().``; a type parameter's name counts as one of the
  names inside the file, so `Client.validate` finds it.
- A SCIP ID already stored (two sources indexing the same package and
  version) keeps its first definition; the later one keeps its tree-sitter
  ID and is counted as a conflict.

### 4. Schema: an `occurrence` table, `calls` and `references` edges

Migration 14 adds:

```sql
ALTER TABLE symbol ADD COLUMN provider TEXT NOT NULL DEFAULT 'treesitter';  -- treesitter | scip
ALTER TABLE symbol ADD COLUMN local_id TEXT;    -- the tree-sitter ID a SCIP symbol replaced
CREATE TABLE occurrence (
  symbol_id TEXT NOT NULL,                       -- may name a symbol defined outside the corpus
  document_id INTEGER NOT NULL REFERENCES document(id) ON DELETE CASCADE,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  roles INTEGER NOT NULL,                        -- SCIP SymbolRole bits
  enclosing TEXT,                                -- the innermost definition holding it
  provider TEXT NOT NULL, confidence INTEGER NOT NULL
);
CREATE INDEX occurrence_symbol ON occurrence(symbol_id);
CREATE INDEX occurrence_document ON occurrence(document_id);
```

- **Why a table for occurrences:** `references` must list every site, but
  `edge`'s key `(src, dst, kind, provider)` holds one row per pair of
  symbols. Changing that key would make every edge a site; keeping sites
  apart keeps `edge` the symbol graph the recursive queries walk.
- **Edges from SCIP:** each reference inside a definition gives an edge from
  the innermost enclosing definition to the referenced symbol: `calls` when
  the referenced symbol is a method or function (its last descriptor is a
  method, `name().`), `references` otherwise. Its evidence is the first
  such site. Enclosing ranges come from the symbols' lines after
  reconciliation, not from a reference occurrence's `enclosing_range`,
  which rust-analyzer fills with the target's definition range.
- **Confidence:** SCIP rows are exact (3). A function passed as a value,
  not called, is still a `calls` edge; see the questions below.

### 5. Graph tools: the first PR ships `references`, `callers`, `callees`

- `references` lists every line holding a non-definition occurrence of one
  symbol (two on one line are one result), each cited with that line's
  text, the enclosing definition, `access` from the SCIP roles (`import`,
  `write`, `read`, `generated`, `test`), role, license class, `indexed_at`
  and `dirty`; at most 50, `total` counting all.
- `callers` and `callees` walk `calls` edges with a recursive CTE up to
  `depth` (default 1, at most 10), each symbol at its shortest distance,
  with its definition's citation when the corpus defines it, the neighbour
  one step nearer (`via`), the call site's citation, `providers` and
  `confidence`; at most 50, `total` counting all.
- **Symbol lookup:** a full symbol ID matches exactly (an external symbol
  that only occurrences name included); otherwise a name resolves as
  `definition` resolves it, over every symbol. Several matches are an
  `ambiguous_symbol` error whose candidates are the IDs, rather than a
  merged answer the agent can't take apart. A symbol no SCIP index covers
  is `no_graph`, never an empty success, since an empty list would read as
  "nothing calls this".
- CLI: `codecite refs <corpus> <symbol>` and
  `codecite graph callers|callees <corpus> <symbol> [--depth n]`, both with
  `--min-confidence`, `--json` and `--allow-stale`. MCP: `references`,
  `callers` and `callees`, the same JSON.
- `impact` and `path` follow in their own PR: they reuse the same walk, and
  holding them back keeps this PR reviewable.

## Follow-up issues

This PR is #234. The rest of M6:

- #235: `impact` and `path` graph tools.
- #236: run a SCIP indexer for a trusted source at its pinned commit, with a
  timeout.
- #237: tree-sitter's approximate `calls` and `imports` edges (inferred), so
  the graph tools answer without SCIP at a lower confidence.

## Unsure — for the owner to check

1. **`scip` as a path only.** Is reading a prebuilt index enough for M6, or
   should codecite run the indexer in this milestone? A prebuilt index for a
   git source must match the pinned commit, and codecite can only catch a
   mismatch coarsely (missing files, lines past the end).
2. **Function references count as calls.** Passing `check` as a value gives
   a `calls` edge at confidence 3. The alternative is confidence 2 for every
   SCIP-derived `calls` edge, which `impact`'s default would still include.
3. **Ambiguity is an error for graph tools** while `definition` returns all
   matches. The candidates are long SCIP IDs an agent must copy back.
4. **No `limit` on `references`.** It returns at most 50 with `total`; a
   `path` filter or a limit may be needed on large corpora.
5. **SCIP IDs across sources.** Two sources that index the same package and
   version produce the same IDs; the first source keeps them.
6. **SCIP-only symbols aren't `definition` results.** `definition` still
   lists only symbols with a chunk, so a field SCIP found is reachable
   through `references` but not `def`.
