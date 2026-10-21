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
on Linux, and the git repositories it lists at their pinned commits, from a
partial clone kept in `~/.cache/codecite/repos`, and downloads the documents it lists, checking each against its
pinned sha256 and caching it in `~/.cache/codecite/documents`, so later runs
work offline, then indexes their sections and numbered steps.
`codecite index <corpus> --print-hashes` prints each document's current hash,
for pinning. `codecite search <corpus> "<query>"` prints ranked, cited results
(`--json` for JSON; `--language`, `--path <glob>` and `--limit` filter them),
and refuses an index older than the manifest's `max_age_days` unless
`--allow-stale`. `codecite read <corpus> <citation>` prints the exact text a
citation points at under its pinned citation, for the citations search prints
and the spec forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`,
a spec URL). `codecite corpora` lists the indexed corpora as JSON, with each
one's age, size, source count and staleness. `codecite mcp` runs the MCP server
over stdio; its one tool so far, `corpora`, returns the same JSON.
`corpora/identity.toml` is the first real corpus: identity-model and the OAuth
and OpenID specs it cites.

## Install

```sh
cargo install codecite --locked
```

This builds the `codecite` binary from crates.io with the pinned dependency
versions. It needs Rust 1.99.1 or newer and a C compiler, since the
tree-sitter grammars and SQLite are compiled from C. Git sources also need
`git` on `PATH`.

## Quick start

Corpus manifests live in this repository, under `corpora/`. `codecite index
<corpus>` reads `corpora/<corpus>.toml` from the current directory, so run it
from a checkout, or pass `--manifest <file>`. The identity corpus indexes
identity-model from `~/repos/auth/identity-model`, so clone it there first.

```console
$ git clone https://github.com/jamescrowley321/codecite && cd codecite
$ codecite index identity
indexed corpus `identity` into ~/.local/share/codecite/identity.db
documents: 865 written, 0 unchanged, 0 removed
chunks: 12187 written
fetched: 0 downloaded, 39 from the cache
skipped: 39 ignored, 7 deny-listed, 4 symlink, 1 binary
elapsed: 6.16s
```

The first run downloads the 39 pinned specs (`fetched: 39 downloaded`); later
runs read them from the cache. `codecite search` ranks chunks by BM25 and
cites each one:

```console
$ codecite search identity "validate DPoP proof jti" --limit 2
2 results · adapt · indexed 2026-10-20T20:34:15Z
1. identity-model@9458d60f4f2f:rust/src/dpop/proof.rs#L207-L212 · function new_jti · 32.7
   fn new_jti() -> Result<String> {
2. identity-model@9458d60f4f2f:go/internal/conformance/dpop_test.go#L23-L87 · function runDPoPVector · 32.2
   func runDPoPVector(t *testing.T, label, base string, v HTTPVector) {
```

`codecite read` prints the text a citation points at, for the citations
search prints and the forms people write:

```console
$ codecite read identity "OIDC Core §3.1.3.7 step 13"
openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13
step · ID Token Validation · SHOULD · quote · indexed 2026-10-20T20:34:20Z

13. If the auth_time Claim was requested, either through a specific request for this Claim or by using the max_age parameter, the Client SHOULD check the auth_time Claim value and request re-authentication if it determines too much time has elapsed since the last End-User authentication.
```

`codecite corpora` lists what is indexed, as the MCP `corpora` tool returns it:

```console
$ codecite corpora
{"corpora":[{"age_days":0.0,"corpus":"self","indexed_at":"2026-10-20T20:49:36Z","max_age_days":30,"size_bytes":7655424,"sources":1,"stale":false}]}
```

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
