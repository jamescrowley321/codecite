# Error Enums Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #7: each library crate gets a typed `Error` enum built with `thiserror` and a `Result<T>` alias. The `codecite` binary uses `anyhow` only in `main`, where it adds context to library errors.

**Architecture:** Each library crate has a private `error` module and re-exports `Error` and `Result` from its root. `codecite-core` owns the base variants (I/O, invalid corpus). `codecite-index`, `codecite-lsp` and `codecite-mcp` each wrap `codecite_core::Error` transparently and add their own I/O variant. The dependency direction stays the same: core depends on no other codecite crate. The binary's `main` returns `anyhow::Result<()>`.

**Tech Stack:** Rust 1.99.1 (edition 2024), `thiserror` 2, `anyhow` 1.

**Spec:** `docs/design.md#interfaces` ("Errors are explicit…"), `docs/design.md#milestones` (M1 Learn: error enums with `thiserror`), and issue #7.

## Global Constraints

- New dependencies: `thiserror = "2"` and `anyhow = "1"`, declared once under `[workspace.dependencies]`. Library crates take `thiserror`; only the `codecite` binary takes `anyhow`.
- `cargo deny check` must stay green. A probe build showed `thiserror-impl` 2.0.21 and `clap_derive` 4.6.7 both use `syn` 3, so `multiple-versions = "deny"` still holds. Both crates are `MIT OR Apache-2.0`.
- Every public error enum is `#[non_exhaustive]`.
- No `rusqlite` variant yet: M2 adds it when the store exists (issue #7 says so).
- A `Display` message is lower case, has no trailing period, and does not repeat its source's message. The source is reachable through `Error::source()`, so `anyhow`'s `{:#}` prints the chain once.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- `?` on a `std::io::Error` inside a function returning `codecite_core::Result<T>` compiles and gives `Error::Io`. Test: `io_error_converts_with_question_mark` in Task 1.
- The I/O variant's `Display` does not print the inner error, and `source()` returns it. Tests: `io_display_omits_source` and `io_source_is_the_io_error` in Task 1. Without this, `{:#}` prints the OS message twice.
- `InvalidCorpus` names the path and the reason. Test: `invalid_corpus_display_names_path_and_reason` in Task 1.
- A core error passed up through `codecite-index`, `codecite-lsp` or `codecite-mcp` prints exactly as it did in core (transparent). Tests: `core_error_converts_and_displays_unchanged` in each crate, Task 2.
- `anyhow` context on a library error prints the context, then the library message, then the I/O cause, each once. Test: `context_chain_prints_each_cause_once` in Task 3.
- A downstream `match` on `codecite_core::Error` without a wildcard arm fails to compile (`#[non_exhaustive]`). Verified by reading the attribute; a compile-fail test would need a new dev dependency for no other use.

---

### Task 1: `codecite-core` error enum

**Files:**
- Modify: `Cargo.toml` (add `thiserror` and `anyhow` to `[workspace.dependencies]`)
- Modify: `crates/codecite-core/Cargo.toml` (add `thiserror.workspace = true`)
- Create: `crates/codecite-core/src/error.rs`
- Modify: `crates/codecite-core/src/lib.rs` (`mod error; pub use error::{Error, Result};`)

**Interfaces:**
- Produces: `codecite_core::Error` with variants `Io(std::io::Error)` and `InvalidCorpus { path: PathBuf, reason: String }`, and `codecite_core::Result<T, E = Error>`. Task 2 consumes `codecite_core::Error`.

- [ ] **Step 1: Add the workspace dependencies**

Under `[workspace.dependencies]` in the root `Cargo.toml`, after `clap`: `thiserror = "2"` and `anyhow = "1"`. In `crates/codecite-core/Cargo.toml`, add a `[dependencies]` table with `thiserror.workspace = true`.

- [ ] **Step 2: Write the failing tests**

In `crates/codecite-core/src/error.rs`, a `#[cfg(test)] mod tests` with:

```rust
use super::*;
use std::error::Error as _;
use std::io;

fn read_missing() -> Result<()> {
    std::fs::read("/nonexistent/codecite-test")?;
    Ok(())
}

#[test]
fn io_error_converts_with_question_mark() {
    assert!(matches!(read_missing(), Err(Error::Io(_))));
}

#[test]
fn io_display_omits_source() {
    let err = Error::from(io::Error::other("disk on fire"));
    assert_eq!(err.to_string(), "I/O error");
}

#[test]
fn io_source_is_the_io_error() {
    let err = Error::from(io::Error::other("disk on fire"));
    assert_eq!(err.source().unwrap().to_string(), "disk on fire");
}

#[test]
fn invalid_corpus_display_names_path_and_reason() {
    let err = Error::InvalidCorpus {
        path: "/tmp/corpus".into(),
        reason: "not a codecite store".into(),
    };
    assert_eq!(err.to_string(), "invalid corpus at /tmp/corpus: not a codecite store");
    assert!(err.source().is_none());
}
```

Run: `cargo test -p codecite-core`
Expected: fails to compile, `Error` and `Result` are not defined.

- [ ] **Step 3: Write the enum**

Above the tests in `error.rs`:

```rust
//! Errors returned by the store and the query engine.

use std::path::PathBuf;

/// An error from `codecite-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Reading or writing a file failed.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// The path does not hold a usable corpus.
    #[error("invalid corpus at {}: {reason}", path.display())]
    InvalidCorpus {
        /// Where the corpus was expected.
        path: PathBuf,
        /// Why it can't be used.
        reason: String,
    },
}

