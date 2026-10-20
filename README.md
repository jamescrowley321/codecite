# codecite

A local index of code and specs that answers AI agents with cited results.

codecite is a single Rust binary. It indexes source repositories and reference
specs into one SQLite file per corpus, and answers through a CLI and an MCP
server. Every result cites where it came from — `repo@sha:path#L10-L42` — so an
agent builds on real implementations and spec text instead of recalling them
from memory.

## Status

Pre-alpha. `codecite index <corpus>` indexes the local git checkouts a corpus
manifest (`corpora/<corpus>.toml`) lists into `~/.local/share/codecite/<corpus>.db`
on Linux, and downloads the documents it lists, checking each against its
pinned sha256 and caching it in `~/.cache/codecite/documents`, so later runs
work offline, then indexes their sections and numbered steps.
`codecite index <corpus> --print-hashes` prints each document's current hash,
for pinning. `codecite search <corpus> "<query>"` prints ranked, cited results
(`--json` for JSON; `--language`, `--path <glob>` and `--limit` filter them),
and refuses an index older than the manifest's `max_age_days` unless
`--allow-stale`. `codecite read <corpus> <citation>` prints the exact text a
citation points at under its pinned citation, for the citations search prints
and the spec forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`,
a spec URL). `corpora/identity.toml` is the first real corpus: identity-model and
the OAuth and OpenID specs it cites. The MCP server comes next.

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

## Build

`rust-toolchain.toml` pins the compiler; rustup installs it on the first
`cargo` call. Build with `cargo build --workspace` and run with
`cargo run -p codecite -- --version`.

CI runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked` and `cargo deny check` on every PR. Run them
locally before pushing; `--locked` fails if `Cargo.lock` is out of date, as it
does in CI. `cargo deny` needs `cargo install cargo-deny --locked`.

## License

codecite is licensed under the [Apache License 2.0](LICENSE). Attribution notices are in [NOTICE](NOTICE).
