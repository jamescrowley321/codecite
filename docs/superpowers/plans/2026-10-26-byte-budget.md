# Byte Budget for Snippets and Reads Implementation Plan

**Goal:** Implement #105. Cap a search snippet and `read` text in bytes as well as lines, so one minified or generated line can't flood an agent's context. The CLI and the MCP tools both get the cut.

**Architecture:** Both caps live in `codecite-core`, next to the line caps. `MAX_SNIPPET_BYTES` (4 KiB) sits beside `MAX_SNIPPET_LINES` in `store/search.rs`, and `MAX_READ_BYTES` (64 KiB) beside `MAX_READ_LINES` in `store/read.rs`. One helper, `byte_prefix`, cuts a string to the longest prefix that fits and ends on a character boundary (`str::floor_char_boundary`). `snippet()` and the read path's `cap()` apply it after the line limit. The CLI's JSON and human forms and the MCP tools all go through these two functions, so they all get the same cut.

**Marking a cut:** The text gets no marker. `ReadResult.truncated` was already true when `max_lines` cut the text, and it is now also true when the byte budget does. `SearchResult` gains `snippet_truncated`, true when the snippet stops before its chunk ends, whether the line cap or the byte cap stopped it. The citation is unchanged, so reading it again returns the same capped text. A truncated `quote` read's `instruction` is `QUOTE_TRUNCATED_INSTRUCTION`, which says the quote is partial. Snippet and `read` text joins lines with `\n` as before, so the kept text is an exact prefix of the lines, not of CRLF source bytes; a CRLF test covers that.

**Budgets:** 4 KiB is about 40 lines of 100 characters, which is what the line cap already allows for ordinary code. With 50 results, a search answer stays near 200 KiB. 64 KiB is about 500 lines of 130 characters, which matches `MAX_READ_LINES`.

## Tasks

1. Unit tests, written first: a snippet and a read of one long line of two-byte characters are cut on a character boundary, to an exact prefix, and marked. A citation read twice returns the same result. A 40-line chunk is not marked.
2. Add the constants, `byte_prefix`, and `snippet_truncated`, and export the constants from `codecite-core`.
3. End-to-end tests:
   - `tests/read.rs` indexes a file whose only line is 1 MiB, and checks `search --limit 50 --json` and `read --max-lines 1 --json` against the budgets. It also checks a multi-byte line's cut.
   - `tests/mcp.rs` pulls its JSON-RPC session into a `Session` helper and checks that the `search` and `read` tools return the same bytes as the CLI for the 1 MiB line.
4. Docs: describe the budgets and how a cut is marked in the `limit`/`max_lines` paragraph of `docs/usage.md`, in `docs/design.md`, and in the MCP tool descriptions in `crates/codecite-mcp/src/server.rs`.
5. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked`.
