# Prepare codecite 0.0.1 for crates.io Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #30. Give every crate the metadata crates.io shows (description, keywords, categories, readme), move all five crates to 0.0.1 together, add install and quick-start examples with real output to the README, and show that `cargo publish --dry-run` passes for every crate in dependency order. The owner runs `cargo publish` and tags the release; this plan stops at the dry run.

**Architecture:** No Rust code changes. The work is package metadata and docs:
- `Cargo.toml` (workspace): `version = "0.0.1"` and `readme = "README.md"` in `[workspace.package]`, and the four internal path dependencies at `version = "0.0.1"`.
- Each `crates/*/Cargo.toml`: `readme.workspace = true`, plus its own `keywords` and `categories`. Each already has a `description`.
- `crates/*/LICENSE` and `crates/*/NOTICE`: symlinks to the root files, so every package carries the Apache-2.0 license text and the attribution notices.
- `README.md`: Install and Quick start sections with the real output of `codecite index identity`, `search` and `read`.
- `docs/design.md`: how the crates are versioned and published.

**Tech Stack:** Rust 1.99.1 (edition 2024), Cargo 1.99.1. No new dependencies; `cargo deny check` is unchanged. Prototyped: every gate passes (288 passed, 7 ignored; deny ok), and `cargo publish --workspace --dry-run` packages, verifies and stops before uploading all five crates.

**Spec:** Issue #30; `docs/design.md#milestones` (M2 ends with "Publish `codecite` 0.0.1") and `docs/design.md#architecture` (the five crates). The gaps the issue leaves are settled under Global Constraints. Task 3 writes the lasting ones into design.md.

## Global Constraints

- **Names.** `codecite`, `codecite-core`, `codecite-index`, `codecite-lsp` and `codecite-mcp` are all unclaimed on crates.io (`https://crates.io/api/v1/crates/<name>` returns 404 for each on 2026-10-20).
- **One version.** All five crates take `version.workspace = true`, so the bump is one line in `[workspace.package]` plus the four internal dependency requirements. Cargo reads a `0.0.x` requirement as exactly `=0.0.x`, so those must move with it. `Cargo.lock` changes only in the five `version` lines.
- **Dependency order.** `codecite-core` has no internal dependencies; `codecite-index`, `codecite-lsp` and `codecite-mcp` depend on core; `codecite` depends on all four. A per-crate dry run of anything above core fails until core is on crates.io (`no matching package named codecite-core found`), so the dry run is `cargo publish --workspace --dry-run`. Since Cargo 1.90 it packages every member, verifies each against a temporary local registry that holds the ones below it, and then stops before each upload, in dependency order. The real publish is `cargo publish --workspace`, or the five `cargo publish -p <crate>` commands in the order above.
- **README.** Every crate uses the root `README.md` through `readme.workspace = true`; Cargo resolves the path against the workspace root and packages the file as `README.md`. The internal crates are not meant to be used alone, so one README is enough.
- **License files.** `cargo package --list` shows no `LICENSE` or `NOTICE` today, because they sit at the workspace root. Apache-2.0 asks that both travel with the code, so each crate gets `LICENSE -> ../../LICENSE` and `NOTICE -> ../../NOTICE` symlinks; Cargo packages a symlink as the file it points to. No `license-file`: `license = "Apache-2.0"` is an SPDX expression and stays.
- **No `homepage`.** crates.io's guidance is to leave it unset when it would only repeat `repository`.
- **Keywords and categories.** At most five keywords, each at most 20 ASCII characters; categories must be crates.io slugs. `command-line-utilities` only on the binary.
- **Manifests stay in the repository.** `codecite index <corpus>` reads `corpora/<corpus>.toml` from the current directory (or `--manifest`), and the package carries no corpora, so the README's quick start runs from a checkout. The identity corpus names `~/repos/auth/identity-model` as its local source; the README says so.
- **Real output.** The README's output comes from the prototype's run of the debug binary with a scratch `XDG_DATA_HOME`, so the owner's `~/.local/share/codecite/identity.db` was never written. The database path in the index output is shown as the default, `~/.local/share/codecite/identity.db`. Everything else is verbatim.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`. Before the PR, also run `cargo publish --workspace --dry-run` and put its output in the PR body.

