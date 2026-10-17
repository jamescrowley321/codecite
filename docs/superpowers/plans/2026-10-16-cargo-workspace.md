# Cargo Workspace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #4: a virtual Cargo workspace with the five crates from the design, a pinned toolchain, and a `codecite` binary that answers `--version`.

**Architecture:** A root `Cargo.toml` with no package of its own (a virtual manifest) lists `crates/*` as members. Metadata, lints and internal dependencies are declared once under `[workspace.*]` and inherited by each crate. The four libraries are empty but documented. The binary uses `clap`'s derive API so later milestones add subcommands to the same `Cli` struct.

**Tech Stack:** Rust 1.99.1 (edition 2024), Cargo workspaces, `clap` 4 with the `derive` feature.

**Spec:** `docs/design.md#architecture` and `docs/design.md#milestones` (M1), and issue #4.

## Global Constraints

- Crates: `codecite-core` (schema and query engine), `codecite-index` (pipeline and providers), `codecite-lsp`, `codecite-mcp`, and the `codecite` binary.
- Dependency direction: `codecite` → all four libraries; `codecite-index`, `codecite-lsp`, `codecite-mcp` → `codecite-core`; `codecite-core` → none of them.
- License `Apache-2.0`; repository `https://github.com/jamescrowley321/codecite`.
- Toolchain pinned to exactly `1.99.1`, components `rustfmt` and `clippy`.
- Workspace version `0.0.0`. M2 publishes `0.0.1`; this plan publishes nothing.
- `Cargo.lock` is committed.
- Gates: `cargo build --workspace`, `cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` all pass.
- Use no dependencies beyond `clap`.

## Review Focus

- `codecite` with no arguments: prints help to stderr and exits with code 2. It must not panic or exit 0 silently. Test: `no_args_prints_usage_and_fails` in Task 2.
- Short flag `-V`: prints the same line as `--version`. Test: `short_version_flag_matches_long` in Task 2.
- An unknown flag such as `--nope` exits with code 2 and names the flag on stderr. Test: `unknown_flag_is_rejected` in Task 2.
- The printed version comes from `Cargo.toml`, not a hand-written string, so the M2 bump to `0.0.1` cannot drift. Test: `version_flag_prints_package_version` in Task 2 asserts against `env!("CARGO_PKG_VERSION")`.
- A contributor running a different local `rustc` gets `1.99.1` automatically through `rust-toolchain.toml`. Verified in Task 1 Step 3.

---

### Task 1: Workspace, toolchain and the four library crates

**Files:**
- Create: `rust-toolchain.toml`
- Create: `Cargo.toml` (virtual workspace)
- Create: `crates/codecite-core/Cargo.toml`, `crates/codecite-core/src/lib.rs`
- Create: `crates/codecite-index/Cargo.toml`, `crates/codecite-index/src/lib.rs`
- Create: `crates/codecite-lsp/Cargo.toml`, `crates/codecite-lsp/src/lib.rs`
- Create: `crates/codecite-mcp/Cargo.toml`, `crates/codecite-mcp/src/lib.rs`

**Interfaces:**
- Produces: `[workspace.dependencies]` entries `codecite-core`, `codecite-index`, `codecite-lsp`, `codecite-mcp` (each `{ path = "crates/<name>", version = "0.0.0" }`) and `clap = { version = "4", features = ["derive"] }`, which Task 2 consumes with `<name>.workspace = true`.

- [ ] **Step 1: Write `rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.99.1"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- [ ] **Step 2: Write the root `Cargo.toml`**

- `[workspace]`: `resolver = "3"`, `members = ["crates/*"]`.
- `[workspace.package]`: `version = "0.0.0"`, `edition = "2024"`, `rust-version = "1.99.1"`, `license = "Apache-2.0"`, `repository` as in Global Constraints.
- `[workspace.lints.rust]`: `unsafe_code = "forbid"`, `missing_docs = "warn"`.
- `[workspace.lints.clippy]`: `all = { level = "warn", priority = -1 }`, `dbg_macro = "warn"`, `todo = "warn"`.
- `[workspace.dependencies]`: as in the Interfaces block. Add `clap` here even though only Task 2 uses it, so all versions live in one place.

Learn: a virtual manifest has `[workspace]` but no `[package]`. Clippy runs with `-D warnings`, so `warn` lints become hard failures in CI and stay soft while iterating locally.

- [ ] **Step 3: Verify the toolchain pin**

Run: `rustup show active-toolchain`
Expected: begins with `1.99.1-` and says it is overridden by `rust-toolchain.toml`. If the toolchain isn't installed, rustup installs it on the first `cargo` call.

- [ ] **Step 4: Create the four library crates**

Each `Cargo.toml` has `name`, then `version.workspace = true`, `edition.workspace = true`, `rust-version.workspace = true`, `license.workspace = true`, `repository.workspace = true`, a one-line `description` taken from Global Constraints, and `[lints] workspace = true`. `codecite-index`, `codecite-lsp` and `codecite-mcp` declare `codecite-core.workspace = true` under `[dependencies]`. `codecite-core` declares none.

Each `src/lib.rs` holds only a `//!` crate doc comment of one or two sentences saying what the crate will own. `missing_docs` requires it.

