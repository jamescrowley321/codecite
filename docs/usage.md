# Using codecite

How to index a corpus, query it, and hand it to an agent. `docs/design.md`
explains why codecite works the way it does; this guide covers what to type.

## Concepts

- **Corpus:** a named set of sources indexed together into one SQLite file,
  `~/.local/share/codecite/<corpus>.db` on Linux (`$XDG_DATA_HOME` moves it).
- **Manifest:** the committed TOML file, `corpora/<corpus>.toml`, that lists a
  corpus's sources, each pinned to a commit or a sha256, with its license class.
- **Citation:** the string that names one span. Every result carries one, and
  `read` turns one back into the exact text.
- **License class:** how an agent may use a source: `adapt` (adapt the code
  with attribution), `approach_only` (describe the approach, never copy),
  `quote` (normative spec text: quote exactly) or `excluded` (not indexed).

```mermaid
flowchart LR
  M["corpora/#lt;corpus#gt;.toml"] -->|codecite index| DB[("#lt;corpus#gt;.db")]
  DB -->|codecite search| R["ranked results,<br/>each with a citation"]
  R -->|codecite read| T["exact text under its<br/>pinned citation"]
  DB -->|codecite mcp| A["agent over MCP"]
```

## Write a manifest

A manifest names the corpus and lists its sources. `kind` is `git` when left out.

```toml
name = "mywork"
max_age_days = 30              # queries refuse an older index unless --allow-stale

# A checkout on this machine, indexed as its HEAD plus uncommitted changes.
[[source]]
kind = "local"
path = "~/repos/myservice"
license = "Apache-2.0"
use = "adapt"
exclude = ["vendor/**"]

# A git repository at a pinned commit, kept as a partial clone in the cache.
[[source]]
url = "https://github.com/panva/openid-client"
commit = "<40-character SHA>"
license = "MIT"
use = "adapt"
include = ["lib/**"]

# A spec page, pinned by the sha256 of its bytes.
[[source]]
kind = "document"
name = "rfc9449"
url = "https://www.rfc-editor.org/rfc/rfc9449.html"
sha256 = "<64 hex characters>"
license = "IETF Trust"
use = "quote"
current = true
aliases = ["RFC 9449", "DPoP"]
```

| Field | Sources | Meaning |
| --- | --- | --- |
| `use` | all, required | `adapt`, `approach_only`, `quote` or `excluded` |
| `license` | all | The license, as people write it |
| `name` | all; required for documents | Defaults to the checkout's directory or the URL's last segment |
| `path` | `local` | The top of a git work tree; `~` expands |
| `url`, `commit` | `git` | The repository and a 40-character lowercase commit |
| `include`, `exclude` | `local`, `git` | Globs over paths |
| `url`, `sha256` | `document` | The page and the 64-character lowercase hash of its bytes |
| `current` | `document` | Exactly one pinned version of each name is current |
| `aliases` | `document` | Names people write, such as `OIDC Core` |
| `obsoleted_by`, `updated_by` | `document` | Hand-written relations to later documents |

Unknown keys are errors, and every error names the source and the field.

To pin a new or changed document, leave a placeholder hash and print the
hash each URL serves now:

```sh
codecite index mywork --print-hashes
```

## Index

```sh
codecite index mywork                      # reads corpora/mywork.toml
codecite index mywork --manifest ~/m.toml  # or any manifest file
```

Run it from a checkout of the repository that holds the manifest. The first run
downloads documents and clones git sources into `~/.cache/codecite`; later runs
work offline and rewrite only the files whose content changed. A document whose
bytes no longer match its pin, or a commit the repository doesn't have, stops
the run before anything is written.

The run ends with a summary:

```text
indexed corpus `identity` into ~/.local/share/codecite/identity.db
documents: 865 written, 0 unchanged, 0 removed
chunks: 12187 written
fetched: 0 downloaded, 39 from the cache
skipped: 39 ignored, 7 deny-listed, 4 symlink, 1 binary
elapsed: 2.12s
```

Skipped files are never indexed: what git ignores, deny-listed secrets (`.env*`,
keys and certificates), symlinks, binaries and files over 1 MiB.

## Search

```sh
codecite search identity "DPoP proof jti replay"
codecite search identity "verify proof" --language rust --limit 5
codecite search identity "nonce" --path "py/src/*" --json
codecite search identity "token_type_hint" --only spec
```

A query matches identifiers however they are written: `validate token` finds
`validateToken`, `validate_token` and `validate-token`. A chunk whose symbol is
exactly the query comes first; the rest rank by BM25, weighting symbol names
over paths over body text. Spec results come from each document's current
version.

Specs and code are ranked separately and then merged, so code that repeats a
term doesn't bury the spec section that defines it. When the query names a
symbol, code keeps the top slots and the best spec sections take the last few;
otherwise the results alternate, spec first. `--only spec` or `--only code`
searches one kind.

```text
1. rfc9449@sha256:2d8657f07291#section-11.1
   section · score 38.85 · quote · indexed 2026-10-20T22:56:30Z
     11.1. DPoP Proof Replay
```

Each result shows its rank and citation, then its kind (`function`, `class`,
`section`, `step` or `window`), symbol, score, license class and when it was
indexed, then the first lines of its text.

## Read

`read` prints the exact text a citation points at and always names the pinned
version it came from.

