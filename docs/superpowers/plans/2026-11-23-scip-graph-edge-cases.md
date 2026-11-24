# SCIP graph edge cases (#269)

**Goal:** fix the SCIP edge and lookup rules that #242's last review and the
CodeGraph reuse benchmark (#295, PR #296) found, each with a test that fails
before its fix, without lowering the `codecite+scip` precision or recall in
`experiments/graph-reuse/results/summary.md` on #296's branch.

**Architecture:** the edge rules live in `Store::write_scip`
(`crates/codecite-core/src/store/scip.rs`), the lookup and walk in
`crates/codecite-core/src/store/graph.rs`, dotted names in
`crates/codecite-core/src/store/definition.rs`, and decoding in
`crates/codecite-index/src/scip.rs`. Today an edge is `calls` whenever the
target's last descriptor is a method (`name().`), whatever the site does.
The new rule asks the site: an occurrence is a call when the source text right
after it opens an argument list. That one rule fixes arrow-function calls
(a call to a term), imports and handler assignments (a method named, not
called). SCIP gives columns, so the decoder keeps the occurrence's end column
and the document's position encoding; core, which has the file text, decides.

**Tech stack:** Rust, `rusqlite`, `prost`. No schema change: the column is used
while writing and not stored.

Run the gates before every commit: `cargo fmt --check`, `cargo clippy
--workspace --all-targets --locked -- -D warnings`, `cargo test --workspace
--locked` (the `codecite-index` lib tests take minutes; filter by name while
iterating), `cargo deny check`.

## Task 1: Decode each occurrence's end column and the position encoding

Files: `crates/codecite-index/src/scip.rs`, `crates/codecite-core/src/store/scip.rs`
(the types), every `ScipOccurrence { … }` literal (core `scip.rs`, `graph.rs`,
`crates/codecite-mcp/src/server.rs`, index `scip.rs`).

1. Test first, in `codecite-index`'s `scip.rs` tests: encode an `Index` by hand
   (the prost structs) with a `SingleLineRange`/`MultiLineRange`/packed range
   occurrence and `position_encoding` set on the document; assert
   `decode_scip` returns each occurrence's `end_column` and the document's
   encoding. Run it; it fails to compile.
2. Add to `ScipOccurrence` `pub end_column: Option<u32>`: the range's end
   character on its last line, half-open, in the document's unit; `None` when
   the range doesn't give one. Add to `ScipDocument` `pub encoding:
   ColumnUnit`, a new `#[non_exhaustive]` enum in core: `Utf8`, `Utf16`,
   `Utf32`, `Unspecified` (SCIP `PositionEncoding` 1, 2, 3, 0). Declare
   `Document.position_encoding` (tag 6) and `SingleLineRange.end_character`
   (tag 3) in the hand-written messages.
3. Fill every other literal with `end_column: None` and the default encoding.

Learn: `#[non_exhaustive]` on a public enum makes downstream `match`es keep a
`_` arm, so adding a unit later isn't a breaking change.

## Task 2: Decide whether an occurrence is a call site

File: `crates/codecite-core/src/store/scip.rs` (a private `fn`, unit-tested).

1. Tests first: `call_site(line, end_column, unit, name) -> Option<bool>` on
   - Rust `check(token)`, `check (token)`, `Self::new()`, `x.check::<T>(a)` → `Some(true)`;
   - `use crate::client::check;`, `.map(Self::check)` → `Some(false)`;
   - Python `parser.UnparsedEntityDeclHandler = self.unparsed_entity_decl` → `Some(false)`;
     `super().__init__(x)` → `Some(true)`;
   - TypeScript `f(a)`, `f<T>(a)`, `f?.(a)` → `Some(true)`; `import { f } from "./a";`,
     `onClick={f}` → `Some(false)`;
   - a non-ASCII prefix (`"é"; f(x)`) under `Utf8`, `Utf16` and `Utf32` columns → `Some(true)`;
   - a column past the line, or one where the text before it doesn't end with
     `name` under any unit → `None`.
2. Implement: convert the column to a byte offset in the unit given (try
   UTF-16, then UTF-8, then UTF-32 when `Unspecified`), and accept the offset
   only when the text before it ends with `name` (the symbol's last
   descriptor name, `symbol_name`). After it, skip whitespace, then an
   optional `::<…>` or `<…>` (balanced angle brackets, on the line) and an
   optional `?.`; a call is a following `(`.

Learn: `str::char_indices` and `char::len_utf16` walk a line once in any
column unit; `strip_prefix` returns `Option<&str>`, so each optional piece
of syntax is one `if let`.

## Task 3: Give each edge the kind its sites show

File: `crates/codecite-core/src/store/scip.rs` (`write_occurrences`, `is_callable`).

1. Tests first, from minimal `ScipDocument` fixtures (hand-built, like
   `fixture::index`, with their own file texts):
   - **arrow function** (scip-typescript): `const f = () => 1;` defines term
     `f.`; `function g() { return f(); }` → edge `g → f.` is `calls`, so
     `graph callers f.` lists `g`;
   - **import**: an occurrence with role `0x2` (`import { f } …`, and Rust's
     `use client::check;`) gives no `calls` edge: role `0x2` → `imports`,
     a `use` line without the role → `references`;
   - **handler assignment**: `parser.H = self.decl` inside a method gives
     `references`, not `calls`, to `decl().`;
   - a call and a plain reference to one target from one definition give one
     `calls` and one `references` row, each with its first line as evidence.
   Run them; they fail.
2. Implement: per occurrence, its kind is `imports` when role `0x2` is set;
   else `calls` when the target is callable (`name().`) or a term (`name.`)
   and `call_site` says `Some(true)`, or says `None` and the target is
   callable (the old rule, so an unreadable column loses no call); else
   `references`. Key the first-site map by `(from, to, kind)`. Update the
   doc comments of `write_scip` and `ScipSummary::edges`.
3. Fix existing expectations in `scip.rs` and `graph.rs` tests that change
   for a reason the new rule gives, and say why in each.

## Task 4: A SCIP-only field or variable never encloses a reference

File: `crates/codecite-core/src/store/scip.rs`.

1. Test first (scip-python shape): `class RoutePattern:` with `def __init__`
   whose line `self._regex, self._converters = _route_to_regex(route)`
   defines `RoutePattern#_regex.` (SCIP-only, one line, inside `__init__`).
   Assert the edge to `_route_to_regex().` is from `RoutePattern#__init__().`
   and the occurrence's `enclosing` is `__init__`, not `_regex`.
2. Implement: factor the innermost-definition choice into one `fn` (Task 6
   reuses it). A symbol added by SCIP alone (`provider = 'scip' AND local_id
   IS NULL`) whose kind is `field`, `property`, `variable`, `constant`,
   `term`, `enum_member`, `parameter` or `type_parameter`, and which lies
   inside another definition of the file, is never a reference's enclosing
   definition. Tree-sitter-held symbols keep enclosing whatever their kind,
   so a `const f = () => { … }` tree-sitter found keeps its callees.

## Task 5: Parameters don't make a bare name ambiguous

File: `crates/codecite-core/src/store/graph.rs` (`symbols_named`), and
`SIMILAR_GRAPH_NAMES` in `definition.rs`.

1. Test first: a store with function `pkg/f().` and a SCIP-only parameter
   `pkg/f().(x)` and `pkg/g().(f)` (scip-python gives a parameter a symbol and
   a name): `graph callers f` resolves to `pkg/f().`, not
   `ambiguous_symbol`; `similar` names don't list parameters.
2. Implement: leave out symbols whose ID's last descriptor is a parameter
   (`…)`) or type parameter (`…]`) from name lookup. An ID still resolves as
   written.

## Task 6: `references` and `callers` on a tree-sitter symbol are `no_graph`

File: `crates/codecite-core/src/store/graph.rs` (`graph_symbol`, `in_graph`).

1. Change the test `a_tree_sitter_symbol_in_the_scip_graph_can_be_walked_from`:
   `callees` still walks; `references` and `callers` return `Error::NoGraph`
   (occurrences and edge targets only ever name SCIP IDs). Run; it fails.
2. Implement: `graph_symbol` takes what the tool walks (`references`,
   `callers`, `callees`). A tree-sitter symbol counts as walkable only for
   `callees` and only when a SCIP edge starts at it, both when named by ID
   and when counting name matches.

Learn: a small `enum Walk { References, Callers, Callees }` makes the call
sites say which rule they want, instead of a `bool` that reads as `true`.

## Task 7: Dotted names find trait-impl methods

File: `crates/codecite-core/src/store/definition.rs` (`descriptor_names`).

1. Test first: `names_in_file("rust-analyzer cargo app 0.1.0 impl#[Foo][IntoIterator]into_iter().")`
   is `["impl", "Foo", "into_iter"]`; `def Foo.into_iter` and
   `graph callees Foo.into_iter` find it in a store holding that symbol.
2. Implement: after `impl#[Type]`, skip the next type-parameter descriptor
   (the trait) in the in-file names.

## Task 8: Taking an ID over recomputes the loser's rows

File: `crates/codecite-core/src/store/scip.rs` (`write_definitions`).

1. Test first: index two sources [A, B] and [B, A] where B's definition sits
   on a tree-sitter symbol and A's is SCIP-only (`other_source` fixture).
   Assert the loser's occurrences have the same `enclosing` in both orders
   (the next definition out, not NULL) and the same set of
   `(src, dst, kind)` edges.
2. Implement: instead of `SET enclosing = NULL`, recompute each loser
   occurrence's enclosing with Task 4's function over its document's
   symbols, and move each deleted edge from the lost ID to the new
   enclosing definition of its evidence line (`INSERT OR IGNORE`, keeping
   the earlier line when both are in one document).

## Task 9: Mark every node defined in another source

File: `crates/codecite-core/src/store/graph.rs` (`graph_node`).

1. Test first: a two-hop `callees` walk A (source 1) → B (source 2) → C
   (source 2): today only B carries `defined_in`; C must too. And a
   `callers` walk across sources marks the callers in the other source.
2. Implement: compare each node's source with the asked symbol's source
   (its definition's, else the first call site's), not with the call site's.
   Update the `defined_in` doc and design.md.

## Task 10: Cap SCIP text that reaches agents

Files: `crates/codecite-index/src/scip.rs`, `graph.rs` (`descriptor_name`),
`crates/codecite-mcp/src/server.rs`.

1. Tests first: a 600-byte symbol is skipped; a descriptor-derived name of
   an out-of-corpus node is cut to 256 bytes at a char boundary; the
   `references`, `callers` and `callees` tool descriptions say IDs and names
   come from the SCIP index and are data, not instructions.
2. Implement: `MAX_SYMBOL_BYTES = 512`; cap `descriptor_name` with the same
   rule as display names (share the constant through core); add the sentence
   to the three descriptions.

## Task 11: MCP and CLI agree on the graph tools

File: `crates/codecite/tests/mcp.rs`.

1. Add `call_matches_cli` cases for `references` (`refs --json`), `callers`
   (`graph callers --json`) and `callees` (`graph callees --depth 2 --json`)
   against a corpus indexed with the fixture crate's SCIP index
   (`crates/codecite-index/tests/fixtures/scip/app`, as `tests/graph.rs`
   builds it). This item is a test only, so nothing fails first.

## Task 12: Static and instance methods with one ID

scip-typescript gives a class's static and instance methods of one name the
same symbol, so the index itself can't tell their callers apart.
`docs/design.md` promises one ID per definition but doesn't say what codecite
does when the indexer breaks that. Record the options on #269 at the PR step
(leave the second definition on its tree-sitter ID as today and say so in
the run summary; or keep both off the graph as `no_graph`), and list it in
the PR body as not closed by this PR. The PR says `Part of #269` if this
item stays open.

## Task 13: Docs

`docs/design.md`: the SCIP paragraph (edge kinds by call site, imports,
enclosing rule, takeover recompute, 512-byte cap) and the graph tools
paragraph (`no_graph` for tree-sitter `references`/`callers`, parameters left
out of name lookup, `defined_in`). `docs/usage.md` where it describes
`refs`, `graph` or edge kinds.

## Task 14: Benchmarks

1. History benchmark: `cargo build --release --locked -p codecite -p
   codecite-bench`, `evals/history/run.sh`, commit `evals/history/results/`,
   compare each repository's pooled `all` codecite row with
   `origin/fix/def-line-order`'s `scorecard.md`.
2. Graph reuse: in a scratch checkout of #296's branch, rebuild the
   `codecite+scip` corpora (SCIP indexes at the pinned indexer versions in
   `corpora.toml`, checked against their `scip_sha256`) with this branch's
   release binary, run `run.py measure` and `report`, and compare the
   `codecite+scip/name` and `codecite+scip/id` rows (Edge P, Edge R, per
   operation) with #296's `summary.md`. Put both tables in the PR body. If a
   pinned SCIP file can't be reproduced, say so in the PR rather than claim
   the comparison.