## Review Focus

- The packaged file lists: `cargo package --list -p <crate>` shows `LICENSE`, `NOTICE` and `README.md` for every crate, and nothing private.
- The dry run's order and its last line for each crate (`aborting upload due to dry run`).
- The README examples match what the binary prints.

---

### Task 1: Package metadata and version 0.0.1

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `crates/codecite/Cargo.toml`, `crates/codecite-core/Cargo.toml`, `crates/codecite-index/Cargo.toml`, `crates/codecite-lsp/Cargo.toml`, `crates/codecite-mcp/Cargo.toml`
- Create: `crates/*/LICENSE`, `crates/*/NOTICE` (symlinks)

- [ ] **Step 1: Write the failing check**

There is no Rust behavior to test, so the check is what Cargo would package and publish:

```sh
for c in codecite-core codecite-index codecite-lsp codecite-mcp codecite; do
  cargo package --list -p "$c" --allow-dirty | grep -qx LICENSE || echo "$c: no LICENSE"
done
cargo metadata --format-version 1 --no-deps | grep -o '"version":"0.0.[01]"' | sort | uniq -c
```

- [ ] **Step 2: Run it to see it fail**

Expected: `<crate>: no LICENSE` for all five, and `5 "version":"0.0.0"`.

> **Learn: crates.io packaging.** `cargo package` builds the `.crate` file `cargo publish` would upload: a tarball of the crate's directory, filtered by `.gitignore` (and `include`/`exclude`, unset here), with `Cargo.toml` rewritten so `workspace = true` fields become plain values and path dependencies become registry ones (the original is kept as `Cargo.toml.orig`). `cargo package --list` prints that file list without building it. Only the crate's own directory goes in, which is why the root `LICENSE` and `NOTICE` are missing; a symlink inside the crate is followed and packaged as a regular file. For a binary crate, `Cargo.lock` goes in too, so `cargo install --locked` builds with the versions CI tested.

- [ ] **Step 3: Apply the metadata**

Apply this diff (`git apply`), leaving `Cargo.lock` for Cargo to update:

`````diff
diff --git a/Cargo.toml b/Cargo.toml
index 8480b75..dc9670a 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -3,11 +3,12 @@ resolver = "3"
 members = ["crates/*"]
 
 [workspace.package]
-version = "0.0.0"
+version = "0.0.1"
 edition = "2024"
 rust-version = "1.99.1"
 license = "Apache-2.0"
 repository = "https://github.com/jamescrowley321/codecite"
+readme = "README.md"
 
 [workspace.lints.rust]
 unsafe_code = "forbid"
@@ -20,11 +21,11 @@ todo = "warn"
 
 [workspace.dependencies]
 # The internal crates' versions must match `workspace.package.version`: for
-# 0.0.x, Cargo treats "0.0.0" as exactly =0.0.0, so a bump changes all five.
-codecite-core = { path = "crates/codecite-core", version = "0.0.0" }
-codecite-index = { path = "crates/codecite-index", version = "0.0.0" }
-codecite-lsp = { path = "crates/codecite-lsp", version = "0.0.0" }
-codecite-mcp = { path = "crates/codecite-mcp", version = "0.0.0" }
+# 0.0.x, Cargo treats "0.0.1" as exactly =0.0.1, so a bump changes all five.
+codecite-core = { path = "crates/codecite-core", version = "0.0.1" }
+codecite-index = { path = "crates/codecite-index", version = "0.0.1" }
+codecite-lsp = { path = "crates/codecite-lsp", version = "0.0.1" }
+codecite-mcp = { path = "crates/codecite-mcp", version = "0.0.1" }
 clap = { version = "4", features = ["derive"] }
 thiserror = "2"
 anyhow = "1"
diff --git a/crates/codecite-core/Cargo.toml b/crates/codecite-core/Cargo.toml
index 847dc64..6f503e2 100644
--- a/crates/codecite-core/Cargo.toml
+++ b/crates/codecite-core/Cargo.toml
@@ -6,6 +6,9 @@ edition.workspace = true
 rust-version.workspace = true
 license.workspace = true
 repository.workspace = true
+readme.workspace = true
+keywords = ["code-search", "citations", "sqlite", "fts5"]
+categories = ["database", "development-tools"]
 
 [lints]
 workspace = true
diff --git a/crates/codecite-index/Cargo.toml b/crates/codecite-index/Cargo.toml
index ed085ef..4be5597 100644
--- a/crates/codecite-index/Cargo.toml
+++ b/crates/codecite-index/Cargo.toml
@@ -6,6 +6,9 @@ edition.workspace = true
 rust-version.workspace = true
 license.workspace = true
 repository.workspace = true
+readme.workspace = true
+keywords = ["code-search", "tree-sitter", "indexing", "rfc", "citations"]
+categories = ["development-tools", "parser-implementations", "text-processing"]
 
 [lints]
 workspace = true
diff --git a/crates/codecite-lsp/Cargo.toml b/crates/codecite-lsp/Cargo.toml
index 6026640..d8b0720 100644
--- a/crates/codecite-lsp/Cargo.toml
+++ b/crates/codecite-lsp/Cargo.toml
@@ -6,6 +6,9 @@ edition.workspace = true
 rust-version.workspace = true
 license.workspace = true
 repository.workspace = true
+readme.workspace = true
+keywords = ["code-search", "lsp", "citations"]
+categories = ["development-tools"]
 
 [lints]
 workspace = true
diff --git a/crates/codecite-mcp/Cargo.toml b/crates/codecite-mcp/Cargo.toml
index 5ccb201..c317336 100644
--- a/crates/codecite-mcp/Cargo.toml
+++ b/crates/codecite-mcp/Cargo.toml
@@ -6,6 +6,9 @@ edition.workspace = true
 rust-version.workspace = true
 license.workspace = true
 repository.workspace = true
+readme.workspace = true
+keywords = ["code-search", "mcp", "citations"]
+categories = ["development-tools"]
 
 [lints]
 workspace = true
diff --git a/crates/codecite/Cargo.toml b/crates/codecite/Cargo.toml
index e766f23..599ee77 100644
--- a/crates/codecite/Cargo.toml
+++ b/crates/codecite/Cargo.toml
@@ -6,6 +6,9 @@ edition.workspace = true
 rust-version.workspace = true
 license.workspace = true
 repository.workspace = true
+readme.workspace = true
+keywords = ["code-search", "citations", "mcp", "sqlite", "rfc"]
+categories = ["command-line-utilities", "development-tools"]
 
 [lints]
 workspace = true
`````

Then link the license files and let Cargo update the lock file:

```sh
for c in crates/*; do ln -s ../../LICENSE "$c/LICENSE"; ln -s ../../NOTICE "$c/NOTICE"; done
cargo metadata --format-version 1 >/dev/null
git diff --stat Cargo.lock
```

Expected: `Cargo.lock | 10 +++++-----`, the five `version = "0.0.0"` lines now `"0.0.1"`.

> **Learn: workspace inheritance.** `[workspace.package]` holds fields members can take with `<field>.workspace = true`. `version`, `license` and `readme` are inherited so one edit changes all five; `keywords` and `categories` are per crate. An inherited `readme` path is resolved against the workspace root. A path dependency also needs a `version` to be publishable: Cargo uses the path in the workspace and the version on crates.io.

- [ ] **Step 4: Run the check again**

Expected: no `no LICENSE` lines, and `5 "version":"0.0.1"`.

- [ ] **Step 5: Run the dry run**

```sh
cargo publish --workspace --dry-run --allow-dirty 2>&1 | grep -v -E '^ *(Compiling|Unpacking|Updating)'
```

