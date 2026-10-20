# Local Source Walk Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #19: walk a `kind = "local"` source in `codecite-index`. List the checkout's files the way git sees them, apply the manifest's `include`/`exclude`, skip deny-listed, oversized and binary files, record HEAD and which files are `dirty`, and report every skipped path with its reason.

**Architecture:** A new module `crates/codecite-index/src/walk.rs` with a child `walk/git.rs` that runs `git` through `std::process::Command`. git is the source of truth for the file list: `git ls-files` lists tracked and untracked files minus ignored ones, a second `git ls-files` lists the ignored entries so they can be counted, and `git status` says which files differ from HEAD. The walk then checks each listed path in a fixed order and either keeps it (`LocalFile`) or records why not (`SkippedFile`). Errors are their own `WalkError` enum, wrapped by the crate's `Error`.

**Tech Stack:** Rust 1.99.1 (edition 2024), `globset` 0.4 (manifest globs), `thiserror` 2, the `git` CLI (2.25 or newer); `tempfile` 3 in tests.

**Spec:** Issue #19, `docs/design.md#indexing-and-retrieval` (step 2, Walk), `docs/design.md#corpus-manifests-and-licensing` (the `kind = "local"` paragraph: results cite HEAD and carry `dirty: true`) and `docs/design.md#security-and-privacy` (the deny-list). Choices the issue leaves open are recorded in the loop's decision journal (DEC-007) and summarised under Global Constraints.

## Global Constraints

