## Blind peer review — PR #197

head `eefee4b50a78bd85d7c91644fdf722c1deae0b57`

Verdict: **PASS**

Scope: diff from the merge base with `origin/feat/one-ranking` to the head (44 files). Each lens ran as a fresh, independent reviewer that saw only the diff.

| Lens | Result | MUST FIX | SHOULD FIX | NITPICK | Summary |
| --- | --- | --- | --- | --- | --- |
| Cold Read | PASS | 0 | 3 | 3 | `codecite add` is mostly sound. Three problems: rewriting the manifest replaces its permissions and symlinks, the ls-remote parse can pin the wrong commit, and dual-licensed repositories get AND where OR is meant. |
| Edge Cases | PASS | 0 | 2 | 1 | `add` can write a manifest that `index` then refuses, because two sources get the same name. It also replaces a symlinked manifest instead of writing through the link. |
| Acceptance Criteria | PASS | 0 | 0 | 4 | All four criteria from #184 are met: pinning, license detection, many repositories and the Done-when run. Each has tests that would fail without it. |
| Security Review | PASS | 0 | 0 | 0 | No findings. Every input to `add` comes from the local user, and the untrusted repository contents it reads are handled safely. |
| Red Team | PASS | 0 | 1 | 1 | `remote_head` drops the existing guard that stops git from reading an enclosing repository's config. A URL with credentials in it is written verbatim into the manifest. |
| Policy & Provenance | PASS | 0 | 0 | 0 | The PR names the harness and the model, the human-accountability box is ticked, and there are no secrets or private data. |
| OWASP Web Top 10 | PASS (skipped) | 0 | 0 | 0 | Skipped: the diff has no web/HTTP surface. It is a local CLI subcommand with no handler, route, MCP tool or session code. |
| OWASP LLM Top 10 | PASS | 0 | 0 | 1 | Light LLM surface only (the `use` class reaches agents over MCP). Pins, quoting and atomic writes are sound. One hardening nitpick. |

### SHOULD FIX

- **Cold Read / Edge Cases**, `crates/codecite-index/src/add.rs` (`write_atomically`): `NamedTempFile` creates its file with mode 0600, and `persist()` renames it over the manifest. Every `add` therefore resets a 0644 manifest to owner-only. It also replaces a symlinked manifest with a regular file and never updates the link's target. *Fix:* resolve the symlink and write next to the real target, copy the original file's permissions onto the temp file, and add a Unix test that checks the mode survives.
- **Cold Read**, `crates/codecite-index/src/clone.rs` (`remote_head`): the code takes the first line of `git ls-remote -- <url> HEAD` without checking that its ref column is exactly `HEAD`. The `HEAD` pattern also matches refs like `refs/remotes/origin/HEAD`. On a remote whose HEAD is dangling, `add` could pin that commit instead of returning `NoHead`. *Fix:* pick the line whose second field is exactly `HEAD`, and add a test for a dangling HEAD.
- **Cold Read**, `crates/codecite-index/src/license.rs`: when a repository has several license files, the detected ids are joined with ` AND `. The common LICENSE-MIT plus LICENSE-APACHE layout means `MIT OR Apache-2.0`, so `add` writes a wrong SPDX expression. The test `license_files_are_joined_sorted` locks this in. *Fix:* join with ` OR `, or leave `license` out and flag it for checking. Keeping the strictest `use` class is still right.
- **Edge Cases**, `crates/codecite-index/src/add.rs` (duplicate check): `add` treats a URL as already present only when the strings match exactly, and `Manifest::parse` does not check that source names are unique. Two examples: `https://github.com/a/tools` with `https://github.com/b/tools`, or `x.git` with `x`. Either produces a manifest that `index` refuses with `DuplicateSource`, yet `add` exits 0 and says to run `codecite index`. That breaks the code's own promise never to write a manifest `index` would refuse. *Fix:* compute each source's effective name the way `git_name` does, and give a colliding source a disambiguating name or return an error. Compare URLs after normalising a trailing `/` and `.git`.
- **Red Team**, `crates/codecite-index/src/clone.rs` (`remote_head`) [CWE-15]: `git ls-remote` runs in the process's current directory with no `-C` and no `GIT_CEILING_DIRECTORIES`. Git therefore applies the `.git/config` of any repository that encloses that directory, for example `core.sshCommand`, `url.*.insteadOf` with `ext::` or `credential.helper`. The existing `run()` helper guards against exactly this. The precondition is that the victim runs `codecite add` inside a directory tree the attacker controls, such as an extracted archive. *Fix:* route the call through the same hardened helper (a codecite-owned directory, a ceiling, and cleared `GIT_DIR`, `GIT_WORK_TREE` and `GIT_INDEX_FILE`), and add a test that a planted `insteadOf` is ignored.

### NITPICK

- **Cold Read**, `add.rs`: URL comparison is exact, so the same repository with and without `.git` or a trailing `/` is pinned and indexed twice.
- **Cold Read / Edge Cases**, `clone.rs`: `is_lower_hex(commit, 40)` rejects 64-character SHA-256 hashes, and the resulting `NoHead` message asks whether the repository is empty, which is misleading. Add a distinct "SHA-256 repositories are not supported" error.
- **Cold Read**, `add.rs`: `add` reads the manifest, does minutes of network and clone work, then writes back the text it read at the start, so any edit made during that window is lost. Re-read and compare before persisting, or lock the manifest.
- **Red Team**, `add.rs` (`source_table`) [CWE-312]: a URL with userinfo (`https://user:token@…`) is written in plain text to a manifest that is meant to be committed, and printed to stdout. Reject or strip userinfo and redact it in output.
- **OWASP LLM Top 10**, `license.rs` [LLM07:2026]: a top-level MIT match is proposed as `adapt` for the whole repository, even when it vendors GPL code or the text is a near-MIT variant. Nothing in the manifest marks the value as unconfirmed. Add a "proposed; confirm" comment, require a near-exact score for `adapt`, or warn on nested license files.
- **Acceptance Criteria**: AC-1 to AC-4 pass (these entries only restate the passes).

> Reviewed in-host: the lenses ran on the same model family that wrote this diff.
> A same-family reviewer shares the author's blind spots. CI reviews with a
> different family — see `docs/model-selection.md`.