Expected, in this order (paths shortened):

```text
   Packaging codecite-core v0.0.1 (crates/codecite-core)
    Packaged 23 files, 233.1KiB (56.7KiB compressed)
   Packaging codecite-index v0.0.1 (crates/codecite-index)
    Packaged 39 files, 331.9KiB (87.5KiB compressed)
   Packaging codecite-lsp v0.0.1 (crates/codecite-lsp)
    Packaged 9 files, 27.8KiB (10.4KiB compressed)
   Packaging codecite-mcp v0.0.1 (crates/codecite-mcp)
    Packaged 9 files, 27.7KiB (10.4KiB compressed)
   Packaging codecite v0.0.1 (crates/codecite)
    Packaged 13 files, 107.0KiB (30.1KiB compressed)
   Verifying codecite-core v0.0.1 (crates/codecite-core)
   …
   Uploading codecite v0.0.1 (crates/codecite)
warning: aborting upload due to dry run
```

The sizes above include Task 2's README; before it, each is a few KiB smaller.

> **Learn: publishing a workspace in order.** crates.io resolves a published crate's dependencies from crates.io only, so `codecite-index` can't be verified until `codecite-core` 0.0.1 exists there. `cargo publish -p codecite-index --dry-run` fails for exactly that reason. `cargo publish --workspace` (Cargo 1.90+) sorts the members by their dependencies, verifies each one against a temporary registry under `target/package/tmp-registry` that holds the ones packaged before it, and uploads them in that order, waiting for each to appear in the index before the next. `--dry-run` does everything but the upload. Publishing is permanent: a version can be yanked but never deleted or reused.

- [ ] **Step 6: Run the gates and commit**

```sh
cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check
git add Cargo.toml Cargo.lock crates/*/Cargo.toml crates/*/LICENSE crates/*/NOTICE
git commit -m "chore: add crates.io metadata and move every crate to 0.0.1 (#30)"
```

### Task 2: README install and quick start

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Apply the README diff**

`````diff
diff --git a/README.md b/README.md
index 6f8f032..718d42b 100644
--- a/README.md
+++ b/README.md
@@ -26,6 +26,64 @@ and the spec forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`
 a spec URL). `corpora/identity.toml` is the first real corpus: identity-model and
 the OAuth and OpenID specs it cites. The MCP server comes next.
 
+## Install
+
+```sh
+cargo install codecite --locked
+```
+
+This builds the `codecite` binary from crates.io with the pinned dependency
+versions. It needs Rust 1.99.1 or newer and a C compiler, since the
+tree-sitter grammars and SQLite are compiled from C. Git sources also need
+`git` on `PATH`.
+
+## Quick start
+
+Corpus manifests live in this repository, under `corpora/`. `codecite index
+<corpus>` reads `corpora/<corpus>.toml` from the current directory, so run it
+from a checkout, or pass `--manifest <file>`. The identity corpus indexes
+identity-model from `~/repos/auth/identity-model`, so clone it there first.
+
+```console
+$ git clone https://github.com/jamescrowley321/codecite && cd codecite
+$ codecite index identity
+indexed corpus `identity` into ~/.local/share/codecite/identity.db
+documents: 865 written, 0 unchanged, 0 removed
+chunks: 12187 written
+fetched: 0 downloaded, 39 from the cache
+skipped: 39 ignored, 7 deny-listed, 4 symlink, 1 binary
+elapsed: 6.16s
+```
+
+The first run downloads the 39 pinned specs (`fetched: 39 downloaded`); later
+runs read them from the cache. `codecite search` ranks chunks by BM25 and
+cites each one:
+
+```console
+$ codecite search identity "validate DPoP proof jti" --limit 2
+1. identity-model@9458d60f4f2f20227bddf2594fd7d10cba66ea0a:rust/src/dpop/proof.rs#L207-L212
+   function new_jti · score 32.68 · adapt · indexed 2026-10-20T20:34:15Z
+     fn new_jti() -> Result<String> {
+         let mut bytes = [0u8; JTI_BYTES];
+         getrandom::fill(&mut bytes)
+2. identity-model@9458d60f4f2f20227bddf2594fd7d10cba66ea0a:go/internal/conformance/dpop_test.go#L23-L87
+   function runDPoPVector · score 32.20 · adapt · indexed 2026-10-20T20:34:15Z
+     func runDPoPVector(t *testing.T, label, base string, v HTTPVector) {
+     	t.Helper()
+     	if v.Expect.Outcome != OutcomeAccept {
+```
+
+`codecite read` prints the text a citation points at, for the citations
+search prints and the forms people write:
+
+```console
+$ codecite read identity "OIDC Core §3.1.3.7 step 13"
+openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13
+step · ID Token Validation · SHOULD · quote · indexed 2026-10-20T20:34:20Z
+
+13. If the auth_time Claim was requested, either through a specific request for this Claim or by using the max_age parameter, the Client SHOULD check the auth_time Claim value and request re-authentication if it determines too much time has elapsed since the last End-User authentication.
+```
+
 ## Planned
 
 - Code-aware full-text search (BM25 over split identifiers), with optional
