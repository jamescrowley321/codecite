# codecite cite check

Issue #222, milestone M5 Manifests and licensing.

## What exists

- `Store::cite` (`crates/codecite-core/src/store/cite.rs`) checks one
  citation through `read`: resolved with what supersedes it, unresolved with
  findings and candidates, or a typed error.
- `codecite cite <corpus> <citation>` prints one result. Nothing finds the
  citations already written in a tree.

## Design

- **Finding** lives in `codecite-core` (`citation/find.rs`):
  `find_citations(text)` returns each citation with its line and column
  (from 1, in characters). One regex: the canonical forms (a name that may
  hold `/`, `@`, a 7-64 digit commit or `sha256:` and 12 digits, an
  optional `:path` that runs to the `#` and stops only at whitespace,
  backticks, quotes, `<`, `>` and `|`, and a `#` fragment of any letter or
  digit, `_`, `-`, `.`, `:` and `/`), or a Markdown link whose text holds `§`
  and whose target is an http(s) URL with an anchor. Trailing dots and
  colons are dropped, and a leading byte-order mark is not counted in a
  column. Written forms in running text are out of scope.
- **Checking** reuses `Store::cite`: `Store::citation_checker(options)`
  checks freshness once and returns a `CitationChecker` that caches each
  distinct citation's outcome. `NotACitation` is skipped; the
  citation-shaped errors (`BadCitation`, `UnknownSource`, `UnknownVersion`,
  `NoSuchFile`, `NoSuchAnchor`, `LinesPastEnd`, ...) become a `refused`
  problem with the error's kind, message and candidates; an unresolved
  result becomes `unresolved`; a resolved one with `superseded_by` becomes
  `superseded` unless `allow_superseded`. Any other error stops the check.
- **CLI**: `cite` gets an optional `check` subcommand
  (`args_conflicts_with_subcommands`, `subcommand_negates_reqs`, no `help`
  subcommand), so `codecite cite <corpus> <citation>` is unchanged. Walking
  and output live in `crates/codecite/src/check.rs`, keeping `main.rs`
  edits small. Exit 1 when any citation fails and 2 when the check can't be
  made. Files found in a walk that are not text are skipped and counted; a
  named one is an error.

## Steps

1. Tests for `find_citations`: every canonical form, spec links, delimiters
   and sentence ends, lines and columns, near misses. Implement.
2. Tests for `CitationChecker` on the `cite` fixture: clean citations, each
   problem kind, `allow_superseded`, skipped non-citations and repeats,
   stale index. Implement.
3. End-to-end tests in `crates/codecite/tests/cite.rs`: a clean tree (with
   hidden, binary and linked files skipped), a failing tree in text and
   JSON, `--allow-superseded`, a missing path, a stale index, and `cite`
   still taking one citation. Implement the subcommand.
4. `docs/design.md` (Interfaces) and `docs/usage.md` (a "Check every
   citation in CI" section).
5. `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D
   warnings`, `cargo test --workspace --locked`.