```sh
codecite read identity "OIDC Core §3.1.3.7 step 11"
codecite read identity "RFC 9449 §4.3" --max-lines 20
codecite read identity "identity-model@9458d60:rust/src/dpop/proof.rs#L207-L212"
```

It accepts:

| Form | Example |
| --- | --- |
| Code citation (a commit may be abbreviated) | `identity-model@9458d60:rust/src/dpop/proof.rs#L207-L212` |
| Spec citation | `rfc9449@sha256:2d8657f07291#section-4.3` |
| Spec step | `openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-11` |
| Written forms, with the manifest's aliases | `RFC 9449 §4.3`, `RFC9449 Section 4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)` |
| A bare section, when only one current document has it | `§3.1.3.7 step 11` |
| A spec URL with its anchor | `https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3` |

A citation that doesn't resolve to exactly one span is an error that lists the
candidates, never a guess:

```text
$ codecite read identity "§4.3"
Error: §4.3 names no document and is not in exactly one current document; name one: rfc6749@sha256:535362fa3b4c#section-4.3, rfc9449@sha256:2d8657f07291#section-4.3, …
```

## Check a citation

`cite` checks a spec citation: whether it names exactly one section or step in
the corpus, what that text is, and why not when it doesn't.

```sh
codecite cite identity "OIDC Core §3.1.3.7 step 13"
codecite cite identity "§4.3" --json
codecite cite identity "[OIDC Core §3.1.3.6](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)"
```

It accepts every spec form `read` does, and one more:

| Form | Example |
| --- | --- |
| A Markdown link, its text a citation and its target a spec URL with its anchor | `[OIDC Core §3.1.3.7](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)` |
| The same, with a bare section taking the URL's document | `[§3.1.3.7 step 13](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)` |

A citation that resolves prints as `read` prints it, with its text cut to 40
lines. An older pinned version, or a document another obsoletes, says what
supersedes it:

```text
$ codecite cite identity "RFC 2616 §14.9"
rfc2616@sha256:261600000000#section-14.9
section · Cache-Control · quote · indexed 2026-10-20T22:56:30Z
superseded by rfc9110
…
```

One that doesn't resolve is still a successful check, exiting 0, with its
findings and the pinned citations it could have meant:

```text
$ codecite cite identity "[OIDC Core §3.1.3.6](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation)"
unresolved
anchor_label_mismatch: OIDC Core §3.1.3.6 is openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.6, but https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation is openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7
candidates:
  openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.6
  openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7
```

`--json` prints the result as one compact JSON object, here for
`"RFC 9449 §4.3 step 99"`:

```json
{"candidates":["rfc9449@sha256:2d8657f07291#section-4.3/step-12","rfc9449@sha256:2d8657f07291#section-4.3/step-11","rfc9449@sha256:2d8657f07291#section-4.3/step-10","rfc9449@sha256:2d8657f07291#section-4.3/step-9","rfc9449@sha256:2d8657f07291#section-4.3/step-8"],"findings":[{"candidates":["rfc9449@sha256:2d8657f07291#section-4.3/step-12","rfc9449@sha256:2d8657f07291#section-4.3/step-11","rfc9449@sha256:2d8657f07291#section-4.3/step-10","rfc9449@sha256:2d8657f07291#section-4.3/step-9","rfc9449@sha256:2d8657f07291#section-4.3/step-8"],"section":"rfc9449@sha256:2d8657f07291#section-4.3","step":99,"type":"no_such_step"}],"resolved":false}
```

The findings are `no_such_section`, `no_such_step`, `anchor_label_mismatch`
(a link whose anchor is another section than its text) and
`ambiguous_document` (a bare § that is not in exactly one current document).
Anything else wrong is an error that exits non-zero, as `read`'s are: text that
is no citation, a malformed one, an unknown document or version, a stale index,
and a code citation, which `read` reads instead.

## List corpora

```sh
codecite corpora
```

```json
{"corpora":[{"age_days":0.0,"corpus":"identity","indexed_at":"2026-10-20T22:56:30Z","max_age_days":30,"size_bytes":41594880,"sources":40,"stale":false}]}
```

## Connect an agent

`codecite mcp` serves MCP over stdio. stdout carries only protocol messages;
it prints one line to stderr naming the data directory. Register it with
Claude Code:

```sh
claude mcp add codecite -- codecite mcp
```

The server's tools, `corpora`, `search` and `read`, return the same JSON as
the CLI commands of the same name print (with `--json` for `search` and
`read`), byte for byte, so anything an agent sees can be reproduced from a
terminal. `search` and `read` take an optional `corpus`; left out or empty,
it is the only corpus in the data directory. `limit` is at most 50 and
`max_lines` at most 500, for the tools and the CLI alike. An unknown corpus, a stale index or a
citation that isn't in the corpus is a tool error whose structured content is
`{"error": {"kind", "message", "candidates"}}`; each tool's output schema
admits both its result and this error. A `read` of a spec, whose
license class is `quote`, carries an `instruction` to quote the text exactly
with its citation.

## The identity corpus

`corpora/identity.toml` indexes identity-model from `~/repos/auth/identity-model`
and the 39 OAuth, OpenID and FAPI documents it cites, with the older versions
it has cited pinned beside the current ones. Clone identity-model there, then
from a codecite checkout:

```sh
codecite index identity
cargo test -p codecite --test identity -- --ignored --nocapture   # its lookup check
```