/// `Result` with [`Error`] as the default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
```

In `lib.rs`, after the crate doc: `mod error;` and `pub use error::{Error, Result};`.

Learn: a library returns a typed error so callers can `match` on what went wrong; an application only reports errors, so `anyhow` (Task 3) is enough there. `thiserror` writes the `Display`, `std::error::Error` and `From` impls that you would otherwise write by hand. `#[from]` generates `impl From<io::Error> for Error`, and `?` calls `From::from` on the error, which is why `read_missing` compiles. `#[from]` also marks the field as the `source()`. `#[non_exhaustive]` makes callers outside the crate add a `_` arm, so adding a variant later isn't a breaking change. The `E = Error` default lets callers still write `Result<T, OtherError>`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite-core`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/codecite-core/Cargo.toml crates/codecite-core/src/lib.rs crates/codecite-core/src/error.rs
git commit -m "feat(core): add the Error enum and Result alias"
```

---

### Task 2: Error enums for `codecite-index`, `codecite-lsp` and `codecite-mcp`

**Files:**
- Modify: `crates/codecite-index/Cargo.toml`, `crates/codecite-lsp/Cargo.toml`, `crates/codecite-mcp/Cargo.toml` (add `thiserror.workspace = true`)
- Create: `crates/codecite-index/src/error.rs`, `crates/codecite-lsp/src/error.rs`, `crates/codecite-mcp/src/error.rs`
- Modify: the three `src/lib.rs` (`mod error; pub use error::{Error, Result};`)

**Interfaces:**
- Consumes: `codecite_core::Error` from Task 1.
- Produces: `codecite_index::Error`, `codecite_lsp::Error`, `codecite_mcp::Error`, each with `Core(codecite_core::Error)` and `Io(std::io::Error)`, plus a `Result<T, E = Error>` alias in each crate.

The three files are the same apart from the doc comments. Each crate's own I/O variant covers what that crate does itself: walking the corpus (index), talking to a language server process (lsp), and reading and writing stdio (mcp).

- [ ] **Step 1: Write the failing tests**

In each `error.rs`, a `#[cfg(test)] mod tests` with:

```rust
use super::*;
use std::error::Error as _;
use std::io;

fn open_corpus() -> Result<()> {
    Err(codecite_core::Error::InvalidCorpus {
        path: "/tmp/corpus".into(),
        reason: "not a codecite store".into(),
    })?;
    Ok(())
}

#[test]
fn core_error_converts_and_displays_unchanged() {
    let err = open_corpus().unwrap_err();
    assert!(matches!(err, Error::Core(_)));
    assert_eq!(err.to_string(), "invalid corpus at /tmp/corpus: not a codecite store");
}

#[test]
fn io_error_converts_and_keeps_source() {
    let err = Error::from(io::Error::other("pipe closed"));
    assert!(matches!(err, Error::Io(_)));
    assert_eq!(err.to_string(), "I/O error");
    assert_eq!(err.source().unwrap().to_string(), "pipe closed");
}
```

