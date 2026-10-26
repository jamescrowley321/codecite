# Doc Comments in Definition Chunks Implementation Plan

**Goal:** Implement #98: a definition's chunk takes in the doc comments and attributes directly above it, so its citation and its terms include the prose that describes it, instead of leaving that prose in a separate `window` chunk.

**Architecture:** One change in `codecite-index/src/chunk/code.rs`. After `definitions` sets each captured definition's line range, `documented_start` walks back from the definition's node over its previous siblings while each one attaches (`attaches`, per language) and no blank line separates it from what follows. Before walking, it climbs out of wrappers that hold only this definition (`export_statement`, a single-declarator `lexical_declaration` or `variable_declaration`, a single-spec Go `type_declaration`, `const_declaration` or `var_declaration`), since the comment sits above the wrapper. Only the chunk's start line moves; nesting still follows node byte ranges, so parents and descriptors are unchanged. The tree-sitter queries are untouched.

**Spec:** Issue #98; `docs/design.md#indexing-and-retrieval` step 3.

## What attaches

- Rust: `///` and `/** */` doc comments and `#[...]` attributes. Not `//!` or `/*! */`: an inner doc documents the enclosing module, not the next item, so attaching a file's module docs to its first item would mis-cite them. Not plain `//`.
- Go: any comment, since Go doc comments are plain `//` lines directly above.
- TypeScript, TSX, JavaScript: `/** */` JSDoc blocks and decorators. Not plain `//`.
- C#: `///` XML doc comments and `/** */`. Attributes are already children of the declaration.
- Python: nothing new; decorators were already part of the chunk and the docstring sits inside the body.

A comment that starts on a line ending in code (`A = 1 // one`) belongs to that code and stops the walk, so it never pulls a definition's chunk onto another definition's line.

## Steps

- [ ] Add `doc_comments_and_attributes_join_their_definition` to `chunk.rs` with a case per language, covering stacked comments and attributes, a blank line and a plain comment stopping the run, Rust `//!` staying out, methods inside a class or `impl`, `export`, Go `const`/`type`/`var` wrappers, and a Go trailing comment. Run it and watch it fail.
- [ ] Implement `documented_start` and `attaches` in `code.rs`; pass the language into `definitions`. Share the column-0 end handling of `line_range` as `last_row`, since a Rust line comment ends at column 0 of the next row.
- [ ] Update the fixture expectations that now include their doc comment (`sample.rs`, `sample.go`) or attribute (`#[cfg(test)]` in `duplicate_names_keep_ids_unique`).
- [ ] Add the rule to `docs/design.md`, step 3.
- [ ] Run fmt, clippy and the workspace tests; run `codecite-bench` before and after on a fresh corpus and put both tables in the PR.
