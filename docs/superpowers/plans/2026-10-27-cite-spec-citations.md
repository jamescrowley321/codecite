# Resolve spec citations with `codecite cite`

Issue: #34. The MCP `cite` tool is #36 and the claim checks are #35; this plan
covers the core operation and the CLI only.

## Approach

`cite` reuses `read`'s resolution and never forks it. `Store::cite` calls the
same code `Store::read` does, with the text capped at `MAX_SNIPPET_LINES`,
and turns the errors that mean "the corpus has the document but not this
span" into findings:

| `read` error | `cite` finding |
| --- | --- |
| `NoSuchSection` | `no_such_section` |
| `NoSuchStep` | `no_such_step` |
| `AmbiguousDocument` | `ambiguous_document` |

Every other error stays an error. A canonical code citation is the new error
`NotASpecCitation`, since `cite` checks spec citations.

The one form `read` doesn't take is a Markdown link,
`[OIDC Core §3.1.3.6](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)`.
`cite` resolves its URL, then its text, with a bare § taking its document
from the URL (`SpecReference::in_document`). When the text's section is not
the anchor's, the result is an `anchor_label_mismatch` finding naming both,
with both as candidates. When they agree, the result is the text's.

`superseded_by` comes from the resolved version's source row: its
`obsoleted_by` document (pinned when that document is indexed), else, when the
version is not current, the current version's pinned citation.

## Steps

1. `codecite-core`: `SpecReference::in_document`; `Error::NotASpecCitation`;
   `store/cite.rs` with `CiteOptions`, `CiteResult`, `Finding` and
   `Store::cite`; the `read` helpers it needs become `pub(super)`. Unit tests
   on a fixture written through `Store`, written first.
2. `codecite`: the `cite <corpus> <citation> [--json] [--allow-stale]`
   subcommand, its human form with control characters escaped, and
   `tests/cite.rs` on a fixture corpus written through `Store`. The ignored
   identity lookup test adds a `cite` of `OIDC Core §3.1.3.7 step 13`.
3. Docs: a "Check a citation" section in `docs/usage.md`; the `cite` row, its
   result fields, findings versus errors and the Markdown link form in
   `docs/design.md`.
4. `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D
   warnings`, `cargo test --workspace --locked`.
