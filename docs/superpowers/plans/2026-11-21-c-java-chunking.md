# C and Java chunking implementation plan

> For agentic workers: use subagent-driven development or inline plan execution; run each feature's tests red before adding its grammar.

**Goal:** Return named source spans for C and Java and measure their cost on separate large public corpora.

**Architecture:** Use the existing `Language` detector and Tree-sitter query pipeline, with one query per language and the existing bounded window fallback. Neither compiler invocation nor call resolution is in scope.

**Tech stack:** Rust, Tree-sitter C and Java grammars, SQLite FTS5.

**Requirements:** #271 and `docs/design.md` (Indexing and retrieval).

## Global constraints

- Treat `.h` as C by default; do not add C++ support.
- Preserve exact source spans, existing symbol conventions, and fallback behavior for uncovered lines and other languages.
- Bump `CHUNKER_VERSION` exactly once in the combined change. Keep frozen eval inputs unchanged.

## Tasks

- [ ] C: add a red `sample.c` chunk/span fixture and end-to-end definition test in `crates/codecite/tests/`. Add extension/name detection in `crates/codecite-core/src/language.rs`, dependency entries in the workspace and index manifests, and C captures under `crates/codecite-index/src/chunk/queries/`. Cover ordinary functions, pointer-return definitions, named types, headers, and prototypes not becoming function definitions.
- [ ] Java: add a red `sample.java` chunk/span fixture and end-to-end definition test. In the same integration points add Java recognition and a Java query. Cover nested classes, interfaces, methods and constructors, annotations or Javadoc, and duplicate method names without claiming build-aware resolution.
- [ ] Integrate the two branches, reconcile dependencies and the single chunker-version bump, update `docs/design.md` and `docs/usage.md`, and verify workspace fmt, locked clippy, and locked tests.
- [ ] With separate SQLite files and no overwrite of existing corpora, compare clean release runs on Linux v6.18 and the existing Keycloak pin. Report first-build time, DB bytes and chunk count, representative definition/search results and any limitations; keep performance anecdotes separate from a labelled benchmark.