- [ ] **Step 5: Run the gates**

Run: `cargo build --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all succeed. `cargo test` reports 0 tests for each crate.

Run: `cargo tree -p codecite-core --depth 1 && cargo tree -p codecite-mcp --depth 1`
Expected: `codecite-core` has no dependencies; `codecite-mcp` lists only `codecite-core`.

- [ ] **Step 6: Commit**

```bash
git add rust-toolchain.toml Cargo.toml Cargo.lock crates/
git commit -m "build: add the Cargo workspace and the four library crates"
```

### Task 2: The `codecite` binary with `--version`

**Files:**
- Create: `crates/codecite/Cargo.toml`
- Create: `crates/codecite/src/main.rs`
- Test: `crates/codecite/tests/cli.rs`
- Modify: `README.md` (add a short "Build" section)

**Interfaces:**
- Consumes: workspace dependencies from Task 1.
- Produces: `struct Cli` in `crates/codecite/src/main.rs`, `#[derive(clap::Parser)]` with `#[command(version, about, arg_required_else_help = true)]`. Later milestones add a `#[command(subcommand)]` field to it.

- [ ] **Step 1: Create `crates/codecite/Cargo.toml`**

Same inherited fields as the libraries, `description = "Index code and specs into SQLite and answer AI agents with cited results"`, and `[dependencies]` `clap`, `codecite-core`, `codecite-index`, `codecite-lsp`, `codecite-mcp`, each `.workspace = true`. Cargo infers the binary target from `src/main.rs`, so no `[[bin]]` section is needed.

- [ ] **Step 2: Write the failing tests in `crates/codecite/tests/cli.rs`**

Run the built binary through `std::process::Command::new(env!("CARGO_BIN_EXE_codecite"))`. Cargo sets that variable for integration tests, so no `assert_cmd` dependency is needed.

```rust
//! Command-line behaviour of the `codecite` binary.

use std::process::{Command, Output};

fn codecite(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .args(args)
        .output()
        .expect("failed to run codecite")
}

#[test]
fn version_flag_prints_package_version() {
    let out = codecite(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("codecite {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag_matches_long() {
    assert_eq!(codecite(&["-V"]).stdout, codecite(&["--version"]).stdout);
}

#[test]
fn no_args_prints_usage_and_fails() {
    let out = codecite(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage: codecite"));
}

#[test]
fn unknown_flag_is_rejected() {
    let out = codecite(&["--nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--nope"));
}
```

- [ ] **Step 3: Give `main.rs` a stub and confirm the tests fail**

The stub is `fn main() {}` with a `//!` doc line. Run: `cargo test -p codecite --test cli`
Expected: it compiles, `version_flag_prints_package_version`, `no_args_prints_usage_and_fails` and `unknown_flag_is_rejected` FAIL, and `short_version_flag_matches_long` passes trivially, since both outputs are empty.

- [ ] **Step 4: Implement `Cli` and `fn main()`**

`main` calls `Cli::parse()` and does nothing else yet. Bind the result to `_cli` so the struct isn't flagged as unused. The `about` text comes from the crate `description` via `#[command(about)]`.

Learn: `#[derive(Parser)]` generates the argument parser at compile time. `version` reads `CARGO_PKG_VERSION`, and `arg_required_else_help` turns a bare `codecite` into a usage message with exit code 2.

- [ ] **Step 5: Run the tests and the gates**

Run: `cargo test -p codecite --test cli`
Expected: 4 passed.

Run: `cargo build --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all succeed.

Run: `cargo tree -p codecite --depth 1`
Expected: `clap` and the four `codecite-*` crates, nothing else.

- [ ] **Step 6: Add "Build" to `README.md`**

Three lines: `rust-toolchain.toml` selects the compiler, `cargo build --workspace` builds it, and `cargo run -p codecite -- --version` runs it.

- [ ] **Step 7: Commit**

```bash
git add crates/codecite Cargo.lock README.md
git commit -m "feat: add the codecite binary with --version"
```