Run: `cargo test -p codecite-index -p codecite-lsp -p codecite-mcp`
Expected: fails to compile, `Error` and `Result` are not defined.

- [ ] **Step 2: Write the enums**

In each `error.rs`, above the tests (shown for `codecite-index`; change the crate name in the doc comments for the others):

```rust
//! Errors returned by the index pipeline.

/// An error from `codecite-index`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The store or query engine failed.
    #[error(transparent)]
    Core(#[from] codecite_core::Error),

    /// Reading or writing a file failed.
    #[error("I/O error")]
    Io(#[from] std::io::Error),
}

/// `Result` with [`Error`] as the default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
```

Then `mod error;` and `pub use error::{Error, Result};` in each `lib.rs`.

Learn: `#[error(transparent)]` forwards both `Display` and `source()` to the wrapped error, so the wrapper adds no line of its own to the chain. Two `#[from]` variants are fine as long as their source types differ; a second `io::Error` variant would give two conflicting `From` impls and fail to compile.

- [ ] **Step 3: Run the tests**

Run: `cargo test -p codecite-index -p codecite-lsp -p codecite-mcp`
Expected: 2 passed in each crate.

- [ ] **Step 4: Commit**

```bash
git add crates/codecite-index crates/codecite-lsp crates/codecite-mcp Cargo.lock
git commit -m "feat: add Error enums to the index, lsp and mcp crates"
```

---

### Task 3: `anyhow` in the binary

**Files:**
- Modify: `crates/codecite/Cargo.toml` (add `anyhow.workspace = true`)
- Modify: `crates/codecite/src/main.rs`

**Interfaces:**
- Consumes: `codecite_core::Error` from Task 1.

- [ ] **Step 1: Write the failing test**

At the bottom of `main.rs`:

```rust
#[cfg(test)]
mod tests {
    use anyhow::Context as _;

    fn open() -> anyhow::Result<()> {
        let err = codecite_core::Error::from(std::io::Error::other("disk on fire"));
        Err(err).context("opening corpus /tmp/corpus")
    }

    #[test]
    fn context_chain_prints_each_cause_once() {
        let err = open().unwrap_err();
        assert_eq!(format!("{err:#}"), "opening corpus /tmp/corpus: I/O error: disk on fire");
        assert!(err.downcast_ref::<codecite_core::Error>().is_some());
    }
}
```

Run: `cargo test -p codecite`
Expected: fails to compile, `anyhow` is not a dependency.

- [ ] **Step 2: Return `anyhow::Result` from `main`**

Add `anyhow.workspace = true` to `crates/codecite/Cargo.toml`. Change `main` to:

```rust
fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    Ok(())
}
```

No command calls a library yet, so `main` adds no context of its own. The first subcommand (M2) wraps each library call with `.context(...)` as the test does.

Learn: `anyhow::Error` holds any error type, and `.context(...)` (from the `Context` trait) puts a message in front of it without losing the original, which `downcast_ref` can still recover. When `main` returns `Err`, Rust prints it with `{:?}`, which for `anyhow` shows the message and an indented `Caused by:` list. `{:#}` prints the same chain on one line, joined by `: `. This is why library `Display` messages leave their source out.

- [ ] **Step 3: Run the tests and check the CLI still behaves**

Run: `cargo test -p codecite && cargo run -q -- --version`
Expected: the new test and the existing CLI tests pass; `--version` prints `codecite 0.0.0`.

- [ ] **Step 4: Run all gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`
Expected: all succeed. `cargo deny` reports no duplicate crates.

- [ ] **Step 5: Check docs for staleness**

`docs/design.md` and `AGENTS.md` say nothing that contradicts typed library errors and `anyhow` in the binary. If either now does, fix it in this commit.

- [ ] **Step 6: Commit**

```bash
git add crates/codecite/Cargo.toml crates/codecite/src/main.rs Cargo.lock
git commit -m "feat(cli): return anyhow::Result from main"
```