- **git, not the `ignore` crate, decides what is ignored.** The issue suggests the `ignore` crate. A prototype showed it skips a *tracked* file that matches `.gitignore` (git never ignores a tracked file), and nothing would report that skip, which breaks "never silently dropped". A local source must be a git checkout anyway, and the design already shells out to `git` for git sources, so the walk asks git:
  - files: `git ls-files -z --cached --others --exclude-standard` (tracked plus untracked-but-not-ignored; `.gitignore` files at every level, `.git/info/exclude` and the global excludes file all apply). Output is not sorted and repeats paths with merge conflicts, so sort and dedup.
  - ignored: `git ls-files -z --others --ignored --exclude-standard --directory --no-empty-directory`. A wholly ignored directory comes back as one entry (`target/`), so `node_modules/` counts once instead of once per file.
  - dirty: `git status --porcelain=v1 -z --untracked-files=all --no-renames`. Each record is `XY <path>`; every listed path differs from HEAD, untracked ones included (HEAD holds no such file, so a citation at HEAD can't reproduce it).
- **Every git call** runs as `git -C <root> …` with `GIT_OPTIONAL_LOCKS=0` (read-only: `status` must not rewrite the index) and with `GIT_DIR`, `GIT_WORK_TREE` and `GIT_INDEX_FILE` removed from its environment, so an inherited value can't point git at another repository.
- **The checkout.** `path` is canonicalized first (`fs::canonicalize`; a missing path is `WalkError::Io` naming it). `git rev-parse --show-toplevel` failing → `NotAGitRepo`. A toplevel that canonicalizes to a different directory → `NotRepoRoot` (strict: paths in citations are relative to the repository root, so a subdirectory source would need a prefix the manifest doesn't have). `git rev-parse --verify --quiet HEAD^{commit}` failing → `NoHead` (an empty repository has nothing to cite). HEAD is kept as git prints it (40 hex digits, or 64 in a SHA-256 repository).
- **Order of checks for each listed path** (the first that applies is the reason):
  1. not UTF-8 → `NonUtf8Path` (citations are strings; the lossy spelling is reported)
  2. not allowed by `include`/`exclude` → `Excluded`
  3. file name on the deny-list → `DenyListed` (checked before any read, so a secret's bytes are never opened)
  4. gone from disk (a tracked file deleted in the work tree) → `Deleted`
  5. a symlink → `Symlink` (never followed: a link could point outside the checkout, at `~/.ssh` for example)
  6. not a regular file (a submodule shows up as a directory) → `NotAFile`
  7. larger than `max_file_size` → `Oversized`
  8. a NUL byte in the first 8000 bytes → `Binary`
  Ignored entries from git are added as `Ignored`. Nothing else is dropped.
- **Deny-list** (matched on the file name, ASCII case-insensitive):
  - starts with `.env` (`.env`, `.env.local`, `.envrc`, `.env.example`);
  - ends with `.pem`, `.key`, `.p12`, `.pfx`, `.crt`, `.cer`, `.der`, `.jks` or `.keystore`;
  - an SSH key name: `id_` followed by a name with no `.`, optionally ending `.pub` (`id_rsa`, `id_ed25519_sk.pub`).
  The issue writes `id_*`, but read literally that skips `id_token.rs`, `id_token.py` and `id_token_logic.py`, core files of identity-model, the first real corpus. The issue's intent is "private keys", so the rule targets SSH key names.
- **Binary sniffing** is git's rule: a NUL byte in the first 8000 bytes. Extensions are never consulted. UTF-8 decoding is left to the chunker.
- **Size cap:** `WalkOptions::max_file_size`, default `DEFAULT_MAX_FILE_SIZE = 1024 * 1024`. A file of exactly the cap is kept; one byte more is `Oversized`. The manifest gains no key for it (the issue doesn't ask for one).
- **Globs:** `include`/`exclude` are `globset` globs matched against the path relative to the source root, with `/` separators and `literal_separator(true)`: `*` stays within one directory, `**` crosses directories (`lib/**`, `lib/vendor/**`). An empty `include` means everything. A bad glob is `WalkError::Glob` naming the pattern.
- **Results are sorted by path.** `LocalFile` holds the relative path, the size and `dirty`; it does not hold the bytes (the chunker reads each file once, and holding a whole repository in memory is not needed for the walk).
- `thiserror` treats a field named `source` as the error's cause. Here that is wanted (`io::Error`, `globset::Error`); labels are never named `source`.
- Tests build throwaway repositories with `tempfile` and the `git` CLI, passing `-c user.name=… -c user.email=… -c commit.gpgsign=false` so they don't depend on the machine's git config. Files the tests expect to be deny-listed, binary or oversized are committed, so a developer's global excludes can't turn them into `Ignored`.
- The workspace forbids `unsafe_code` and warns on `missing_docs`. Every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- An ignored directory, a binary, an oversized file and deny-listed files are skipped and each is counted; a modified tracked file and an untracked file are `dirty`, a clean one is not. Test: `walk_skips_and_counts` (Task 3).
- A tracked file matching `.gitignore` is walked, not dropped. Tests: `files_include_tracked_files_that_match_gitignore` (Task 1), `walk_skips_and_counts` (Task 3).
- `id_token.rs` is walked while `id_ed25519` is deny-listed. Tests: `deny_list` (Task 2), `walk_skips_and_counts` (Task 3).
- A non-repository, a subdirectory of a checkout and a repository with no commits are errors. Tests: `head_errors` (Task 1).
- A symlink is never followed. Test: `walk_skips_and_counts` (Task 3, unix only).

---

### Task 1: Errors, dependencies and the git commands

**Files:**
- Modify: `Cargo.toml` (workspace dependencies `globset`, `tempfile`)
- Modify: `crates/codecite-index/Cargo.toml`
- Create: `crates/codecite-index/src/walk.rs` (`WalkError` and the module skeleton)
- Create: `crates/codecite-index/src/walk/git.rs`
- Create: `crates/codecite-index/src/walk/test_repo.rs` (test-only helper)
- Modify: `crates/codecite-index/src/lib.rs` (module, re-export, crate doc)
- Modify: `crates/codecite-index/src/error.rs` (`Walk` variant)

**Interfaces:**
- Produces `pub enum WalkError` (`#[non_exhaustive]`), re-exported from `codecite_index`, and `Error::Walk(#[from] WalkError)` with `#[error(transparent)]`.
- Produces, `pub(super)` in `walk::git`: `run`, `head`, `files`, `ignored`, `dirty`, `split_paths`.
- Produces, test-only: `walk::test_repo::Repo`.

- [ ] **Step 1: Add the dependencies**

`Cargo.toml`, under `[workspace.dependencies]`:

```toml
globset = "0.4"
tempfile = "3"
```

`crates/codecite-index/Cargo.toml`: `globset.workspace = true` under `[dependencies]`, and a new section:

```toml
[dev-dependencies]
tempfile.workspace = true
```

Run: `cargo deny check`
Expected: green. Prototyped on this branch: `globset` 0.4.20 and `tempfile` 3.27.0 add no duplicate versions, and every license is already allowed.

- [ ] **Step 2: The test helper**

`crates/codecite-index/src/walk/test_repo.rs`:

```rust
//! A throwaway git repository for the walk's tests.

use std::fs;
use std::path::Path;
use std::process::Command;

/// A git repository in a temporary directory, deleted on drop.
pub(crate) struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    /// `git init` in a new temporary directory.
    pub(crate) fn new() -> Self {
        let repo = Repo {
            dir: tempfile::tempdir().unwrap(),
        };
        repo.git(&["init", "-q"]);
        repo
    }

    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Runs git in the repository with a fixed identity; panics on failure.
    pub(crate) fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(["-c", "user.name=codecite", "-c", "user.email=codecite@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Writes `contents` to `path`, creating parent directories.
    pub(crate) fn write(&self, path: &str, contents: impl AsRef<[u8]>) {
        let full = self.path().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, contents).unwrap();
    }

    /// Stages everything (`-f`, so global excludes can't interfere) and commits.
    pub(crate) fn commit_all(&self, paths: &[&str]) {
        let mut args = vec!["add", "-f", "--"];
        args.extend_from_slice(paths);
        self.git(&args);
        self.git(&["commit", "-q", "-m", "test"]);
    }
}
```

Learn — `tempfile::TempDir` deletes its directory when dropped, so each test cleans up after itself even when an assert panics. `impl AsRef<[u8]>` lets `write` take a `&str`, a `String` or a `Vec<u8>`.

- [ ] **Step 3: Write the failing tests**

`crates/codecite-index/src/walk.rs` (skeleton; the walk itself comes in Task 3):

```rust
//! Walking a local source: list the checkout's files the way git sees them,
//! skip what must not be indexed, and report every skip with its reason.

// The helpers have no caller outside tests until `walk_local` lands.
#![allow(dead_code)]

mod git;
#[cfg(test)]
mod test_repo;
```

and `mod walk;` plus `pub use walk::WalkError;` in `lib.rs`. In `walk/git.rs`, only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::test_repo::Repo;

    fn strings(paths: Vec<Result<String, String>>) -> Vec<String> {
        paths.into_iter().map(Result::unwrap).collect()
    }

    #[test]
    fn split_paths_reports_non_utf8_paths_lossily() {
        let out = b"a.rs\0dir/b \xff.rs\0\0";
        let paths: Vec<_> = split_paths(out).collect();
        assert_eq!(
            paths,
            [Ok("a.rs".to_owned()), Err("dir/b \u{fffd}.rs".to_owned())]
        );
    }

    #[test]
    fn head_is_the_checked_out_commit() {
        let repo = Repo::new();
        repo.write("a.rs", "fn a() {}\n");
        repo.commit_all(&["a.rs"]);
        let head = head(repo.path()).unwrap();
        assert!(
            head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()),
            "{head}"
        );
        let expected = run(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(head, String::from_utf8(expected).unwrap().trim_end());
    }

    #[test]
    fn head_errors() {
        let plain = tempfile::tempdir().unwrap();
        let err = head(plain.path()).unwrap_err();
        assert!(matches!(err, WalkError::NotAGitRepo { .. }), "{err:?}");

        let repo = Repo::new();
        let err = head(repo.path()).unwrap_err();
        assert!(matches!(err, WalkError::NoHead { .. }), "{err:?}");

        repo.write("src/a.rs", "a\n");
        repo.commit_all(&["src/a.rs"]);
        let err = head(&repo.path().join("src")).unwrap_err();
        assert!(matches!(err, WalkError::NotRepoRoot { .. }), "{err:?}");
        assert!(err.to_string().contains("not the top of its git work tree"), "{err}");
    }

    #[test]
    fn files_include_tracked_files_that_match_gitignore() {
        let repo = Repo::new();
        repo.write(".gitignore", "target/\n*.log\n");
        repo.write("keep.log", "tracked on purpose\n");
        repo.write("a.rs", "a\n");
        repo.commit_all(&[".gitignore", "keep.log", "a.rs"]);
        repo.write("new.rs", "untracked\n");
        repo.write("debug.log", "ignored\n");
        repo.write("target/debug/app", "ignored\n");

        assert_eq!(
            strings(files(repo.path()).unwrap()),
            [".gitignore", "a.rs", "keep.log", "new.rs"]
        );
        assert_eq!(
            strings(ignored(repo.path()).unwrap()),
            ["debug.log", "target/"]
        );
    }

    #[test]
    fn dirty_lists_paths_that_differ_from_head() {
        let repo = Repo::new();
        repo.write("clean.rs", "c\n");
        repo.write("modified.rs", "m\n");
        repo.write("staged.rs", "s\n");
        repo.commit_all(&["clean.rs", "modified.rs", "staged.rs"]);
        repo.write("modified.rs", "m2\n");
        repo.write("staged.rs", "s2\n");
        repo.git(&["add", "staged.rs"]);
        repo.write("src/untracked.rs", "u\n");

        let mut dirty: Vec<_> = dirty(repo.path()).unwrap().into_iter().collect();
        dirty.sort();
        assert_eq!(dirty, ["modified.rs", "src/untracked.rs", "staged.rs"]);
    }
}
```

Run: `cargo test -p codecite-index walk::git`
Expected: FAIL to compile (`split_paths`, `head`, `files`, `ignored`, `dirty`, `run`, `WalkError` don't exist).

- [ ] **Step 4: `WalkError`**

In `walk.rs`, after the module declarations:

```rust
use std::io;
use std::path::PathBuf;

/// Why a local source could not be walked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WalkError {
    /// The source's `path` is not inside a git work tree.
    #[error("{path}: not a git work tree; a local source must be a git checkout")]
    NotAGitRepo {
        /// The source's path.
        path: PathBuf,
    },

    /// The source's `path` is inside a git work tree but not at its top.
    #[error("{path}: not the top of its git work tree ({toplevel}); point `path` at the checkout's root")]
    NotRepoRoot {
        /// The source's path.
        path: PathBuf,
        /// The work tree's top level.
        toplevel: PathBuf,
    },

    /// The repository has no commits, so there is no HEAD to cite.
    #[error("{path}: the repository has no commits, so there is no HEAD to cite")]
    NoHead {
        /// The source's path.
        path: PathBuf,
    },

    /// `git` could not be started (not installed, or not on `PATH`).
    #[error("could not run git")]
    GitSpawn {
        /// Why the process did not start.
        source: io::Error,
    },

    /// A `git` command exited with an error.
    #[error("`{command}` failed: {stderr}")]
    Git {
        /// The command line, without `-C <root>`.
        command: String,
        /// What git printed on stderr, trimmed.
        stderr: String,
    },

    /// An `include` or `exclude` glob is invalid.
    #[error("invalid glob {pattern:?}")]
    Glob {
        /// The glob as written.
        pattern: String,
        /// What `globset` rejected.
        source: globset::Error,
    },

    /// Reading a path in the checkout failed.
    #[error("{path}: I/O error")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
}
```

In `error.rs`, a new variant:

```rust
    /// A local source could not be walked.
    #[error(transparent)]
    Walk(#[from] crate::WalkError),
```

Learn — `thiserror` displays a `PathBuf` field in `#[error("{path}")]` through `Path::display`, because `PathBuf` has no `Display` of its own. If the derive rejects it, write `{}` with `path.display()` as an extra format argument.

- [ ] **Step 5: The git commands**

`walk/git.rs`, above the tests:

```rust
//! The `git` commands the walk runs against a local checkout.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::WalkError;

/// Runs `git -C <root> <args>` and returns its stdout.
pub(super) fn run(root: &Path, args: &[&str]) -> Result<Vec<u8>, WalkError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        // Read-only: don't let `status` refresh and rewrite the index.
        .env("GIT_OPTIONAL_LOCKS", "0")
        // `-C` picks the repository; an inherited GIT_DIR would override it.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map_err(|source| WalkError::GitSpawn { source })?;
    if !output.status.success() {
        return Err(WalkError::Git {
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}
```

Learn — `std::process::Command` is a builder: `arg`/`args`/`env`/`env_remove` each return `&mut Command`, and `output()` runs the process to completion and captures stdout, stderr and the exit status. A non-zero exit is not an `Err` from `output()`; check `status.success()` yourself.

```rust
/// Checks that `root` is the top of a git work tree with at least one commit
/// and returns HEAD's commit.
pub(super) fn head(root: &Path) -> Result<String, WalkError> {
    let root = canonical(root)?;
    let toplevel = match run(&root, &["rev-parse", "--show-toplevel"]) {
        Ok(out) => PathBuf::from(String::from_utf8_lossy(&out).trim_end()),
        Err(WalkError::Git { .. }) => return Err(WalkError::NotAGitRepo { path: root }),
        Err(e) => return Err(e),
    };
    if canonical(&toplevel)? != root {
        return Err(WalkError::NotRepoRoot { path: root, toplevel });
    }
    match run(&root, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]) {
        Ok(out) => Ok(String::from_utf8_lossy(&out).trim_end().to_owned()),
        Err(WalkError::Git { .. }) => Err(WalkError::NoHead { path: root }),
        Err(e) => Err(e),
    }
}

/// `path` with symlinks resolved, so `/var/…` and `/private/var/…` compare equal.
pub(super) fn canonical(path: &Path) -> Result<PathBuf, WalkError> {
    fs::canonicalize(path).map_err(|source| WalkError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Tracked files plus untracked files git doesn't ignore, sorted, each once.
pub(super) fn files(root: &Path) -> Result<Vec<Result<String, String>>, WalkError> {
    let out = run(root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?;
    let mut paths: Vec<_> = split_paths(&out).collect();
    paths.sort();
    // A file with a merge conflict is listed once per stage.
    paths.dedup();
    Ok(paths)
}

/// Untracked paths git ignores, sorted. A wholly ignored directory is one
/// entry ending in `/`.
pub(super) fn ignored(root: &Path) -> Result<Vec<Result<String, String>>, WalkError> {
    let out = run(
        root,
        &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory", "--no-empty-directory"],
    )?;
    let mut paths: Vec<_> = split_paths(&out).collect();
    paths.sort();
    Ok(paths)
}

/// Paths that differ from HEAD: modified, staged, deleted or untracked.
pub(super) fn dirty(root: &Path) -> Result<HashSet<String>, WalkError> {
    let out = run(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--no-renames"])?;
    // Each record is `XY <path>`; a non-UTF-8 path is skipped by the walk anyway.
    Ok(split_paths(&out)
        .filter_map(Result::ok)
        .filter_map(|record| record.get(3..).map(str::to_owned))
        .collect())
}

/// Splits NUL-separated `git -z` output into paths. A path that isn't UTF-8
/// is an `Err` holding its lossy spelling, for the skip report.
pub(super) fn split_paths(out: &[u8]) -> impl Iterator<Item = Result<String, String>> + '_ {
    out.split(|&b| b == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
        })
}
```

Learn — an iterator over `Result`: `split_paths` yields `Result<String, String>` per path instead of failing the whole listing on one bad name. `filter_map(Result::ok)` drops the `Err`s where they don't matter (`dirty`); `files` keeps them so the walk can report each one. `Result<T, E>` is `Ord` when `T` and `E` are (`Ok` sorts before `Err`), so `sort` and `dedup` work on the `Vec` directly. `impl Iterator<…> + '_` says the iterator borrows from `out`.

Run: `cargo test -p codecite-index walk::git`
Expected: all pass. Then the gates. The module-level `#![allow(dead_code)]` keeps `clippy -D warnings` green while the helpers are only called from tests; Task 3 deletes it. (A lint attribute at the top of a module covers its child modules too.)

- [ ] **Step 6: Commit**

`lib.rs` crate doc gains one sentence: "It also walks local checkouts, listing the files git sees and reporting every file it skips." Commit `Cargo.toml`, `Cargo.lock`, `crates/codecite-index/Cargo.toml`, `crates/codecite-index/src/{lib.rs,error.rs,walk.rs,walk/git.rs,walk/test_repo.rs}`:

```
feat(index): read HEAD, files and dirty paths from a local checkout (#19)
```

---

### Task 2: Deny-list, binary sniffing and path globs

**Files:**
- Modify: `crates/codecite-index/src/walk.rs`

**Interfaces:**
- Produces, private in `walk`: `fn is_deny_listed(name: &str) -> bool`, `fn looks_binary(prefix: &[u8]) -> bool`, `const SNIFF_LEN: usize = 8000`, `struct PathFilter` with `fn new(include: &[String], exclude: &[String]) -> Result<PathFilter, WalkError>` and `fn allows(&self, path: &str) -> bool`.

- [ ] **Step 1: Write the failing tests**

A `#[cfg(test)] mod tests` in `walk.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_list() {
        for name in [
            ".env", ".env.local", ".envrc", ".env.example", "server.pem", "SERVER.PEM",
            "tls.key", "bundle.p12", "store.pfx", "ca.crt", "ca.cer", "key.der", "app.jks",
            "release.keystore", "id_rsa", "id_ed25519", "id_ecdsa_sk.pub",
        ] {
            assert!(is_deny_listed(name), "{name} should be deny-listed");
        }
        for name in [
            "id_token.rs", "id_token_logic.py", "id_", "keys.rs", "pem.rs", "environment.ts",
            "my.env", "monkey.txt", "README.md",
        ] {
            assert!(!is_deny_listed(name), "{name} should not be deny-listed");
        }
    }

    #[test]
    fn binary_means_a_nul_byte_in_the_prefix() {
        assert!(!looks_binary(b""));
        assert!(!looks_binary("fn main() {}\n// ünïcödé\n".as_bytes()));
        assert!(looks_binary(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        assert!(looks_binary(b"text then \0"));
    }

    fn filter(include: &[&str], exclude: &[&str]) -> PathFilter {
        let owned = |globs: &[&str]| globs.iter().map(|g| g.to_string()).collect::<Vec<_>>();
        PathFilter::new(&owned(include), &owned(exclude)).unwrap()
    }

    #[test]
    fn path_filter() {
        let all = filter(&[], &[]);
        assert!(all.allows("a.rs") && all.allows("deep/er/b.rs"));

        let lib = filter(&["lib/**", "*.md"], &["lib/vendor/**"]);
        assert!(lib.allows("lib/a.rs"));
        assert!(lib.allows("lib/deep/b.rs"));
        assert!(lib.allows("README.md"));
        // `*` stays within one directory.
        assert!(!lib.allows("docs/guide.md"));
        assert!(!lib.allows("lib/vendor/x.rs"));
        assert!(!lib.allows("src/main.rs"));
    }

    #[test]
    fn bad_glob_names_the_pattern() {
        let err = PathFilter::new(&["src/[".to_owned()], &[]).unwrap_err();
        assert!(matches!(err, WalkError::Glob { ref pattern, .. } if pattern == "src/["), "{err:?}");
        assert_eq!(err.to_string(), "invalid glob \"src/[\"");
    }
}
```

Run: `cargo test -p codecite-index walk::tests`
Expected: FAIL to compile (`is_deny_listed`, `looks_binary`, `PathFilter` don't exist).

- [ ] **Step 2: Implement**

```rust
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// How many leading bytes the binary sniff reads: git's own limit.
const SNIFF_LEN: usize = 8000;

/// File-name endings of private keys and certificates.
const DENIED_EXTENSIONS: [&str; 9] =
    [".pem", ".key", ".p12", ".pfx", ".crt", ".cer", ".der", ".jks", ".keystore"];

/// Whether a file name is on the secrets deny-list: env files, private keys,
/// certificates and SSH keys. Matching ignores ASCII case.
fn is_deny_listed(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with(".env")
        || DENIED_EXTENSIONS.iter().any(|ext| name.ends_with(ext))
        || is_ssh_key_name(&name)
}

/// `id_rsa`, `id_ed25519`, `id_ecdsa_sk.pub`: `id_` then a name without a
/// dot, with an optional `.pub`. `id_token.rs` has a dot, so it is code.
fn is_ssh_key_name(name: &str) -> bool {
    let stem = name.strip_suffix(".pub").unwrap_or(name);
    stem.strip_prefix("id_")
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('.'))
}

/// Whether a file's leading bytes look binary: a NUL byte, git's rule.
fn looks_binary(prefix: &[u8]) -> bool {
    prefix.contains(&0)
}

/// The manifest's `include` and `exclude` globs.
struct PathFilter {
    /// `None` when `include` is empty: everything is included.
    include: Option<GlobSet>,
    exclude: GlobSet,
}

impl PathFilter {
    fn new(include: &[String], exclude: &[String]) -> Result<Self, WalkError> {
        let include = if include.is_empty() {
            None
        } else {
            Some(glob_set(include)?)
        };
        Ok(PathFilter {
            include,
            exclude: glob_set(exclude)?,
        })
    }

    /// Whether `path` (relative, `/`-separated) is included and not excluded.
    fn allows(&self, path: &str) -> bool {
        self.include.as_ref().is_none_or(|set| set.is_match(path)) && !self.exclude.is_match(path)
    }
}

/// Compiles globs where `*` stays within one directory and `**` crosses them.
fn glob_set(patterns: &[String]) -> Result<GlobSet, WalkError> {
    let glob_error = |pattern: &str| {
        let pattern = pattern.to_owned();
        move |source| WalkError::Glob { pattern, source }
    };
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(glob_error(pattern))?;
        set.add(glob);
    }
    set.build().map_err(glob_error(&patterns.join(", ")))
}
```

Learn — `Option::is_none_or` (Rust 1.82) reads as "no include list, or the list matches", replacing a `match` on the `Option`. `is_some_and` is its mirror. `glob_error` is a closure that returns a closure: `map_err` needs an `FnOnce(globset::Error) -> WalkError`, and the outer closure captures which pattern failed.

Run: `cargo test -p codecite-index walk::tests`
Expected: all pass. If the `"src/["` case compiles fine in `globset` (it reports an unclosed class), keep the test; if it doesn't fail, switch it to `"a{b"` (an unclosed alternate).

- [ ] **Step 3: Gates and commit**

Commit `crates/codecite-index/src/walk.rs`:

```
feat(index): deny-list secrets, sniff binaries and match manifest globs (#19)
```

---

### Task 3: `walk_local` and the design doc

**Files:**
- Modify: `crates/codecite-index/src/walk.rs`
- Modify: `crates/codecite-index/src/lib.rs` (re-exports)
- Modify: `docs/design.md`

**Interfaces:**
- Produces, re-exported from `codecite_index`:
  - `pub fn walk_local(source: &LocalSource, options: &WalkOptions) -> Result<LocalTree, WalkError>`
  - `pub struct WalkOptions { pub max_file_size: u64 }` with `Default` (`DEFAULT_MAX_FILE_SIZE`), and `pub const DEFAULT_MAX_FILE_SIZE: u64 = 1024 * 1024`.
  - `pub struct LocalTree { pub root: PathBuf, pub head: String, pub files: Vec<LocalFile>, pub skipped: Vec<SkippedFile> }` with `fn skip_counts(&self) -> BTreeMap<SkipReason, usize>`.
  - `pub struct LocalFile { pub path: String, pub size: u64, pub dirty: bool }`.
  - `pub struct SkippedFile { pub path: String, pub reason: SkipReason }`.
  - `pub enum SkipReason { Ignored, Excluded, DenyListed, Deleted, Symlink, NotAFile, Oversized, Binary, NonUtf8Path }`, `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]`, `#[non_exhaustive]`.
  - All derive `Debug, Clone, PartialEq, Eq` (the structs).

- [ ] **Step 1: Write the failing tests**

Add to `walk.rs`'s test module:

```rust
    use crate::walk::test_repo::Repo;
    use crate::{LicenseClass, LocalSource};
    use std::collections::BTreeMap;

    fn local(repo: &Repo) -> LocalSource {
        LocalSource {
            name: None,
            path: repo.path().to_owned(),
            license: None,
            license_class: LicenseClass::ApproachOnly,
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }

    fn paths(tree: &LocalTree) -> Vec<(&str, bool)> {
        tree.files.iter().map(|f| (f.path.as_str(), f.dirty)).collect()
    }

    #[test]
    fn walk_skips_and_counts() {
        let repo = Repo::new();
        repo.write(".gitignore", "target/\n");
        repo.write("src/lib.rs", "pub fn lib() {}\n");
        repo.write("src/main.rs", "fn main() {}\n");
        repo.write("src/id_token.rs", "pub struct IdToken;\n");
        // Text without an extension is indexed; extensions are never trusted.
        repo.write("scripts/build", "#!/bin/sh\necho build\n");
        // Binary by content, whatever the name says.
        repo.write("assets/logo.png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
        repo.write("data/notes.txt", b"looks like text \0 but is not");
        repo.write("big.txt", vec![b'a'; DEFAULT_MAX_FILE_SIZE as usize + 1]);
        repo.write("exact.txt", vec![b'a'; DEFAULT_MAX_FILE_SIZE as usize]);
        repo.write(".env", "SECRET=1\n");
        repo.write("certs/server.pem", "-----BEGIN PRIVATE KEY-----\n");
        repo.write("id_ed25519", "-----BEGIN OPENSSH PRIVATE KEY-----\n");
        repo.commit_all(&["."]);

        repo.write("src/main.rs", "fn main() { changed() }\n");
        repo.write("src/new.rs", "pub fn new() {}\n");
        repo.write("target/debug/app", "build output\n");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hostname", repo.path().join("link")).unwrap();

        let tree = walk_local(&local(&repo), &WalkOptions::default()).unwrap();

        assert_eq!(tree.head.len(), 40, "{}", tree.head);
        assert_eq!(
            paths(&tree),
            [
                (".gitignore", false),
                ("exact.txt", false),
                ("scripts/build", false),
                ("src/id_token.rs", false),
                ("src/lib.rs", false),
                ("src/main.rs", true),
                ("src/new.rs", true),
            ]
        );
        assert_eq!(tree.files[1].size, DEFAULT_MAX_FILE_SIZE);

        let mut expected = BTreeMap::from([
            (SkipReason::Ignored, 1),
            (SkipReason::DenyListed, 3),
            (SkipReason::Oversized, 1),
            (SkipReason::Binary, 2),
        ]);
        if cfg!(unix) {
            expected.insert(SkipReason::Symlink, 1);
        }
        assert_eq!(tree.skip_counts(), expected);

        let reason = |path: &str| tree.skipped.iter().find(|s| s.path == path).map(|s| s.reason);
        assert_eq!(reason("target/"), Some(SkipReason::Ignored));
        assert_eq!(reason("id_ed25519"), Some(SkipReason::DenyListed));
        assert_eq!(reason("data/notes.txt"), Some(SkipReason::Binary));
        assert_eq!(reason("big.txt"), Some(SkipReason::Oversized));
    }

    #[test]
    fn walk_applies_include_and_exclude() {
        let repo = Repo::new();
        repo.write("src/a.rs", "a\n");
        repo.write("src/gen/b.rs", "b\n");
        repo.write("docs/c.md", "c\n");
        repo.commit_all(&["."]);

        let mut source = local(&repo);
        source.include = vec!["src/**".into()];
        source.exclude = vec!["src/gen/**".into()];
        let tree = walk_local(&source, &WalkOptions::default()).unwrap();
        assert_eq!(paths(&tree), [("src/a.rs", false)]);
        assert_eq!(tree.skip_counts(), BTreeMap::from([(SkipReason::Excluded, 2)]));
    }

    #[test]
    fn walk_reports_deleted_files_and_honors_the_size_option() {
        let repo = Repo::new();
        repo.write("gone.rs", "g\n");
        repo.write("small.rs", "s\n");
        repo.write("medium.rs", "0123456789\n");
        repo.commit_all(&["."]);
        std::fs::remove_file(repo.path().join("gone.rs")).unwrap();

        let tree = walk_local(&local(&repo), &WalkOptions { max_file_size: 4 }).unwrap();
        assert_eq!(paths(&tree), [("small.rs", false)]);
        assert_eq!(
            tree.skipped,
            [
                SkippedFile { path: "gone.rs".into(), reason: SkipReason::Deleted },
                SkippedFile { path: "medium.rs".into(), reason: SkipReason::Oversized },
            ]
        );
    }

    #[test]
    fn walk_errors_name_the_problem() {
        let plain = tempfile::tempdir().unwrap();
        let mut source = local(&Repo::new());
        source.path = plain.path().to_owned();
        let err = walk_local(&source, &WalkOptions::default()).unwrap_err();
        assert!(matches!(err, WalkError::NotAGitRepo { .. }), "{err:?}");

        source.path = plain.path().join("missing");
        let err = walk_local(&source, &WalkOptions::default()).unwrap_err();
        assert!(matches!(err, WalkError::Io { .. }), "{err:?}");

        let err: crate::Error = err.into();
        assert!(matches!(err, crate::Error::Walk(_)));
    }
```

Run: `cargo test -p codecite-index walk`
Expected: FAIL to compile (`walk_local`, `WalkOptions`, `LocalTree`, … don't exist).

- [ ] **Step 2: Implement**

In `walk.rs`:

```rust
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::LocalSource;

/// The default size cap: files larger than 1 MiB are skipped.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 1024 * 1024;

/// Knobs for [`walk_local`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkOptions {
    /// Files larger than this many bytes are skipped as [`SkipReason::Oversized`].
    pub max_file_size: u64,
}

impl Default for WalkOptions {
    fn default() -> Self {
        WalkOptions {
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        }
    }
}

/// A walked local checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTree {
    /// The checkout's root, canonicalized.
    pub root: PathBuf,
    /// HEAD's commit, which results from this checkout cite.
    pub head: String,
    /// The files to index, sorted by path.
    pub files: Vec<LocalFile>,
    /// Everything not indexed, with the reason: listed paths in path order,
    /// then ignored entries.
    pub skipped: Vec<SkippedFile>,
}

/// A file to index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    /// The path from the checkout's root, `/`-separated.
    pub path: String,
    /// The size in bytes when walked.
    pub size: u64,
    /// Whether the file differs from HEAD (modified, staged or untracked).
    pub dirty: bool,
}

/// A path the walk did not index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    /// The path from the checkout's root; an ignored directory ends in `/`.
    pub path: String,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// Why the walk skipped a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SkipReason {
    /// git ignores it (`.gitignore`, `.git/info/exclude`, global excludes).
    Ignored,
    /// The manifest's `include`/`exclude` leaves it out.
    Excluded,
    /// Its name is on the secrets deny-list; it was never opened.
    DenyListed,
    /// Tracked, but deleted from the work tree.
    Deleted,
    /// A symbolic link; links are never followed.
    Symlink,
    /// Not a regular file (a submodule, for example).
    NotAFile,
    /// Larger than [`WalkOptions::max_file_size`].
    Oversized,
    /// Binary content: a NUL byte near the start.
    Binary,
    /// The path is not valid UTF-8, so no citation can name it.
    NonUtf8Path,
}

impl LocalTree {
    /// How many paths were skipped for each reason.
    pub fn skip_counts(&self) -> BTreeMap<SkipReason, usize> {
        let mut counts = BTreeMap::new();
        for skipped in &self.skipped {
            *counts.entry(skipped.reason).or_default() += 1;
        }
        counts
    }
}

/// Walks a local source: lists the files git sees, applies the manifest's
/// globs, skips deny-listed, oversized and binary files, and marks each kept
/// file `dirty` when it differs from HEAD.
pub fn walk_local(source: &LocalSource, options: &WalkOptions) -> Result<LocalTree, WalkError> {
    let filter = PathFilter::new(&source.include, &source.exclude)?;
    let root = git::canonical(&source.path)?;
    let head = git::head(&root)?;
    let dirty = git::dirty(&root)?;
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for path in git::files(&root)? {
        let path = match path {
            Ok(path) => path,
            Err(lossy) => {
                skipped.push(SkippedFile { path: lossy, reason: SkipReason::NonUtf8Path });
                continue;
            }
        };
        match check(&root, &path, &filter, options)? {
            Check::Keep { size } => {
                let dirty = dirty.contains(&path);
                files.push(LocalFile { path, size, dirty });
            }
            Check::Skip(reason) => skipped.push(SkippedFile { path, reason }),
        }
    }
    for path in git::ignored(&root)? {
        let path = path.unwrap_or_else(|lossy| lossy);
        skipped.push(SkippedFile { path, reason: SkipReason::Ignored });
    }
    Ok(LocalTree { root, head, files, skipped })
}

/// What [`check`] decided for one path.
enum Check {
    Keep { size: u64 },
    Skip(SkipReason),
}

/// Runs the per-path checks in order; the first that fails is the reason.
fn check(root: &Path, path: &str, filter: &PathFilter, options: &WalkOptions) -> Result<Check, WalkError> {
    if !filter.allows(path) {
        return Ok(Check::Skip(SkipReason::Excluded));
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    if is_deny_listed(name) {
        return Ok(Check::Skip(SkipReason::DenyListed));
    }
    let full = root.join(path);
    let io_error = |source| WalkError::Io { path: full.clone(), source };
    let meta = match fs::symlink_metadata(&full) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Check::Skip(SkipReason::Deleted)),
        Err(e) => return Err(io_error(e)),
    };
    let reason = if meta.file_type().is_symlink() {
        Some(SkipReason::Symlink)
    } else if !meta.is_file() {
        Some(SkipReason::NotAFile)
    } else if meta.len() > options.max_file_size {
        Some(SkipReason::Oversized)
    } else {
        None
    };
    if let Some(reason) = reason {
        return Ok(Check::Skip(reason));
    }
    let mut prefix = Vec::with_capacity(SNIFF_LEN);
    File::open(&full)
        .and_then(|file| file.take(SNIFF_LEN as u64).read_to_end(&mut prefix))
        .map_err(io_error)?;
    if looks_binary(&prefix) {
        return Ok(Check::Skip(SkipReason::Binary));
    }
    Ok(Check::Keep { size: meta.len() })
}
```

Learn — `symlink_metadata` describes the link itself, where `metadata` would follow it; that is what keeps a link to `~/.ssh/id_rsa` from being read. `Read::take(n)` wraps a reader so it stops after `n` bytes, so sniffing a 1 MiB file reads 8000 bytes. A match guard (`Err(e) if e.kind() == …`) handles one error kind and lets the rest fall through to the next arm.

`lib.rs`:

```rust
mod walk;
pub use walk::{
    DEFAULT_MAX_FILE_SIZE, LocalFile, LocalTree, SkipReason, SkippedFile, WalkError, WalkOptions,
    walk_local,
};
```

Delete the `#![allow(dead_code)]` and its comment from the top of `walk.rs`.

Run: `cargo test -p codecite-index`
Expected: all pass, and clippy finds no dead code. `cargo fmt` will rewrap the struct literals above.

- [ ] **Step 3: Adversarial probe**

Point a `LocalSource` at a real checkout (this worktree) from a scratch test or `cargo run --example` that is not committed, and check: no panic, `head` matches `git rev-parse HEAD`, `target/` is one `Ignored` entry, `.env` (if present) is `DenyListed`. Also try a path inside `.git/` (`NotAGitRepo` or `NotRepoRoot`, no panic) and a source whose `include` is `["["]` (`Glob`).

- [ ] **Step 4: Update `docs/design.md`**

- `## Indexing and retrieval`, step 2 becomes:

> 2. **Walk:** list the files git sees (`git ls-files`, so `.gitignore`, `.git/info/exclude` and the global excludes apply and tracked files are never ignored), apply the manifest's `include`/`exclude` globs, and skip deny-listed secrets, symlinks, files over 1 MiB and binaries (a NUL byte in the first 8000 bytes, git's rule; extensions are never trusted). Every skipped path is reported with its reason; an ignored directory counts once.

- `## Corpus manifests and licensing`, the `kind = "local"` paragraph: after "carry `dirty: true` for files with uncommitted changes", add: "(modified, staged or untracked). `path` must be the top of a git work tree with at least one commit; anything else is an error."
- `## Security and privacy`, the data-boundary bullet: "A deny-list skips `.env`, key and certificate files" → "A deny-list skips files by name before opening them: `.env*`, keys and certificates (`*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.crt`, `*.cer`, `*.der`, `*.jks`, `*.keystore`) and SSH keys (`id_rsa`, `id_ed25519.pub`; not `id_token.rs`)".

Check `README.md` and `AGENTS.md` for statements about the walk that are now stale; fix any in this commit.

- [ ] **Step 5: Gates and commit**

Run the gates. Commit `crates/codecite-index/src/{walk.rs,lib.rs}` and `docs/design.md`:

```
feat(index): walk a local checkout and report every skipped file (#19)
```
