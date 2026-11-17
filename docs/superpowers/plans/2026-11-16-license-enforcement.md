# License-class enforcement

Issue #223, milestone M5 Manifests and licensing.

## What exists

- `codecite add` and SPDX detection (`crates/codecite-index/src/license.rs`):
  a repository's top-level license files are matched against embedded texts
  and `add` proposes `license` and `use`.
- Every result carries `license_class`; `excluded` sources are never fetched,
  cloned or indexed, and a source turned `excluded` is deleted on the next
  run.
- Only `quote` results carry an `instruction` (through `read::instruction`).

## What is missing

1. Results from an `approach_only` source say nothing about not copying,
   though the design promises an explicit instruction.
2. Nothing stops `use = "adapt"` on a copyleft `license`.

## Steps

1. **Manifest check, test-first.** In `license.rs`, `forbids_adapting(license)`
   parses `license` with `spdx::Expression::parse_mode(.., LAX)` and evaluates
   it with copyleft (`LicenseId::is_copyleft`, the `approach_only` entries of
   `LICENSES`) and a short list of weak-copyleft and source-available ids
   counted as failing. It returns the failing ids, or `None` when the
   expression is met or is not SPDX. Tests: GPL, AND, AGPL, LGPL, MPL, EPL,
   SSPL, `WITH` exceptions, `GPL-3.0`, `GPL-2.0+` fail; MIT, OR with MIT,
   CC0, `LicenseRef-*`, `IETF Trust`, empty pass.
2. `ManifestError::LicenseForbidsAdapt` names the source, the license and the
   failing ids; `parse_source` returns it for `use = "adapt"`. Tests: git and
   local sources refused; OR, non-SPDX and missing licenses pass; other uses
   pass; every manifest in `corpora/` parses.
3. **Instruction, test-first.** `APPROACH_ONLY_INSTRUCTION` in
   `codecite-core`; `read::instruction` returns it for `approach_only`, so
   `read`, `cite`, `context` and `requirements` carry it; `copy_instruction`
   gives search hits, grep matches and definitions the same field, only for
   `approach_only`. One test builds the definition fixture as
   `approach_only` and as `adapt` and checks all five results and the JSON.
4. The MCP server's instructions and the `read` tool's description say what
   `approach_only` and `adapt` allow; the instructions test checks it.
5. `docs/design.md` (Corpus manifests and licensing, Interfaces) and
   `docs/usage.md` (Write a manifest, Connect an agent) describe both.
6. `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D
   warnings`, `cargo test --workspace --locked`.