`````

- [ ] **Step 2: Check the examples against the binary**

```sh
cargo build -p codecite
export XDG_DATA_HOME=$(mktemp -d)
target/debug/codecite index identity
target/debug/codecite search identity "validate DPoP proof jti" --limit 2
target/debug/codecite read identity "OIDC Core §3.1.3.7 step 13"
```

Expected: the same counts, citations and text as the README. The `indexed` timestamps and `elapsed` differ, and the first run on a fresh cache downloads the 39 documents. If identity-model's HEAD has moved, the search citations name the new commit; update the README to match what the binary prints.

- [ ] **Step 3: Commit**

```sh
git add README.md
git commit -m "docs: add install and quick start examples to the README (#30)"
```

### Task 3: Docs

**Files:**
- Modify: `docs/design.md`

- [ ] **Step 1: Apply the design.md diff**

`````diff
diff --git a/docs/design.md b/docs/design.md
index 4d89f8a..e4ce94d 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -71,6 +71,8 @@ flowchart LR
 
 The workspace has five crates: `codecite-core` (schema and query engine), `codecite-index` (pipeline and providers), `codecite-lsp`, `codecite-mcp`, and the `codecite` binary that ties them together.
 
+All five publish to crates.io at one shared version, `workspace.package.version`. The internal crates require each other's exact version, since Cargo reads a `0.0.x` requirement as `=0.0.x`, so a release bumps all five together. `cargo publish --workspace` packages, verifies and uploads them in dependency order (`codecite-core`, then `codecite-index`, `codecite-lsp` and `codecite-mcp`, then `codecite`). Each package carries the root `README.md`, `LICENSE` and `NOTICE`; the crates link the last two as symlinks, which Cargo packages as files. Corpus manifests are not packaged: `codecite index <corpus>` reads `corpora/<corpus>.toml` from the current directory, or the file `--manifest` names.
+
 ## Data model
 
 Each corpus is one SQLite file in WAL mode, holding sources, chunks, a full-text index, optional vectors and a provider-tagged symbol graph.
`````

- [ ] **Step 2: Run the gates and commit**

```sh
cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check
git add docs/design.md
git commit -m "docs: describe how the crates are versioned and published (#30)"
```

## Publishing (owner, after merge)

Not part of this plan; listed so the PR can quote it:

```sh
cargo publish --workspace --dry-run
cargo publish --workspace
git tag -a v0.0.1 -m "codecite 0.0.1" && git push origin v0.0.1
```

`cargo publish --workspace` needs `cargo login` with a crates.io token. The per-crate equivalent, in order: `cargo publish -p codecite-core`, `-p codecite-index`, `-p codecite-lsp`, `-p codecite-mcp`, `-p codecite`.
