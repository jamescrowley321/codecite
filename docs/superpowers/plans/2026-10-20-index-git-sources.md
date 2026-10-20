# Index Git Sources at a Pinned Commit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #29. Index `kind = "git"` sources: keep a partial clone of each repository in the user's cache, fetch the manifest's pinned commit into it, check it out, and index the checkout as a local tree is indexed. Results cite the pinned commit and are never `dirty`. A commit the remote doesn't have is a typed error, and re-indexing at a new commit rewrites only the documents that changed.

**Architecture:**
- New module `crates/codecite-index/src/clone.rs`: `Clones` (a cache directory of clones) and `CloneError`. `Clones::fetch(&GitSource)` makes sure the clone holds the commit: it clones with `git clone --quiet --no-checkout --filter=blob:none -- <url> .` into a temporary directory renamed into place when there is no clone, and runs `git fetch --quiet --filter=blob:none origin <commit>` only when `git cat-file -e <commit>^{commit}` fails. `Clones::checkout(&GitSource)` then runs `checkout --quiet --force --detach <commit>` and `clean --quiet -ffdx`. A clone lives in `<dir>/<sha256 of the URL>`.
- `index_corpus` takes a fifth argument, `clones: &Clones`. It fetches every git source's commit right after the documents, before anything is written. In the source loop a git source is checked out, walked with `walk_local` on its clone (so every skip rule applies), and its files' `dirty` is forced to `false`. Local and git sources then share one writer, `index_tree`, which takes a `Walked` struct (name, kind, URL, license, walk). The source row has `kind = 'git'`, `url` = the manifest URL and `revision` = the pinned commit (the clone's HEAD).
- Re-indexing at a new commit needs no `git diff`: the writer already skips each file whose content hash is unchanged and deletes each stored file the walk no longer lists, so only the files that changed between the two commits are rewritten.
- `codecite index` keeps clones in `~/.cache/codecite/repos` beside `~/.cache/codecite/documents`.

**Tech Stack:** Rust 1.99.1 (edition 2024). No new dependencies: `tempfile` (already a normal dependency of `codecite-index`, for the document cache) makes the temporary clone directory, and `sha2` names the clone. `cargo deny check` is unchanged. Tests need `git` on `PATH`, as the walk's tests already do. Prototyped: every gate passes (286 passed, 7 ignored; deny ok), and a real run against `https://github.com/BurntSushi/walkdir` is in Task 2, Step 6.

**Spec:** Issue #29; `docs/design.md#indexing-pipeline` (Fetch, Walk, Write) and `#freshness`. The gaps the issue leaves are settled under Global Constraints, and Task 3 writes them into design.md.

## Global Constraints

- **Where clones live.** `<cache dir>/codecite/repos/<lowercase hex sha256 of the URL>`. Hashing makes any URL a safe directory name, and two URLs never share a clone. Two sources with one URL share a clone; the run indexes sources one after another, so each is checked out, walked and written before the next checkout.
- **Fetch before write.** As with documents, every git source's commit is fetched before the first write, so a missing commit leaves the store untouched. A source marked `use = "excluded"` is never cloned and is reported as skipped, as an excluded local source is.
- **Offline after the first run.** A clone that already has the pinned commit runs no network command at all. Checking out a commit in a partial clone fetches the files' contents it lacks from the promisor remote, which is the point of `--filter=blob:none`.
- **Typed errors.** `CloneError::CommitNotFound { url, commit }` when the commit is still missing after the fetch (a server refuses a commit no ref reaches, which also counts as missing). `CloneError::BadPin` when `commit` is not 40 lowercase hex digits: `Manifest::parse` already refuses those, but the commit becomes a git argument, so `--upload-pack=…` must never reach git from a `GitSource` built in code. `GitSpawn`, `Git { url, command, stderr }` and `Cache { path, source }` as in the walk and the document cache. `codecite_index::Error::Clone` wraps it.
- **Trust.** The clone is codecite's own, so its config is trusted, unlike a local checkout's. Every command still sets `core.fsmonitor=false`, `GIT_TERMINAL_PROMPT=0` (never prompt for credentials), `GIT_LFS_SKIP_SMUDGE=1` (LFS pointers are indexed as they are), `GIT_CEILING_DIRECTORIES` = the cache directory (a missing clone is never mistaken for an enclosing repository) and removes `GIT_DIR`, `GIT_WORK_TREE` and `GIT_INDEX_FILE`. The URL follows `--`. The walk then runs on the checkout with its own rules (no transport, symlinks skipped).
- **A broken cache entry.** A directory at the clone's path that `git rev-parse --show-toplevel` rejects (a clone cut short before this design's rename, or anything else) is deleted and cloned again.
- **Names.** The manifest's `name`, else the URL's last path segment without `.git` and trailing `/` (`identity-model` for `https://github.com/acme/identity-model.git`, `tools` for `git@github.com:acme/tools.git`); `git` when that is empty. Two sources with one name are still `DuplicateSource`.
- **Tests.** No network: `test_repo::Remote` is a bare repository with `uploadpack.allowFilter`, served as `file://…`, since a plain path makes a local clone, which ignores `--filter`. `Repo::push` pushes HEAD to a branch of it and returns the commit.
- The workspace forbids `unsafe_code`; public items need doc comments.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.

## Review Focus

- `Clones::fetch`: the fetch by commit id, and that a refused fetch becomes `CommitNotFound` rather than a raw git error.
- `clone_into`: the temporary directory is renamed into place only after `git clone` succeeds; on failure the `TempDir` is dropped and deleted.
- `index_corpus`: git commits are fetched before any write, and the git source's `dirty` is always false.

---

### Task 1: The clone cache (`codecite-index`)

**Files:**
- Create: `crates/codecite-index/src/clone.rs`
- Modify: `crates/codecite-index/src/test_repo.rs`, `crates/codecite-index/src/lib.rs`, `crates/codecite-index/src/error.rs`

**Interfaces:**
- Produces (for Task 2): `Clones::new(impl Into<PathBuf>)`, `Clones::path(&self, &str) -> PathBuf`, `Clones::fetch(&self, &GitSource) -> Result<PathBuf, CloneError>`, `Clones::checkout(&self, &GitSource) -> Result<PathBuf, CloneError>`, `CloneError`, `Error::Clone`. Test helpers `test_repo::Remote::{new, url}` and `Repo::push(&self, &Remote, &str) -> String`.

- [ ] **Step 1: Write the failing tests**

This adds the bare remote to `test_repo.rs` and creates `clone.rs` holding only its test module, registered with `mod clone;`.

> **Learn: partial clones.** `git clone --filter=blob:none` asks the server for every commit and tree but no blobs (file contents). The clone records the remote as a *promisor* (`remote.origin.promisor = true`, `remote.origin.partialclonefilter = blob:none`), and any command that needs a missing blob, such as a checkout, fetches it from the remote on demand. So pinning a large repository costs its history's commits and trees plus the files at the pinned commit, never every version of every file. `--no-checkout` skips checking out the default branch, whose blobs we'd fetch only to throw away. The server must allow filters (`uploadpack.allowFilter`); GitHub does. A local path clone hard-links objects and ignores `--filter`, which is why the tests serve the bare repository as `file://`.

`````diff
diff --git a/crates/codecite-index/src/clone.rs b/crates/codecite-index/src/clone.rs
new file mode 100644
index 0000000..7eff7d3
--- /dev/null
+++ b/crates/codecite-index/src/clone.rs
@@ -0,0 +1,140 @@
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::LicenseClass;
+    use crate::test_repo::{Remote, Repo};
+
+    fn source(remote: &Remote, commit: &str) -> GitSource {
+        GitSource {
+            name: None,
+            url: remote.url(),
+            commit: commit.into(),
+            license: None,
+            license_class: LicenseClass::Adapt,
+            include: Vec::new(),
+            exclude: Vec::new(),
+        }
+    }
+
+    fn git_out(dir: &Path, args: &[&str]) -> String {
+        let out = Command::new("git")
+            .arg("-C")
+            .arg(dir)
+            .args(args)
+            .output()
+            .unwrap();
+        assert!(out.status.success(), "git {args:?}");
+        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
+    }
+
+    /// A remote with one commit holding `a.rs`, and a clone cache.
+    fn setup() -> (Repo, Remote, String, tempfile::TempDir) {
+        let repo = Repo::new();
+        repo.write("a.rs", "fn a() {}\n");
+        repo.commit_all(&["a.rs"]);
+        let remote = Remote::new();
+        let commit = repo.push(&remote, "main");
+        (repo, remote, commit, tempfile::tempdir().unwrap())
+    }
+
+    #[test]
+    fn checkout_makes_a_partial_clone_at_the_commit() {
+        let (_repo, remote, commit, cache) = setup();
+        let clones = Clones::new(cache.path().join("repos"));
+        let path = clones.checkout(&source(&remote, &commit)).unwrap();
+
+        assert_eq!(path, clones.path(&remote.url()));
+        assert_eq!(git_out(&path, &["rev-parse", "HEAD"]), commit);
+        assert_eq!(
+            git_out(&path, &["config", "remote.origin.partialclonefilter"]),
+            "blob:none"
+        );
+        assert_eq!(
+            fs::read_to_string(path.join("a.rs")).unwrap(),
+            "fn a() {}\n"
+        );
+        // Only the clone is left in the cache, no temporary directory.
+        let entries: Vec<_> = fs::read_dir(cache.path().join("repos"))
+            .unwrap()
+            .map(|e| e.unwrap().file_name())
+            .collect();
+        assert_eq!(entries, [path.file_name().unwrap()]);
+    }
+
+    #[test]
+    fn a_new_commit_is_fetched_into_the_same_clone() {
+        let (repo, remote, first, cache) = setup();
+        let clones = Clones::new(cache.path());
+        let path = clones.checkout(&source(&remote, &first)).unwrap();
+        // A marker in the clone's git directory survives a fetch, not a re-clone.
+        fs::write(path.join(".git/marker"), "").unwrap();
+
+        repo.write("b.rs", "fn b() {}\n");
+        repo.commit_all(&["b.rs"]);
+        // A side branch: fetching by id must reach commits no clone saw.
+        let second = repo.push(&remote, "side");
+        assert_eq!(clones.checkout(&source(&remote, &second)).unwrap(), path);
+        assert!(path.join(".git/marker").exists());
+        assert_eq!(git_out(&path, &["rev-parse", "HEAD"]), second);
+        assert!(path.join("b.rs").exists());
+
+        // Back to the first commit: b.rs goes, and so does a stray file.
+        fs::write(path.join("stray.txt"), "left behind\n").unwrap();
+        clones.checkout(&source(&remote, &first)).unwrap();
+        assert!(!path.join("b.rs").exists());
+        assert!(!path.join("stray.txt").exists());
+    }
+
+    #[test]
+    fn a_clone_with_the_commit_needs_no_remote() {
+        let (_repo, remote, commit, cache) = setup();
+        let clones = Clones::new(cache.path());
+        let git = source(&remote, &commit);
+        clones.checkout(&git).unwrap();
+
+        drop(remote);
+        let path = clones.checkout(&git).unwrap();
+        assert_eq!(git_out(&path, &["rev-parse", "HEAD"]), commit);
+    }
+
+    #[test]
+    fn a_commit_the_remote_lacks_is_a_typed_error() {
+        let (_repo, remote, _commit, cache) = setup();
+        let clones = Clones::new(cache.path());
+        let missing = "0123456789abcdef0123456789abcdef01234567";
+        let err = clones.fetch(&source(&remote, missing)).unwrap_err();
+        assert!(
+            matches!(&err, CloneError::CommitNotFound { url, commit }
+                if *url == remote.url() && commit == missing),
+            "{err:?}"
+        );
+        assert!(
+            err.to_string().contains("has no commit 0123456789ab"),
+            "{err}"
+        );
+    }
+
+    #[test]
+    fn a_pin_that_is_not_a_commit_id_never_reaches_git() {
+        let (_repo, remote, _commit, cache) = setup();
+        let clones = Clones::new(cache.path().join("repos"));
+        for pin in ["--upload-pack=touch /tmp/x", "HEAD", "0123456"] {
+            let err = clones.fetch(&source(&remote, pin)).unwrap_err();
+            assert!(matches!(err, CloneError::BadPin { .. }), "{err:?}");
+        }
+        assert!(!cache.path().join("repos").exists());
+    }
+
+    #[test]
+    fn a_broken_clone_is_replaced() {
+        let (_repo, remote, commit, cache) = setup();
+        let clones = Clones::new(cache.path());
+        let path = clones.path(&remote.url());
+        fs::create_dir_all(&path).unwrap();
+        fs::write(path.join("half"), "").unwrap();
+
+        clones.checkout(&source(&remote, &commit)).unwrap();
+        assert!(!path.join("half").exists());
+        assert_eq!(git_out(&path, &["rev-parse", "HEAD"]), commit);
+    }
+}
diff --git a/crates/codecite-index/src/lib.rs b/crates/codecite-index/src/lib.rs
index 25b9533..9a63332 100644
--- a/crates/codecite-index/src/lib.rs
+++ b/crates/codecite-index/src/lib.rs
@@ -14,6 +14,8 @@ pub use chunk::{
     Chunk, ChunkKind, Chunker, ChunkerError, Language, Symbol, SymbolKind, WINDOW_LINES,
 };
 
+mod clone;
+
 mod error;
 pub use error::{Error, Result};
 
diff --git a/crates/codecite-index/src/test_repo.rs b/crates/codecite-index/src/test_repo.rs
index ccfcb53..65552d9 100644
--- a/crates/codecite-index/src/test_repo.rs
+++ b/crates/codecite-index/src/test_repo.rs
@@ -1,4 +1,5 @@
-//! A throwaway git repository for the walk's and the index's tests.
+//! Throwaway git repositories for the walk's, the clone cache's and the
+//! index's tests: a work tree to commit in and a bare remote to push to.
 
 use std::fs;
 use std::path::Path;
@@ -63,4 +64,48 @@ impl Repo {
         self.git(&args);
         self.git(&["commit", "-q", "-m", "test"]);
     }
+
+    /// Pushes HEAD to `branch` on `remote` and returns HEAD's commit.
+    pub(crate) fn push(&self, remote: &Remote, branch: &str) -> String {
+        self.git(&[
+            "push",
+            "-q",
+            &remote.url(),
+            &format!("HEAD:refs/heads/{branch}"),
+        ]);
+        let output = Command::new("git")
+            .arg("-C")
+            .arg(self.path())
+            .args(["rev-parse", "HEAD"])
+            .output()
+            .unwrap();
+        String::from_utf8(output.stdout)
+            .unwrap()
+            .trim_end()
+            .to_owned()
+    }
+}
+
+/// A bare repository in a temporary directory, served to clones over
+/// `file://` so they behave as they would over the network: a plain path
+/// would make a local clone, which ignores `--filter`.
+pub(crate) struct Remote {
+    repo: Repo,
+}
+
+impl Remote {
+    /// `git init --bare`, allowing partial clones.
+    pub(crate) fn new() -> Self {
+        let repo = Repo {
+            dir: tempfile::tempdir().unwrap(),
+        };
+        repo.git(&["init", "-q", "--bare"]);
+        repo.git(&["config", "uploadpack.allowFilter", "true"]);
+        Remote { repo }
+    }
+
+    /// The remote's `file://` URL.
+    pub(crate) fn url(&self) -> String {
+        format!("file://{}", self.repo.path().display())
+    }
 }
`````

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite-index --no-run`
Expected: compile errors for what Step 3 adds: `cannot find type Clones in this scope`, `cannot find type CloneError in this scope`, `cannot find type GitSource in this scope`, and `fs`, `Path` and `Command` unresolved, since the test module's `use super::*` finds no imports yet (18 errors).

- [ ] **Step 3: Add `Clones` and `CloneError`**

> **Learn: driving `git` from Rust.** `std::process::Command` builds a child process: `.arg`/`.args` add arguments one by one, never through a shell, so a URL with spaces or `;` is one argument and needs no quoting. `.env` and `.env_remove` set and clear variables for the child only. `.output()` runs it to completion and returns its exit status, stdout and stderr as bytes; `String::from_utf8_lossy` turns stderr into text for the error. `run` maps the two failure kinds onto variants: a process that never started is `GitSpawn` (git not installed) and a non-zero exit is `Git { command, stderr }`. Callers that expect git to fail (`cat-file -e` for a missing commit) match `Err(CloneError::Git { .. })` and pass every other error on. Two git-specific guards: the URL comes after `--`, so a URL starting with `-` can't be read as an option, and the pinned commit is checked to be 40 hex digits before it is ever an argument.
>
> Also used: `tempfile::Builder::tempdir_in` makes the clone's temporary directory beside its final place, so `fs::rename` is a move within one file system, and `TempDir::keep` stops it being deleted on drop once the clone has succeeded. A `TempDir` dropped on an early `?` return deletes the half-made clone. `format!("{}^{{commit}}", …)` doubles the braces to print literal ones.

`````diff
diff --git a/crates/codecite-index/src/clone.rs b/crates/codecite-index/src/clone.rs
index 7eff7d3..eeea6fc 100644
--- a/crates/codecite-index/src/clone.rs
+++ b/crates/codecite-index/src/clone.rs
@@ -1,3 +1,251 @@
+//! Git sources: one partial clone per repository URL, kept in the cache,
+//! fetched until it holds the manifest's pinned commit and checked out at it.
+
+use std::fs;
+use std::io;
+use std::path::{Path, PathBuf};
+use std::process::Command;
+
+use sha2::{Digest, Sha256};
+
+use crate::GitSource;
+use crate::fetch::hex;
+use crate::manifest::is_lower_hex;
+
+/// Keeps a partial clone of each git source's repository in a cache
+/// directory and checks it out at the pinned commit.
+///
+/// A clone is made with `--filter=blob:none`: it holds every commit and tree
+/// but fetches a file's contents only when a checkout needs them, so pinning
+/// a large repository costs the files at the pinned commit, not its whole
+/// history. A clone is never made twice: when the pinned commit moves, the
+/// clone is fetched, and when the clone already has the commit, nothing is
+/// fetched at all, so indexing works offline after the first run.
+#[derive(Debug, Clone)]
+pub struct Clones {
+    dir: PathBuf,
+}
+
+/// Why a git source could not be cloned, fetched or checked out.
+#[derive(Debug, thiserror::Error)]
+#[non_exhaustive]
+pub enum CloneError {
+    /// The pinned `commit` is not 40 lowercase hex digits. [`crate::Manifest::parse`]
+    /// rejects these; this guards a [`GitSource`] built any other way, since
+    /// the commit becomes an argument to git.
+    #[error("git source {url} pins commit {commit:?}, which is not 40 lowercase hex digits")]
+    BadPin {
+        /// The repository URL.
+        url: String,
+        /// The pin as given.
+        commit: String,
+    },
+
+    /// The repository has no commit with the pinned id, even after fetching.
+    #[error(
+        "git source {url} has no commit {commit}; check the manifest's `commit`, \
+         or push the commit if it only exists locally"
+    )]
+    CommitNotFound {
+        /// The repository URL.
+        url: String,
+        /// The pinned commit.
+        commit: String,
+    },
+
+    /// `git` could not be started (not installed, or not on `PATH`).
+    #[error("could not run git")]
+    GitSpawn {
+        /// Why the process did not start.
+        source: io::Error,
+    },
+
+    /// A `git` command exited with an error.
+    #[error("`{command}` failed for git source {url}: {stderr}")]
+    Git {
+        /// The repository URL.
+        url: String,
+        /// The command line, without `-C <dir>`.
+        command: String,
+        /// What git printed on stderr, trimmed.
+        stderr: String,
+    },
+
+    /// Creating or removing a directory in the clone cache failed.
+    #[error("clone cache {}", path.display())]
+    Cache {
+        /// The directory.
+        path: PathBuf,
+        /// What went wrong.
+        source: io::Error,
+    },
+}
+
+impl Clones {
+    /// Clones kept under `dir`, created on first use.
+    pub fn new(dir: impl Into<PathBuf>) -> Self {
+        Clones { dir: dir.into() }
+    }
+
+    /// The clone of `url`: a directory named by the URL's sha256, so any URL
+    /// is a safe directory name and two URLs never share a clone.
+    pub fn path(&self, url: &str) -> PathBuf {
+        self.dir.join(hex(&Sha256::digest(url)))
+    }
+
+    /// Makes sure the clone of `git.url` holds `git.commit`: clones the
+    /// repository when there is no clone yet, else fetches when the commit
+    /// is missing. Returns the clone's directory.
+    ///
+    /// A commit the remote doesn't have is [`CloneError::CommitNotFound`].
+    pub fn fetch(&self, git: &GitSource) -> Result<PathBuf, CloneError> {
+        if !is_lower_hex(&git.commit, 40) {
+            return Err(CloneError::BadPin {
+                url: git.url.clone(),
+                commit: git.commit.clone(),
+            });
+        }
+        let path = self.path(&git.url);
+        if !self.is_clone(&path, git)? {
+            self.clone_into(&path, git)?;
+        }
+        if !has_commit(&path, git)? {
+            // A server can refuse a commit no branch or tag reaches; then
+            // the commit is not there to pin.
+            let fetched = run(
+                &path,
+                git,
+                &[
+                    "fetch",
+                    "--quiet",
+                    "--filter=blob:none",
+                    "origin",
+                    &git.commit,
+                ],
+            );
+            match fetched {
+                Ok(_) | Err(CloneError::Git { .. }) => {}
+                Err(e) => return Err(e),
+            }
+            if !has_commit(&path, git)? {
+                return Err(CloneError::CommitNotFound {
+                    url: git.url.clone(),
+                    commit: git.commit.clone(),
+                });
+            }
+        }
+        Ok(path)
+    }
+
+    /// Checks out `git.commit` in its clone, which [`Clones::fetch`] made
+    /// sure holds it, and removes every file the commit doesn't track, so
+    /// the work tree is exactly the commit. Fetches the files' contents
+    /// that the clone doesn't have yet. Returns the clone's directory.
+    pub fn checkout(&self, git: &GitSource) -> Result<PathBuf, CloneError> {
+        let path = self.fetch(git)?;
+        run(
+            &path,
+            git,
+            &["checkout", "--quiet", "--force", "--detach", &git.commit],
+        )?;
+        run(&path, git, &["clean", "--quiet", "-ffdx"])?;
+        Ok(path)
+    }
+
+    /// Whether `path` is a clone. A directory that isn't one, left by a
+    /// clone that was cut short, is deleted.
+    fn is_clone(&self, path: &Path, git: &GitSource) -> Result<bool, CloneError> {
+        if !path.exists() {
+            return Ok(false);
+        }
+        match run(path, git, &["rev-parse", "--show-toplevel"]) {
+            Ok(_) => return Ok(true),
+            Err(CloneError::Git { .. }) => {}
+            Err(e) => return Err(e),
+        }
+        fs::remove_dir_all(path).map_err(|source| CloneError::Cache {
+            path: path.to_owned(),
+            source,
+        })?;
+        Ok(false)
+    }
+
+    /// Clones `git.url` into `path` without checking anything out. The clone
+    /// is made in a temporary directory and renamed into place, so a clone
+    /// cut short never looks like a finished one.
+    fn clone_into(&self, path: &Path, git: &GitSource) -> Result<(), CloneError> {
+        let cache_error = |source| CloneError::Cache {
+            path: self.dir.clone(),
+            source,
+        };
+        fs::create_dir_all(&self.dir).map_err(cache_error)?;
+        let tmp = tempfile::Builder::new()
+            .prefix(".clone-")
+            .tempdir_in(&self.dir)
+            .map_err(cache_error)?;
+        run(
+            tmp.path(),
+            git,
+            &[
+                "clone",
+                "--quiet",
+                "--no-checkout",
+                "--filter=blob:none",
+                "--",
+                &git.url,
+                ".",
+            ],
+        )?;
+        fs::rename(tmp.keep(), path).map_err(cache_error)
+    }
+}
+
+/// Whether the clone at `path` holds `git.commit` as a commit.
+fn has_commit(path: &Path, git: &GitSource) -> Result<bool, CloneError> {
+    let object = format!("{}^{{commit}}", git.commit);
+    match run(path, git, &["cat-file", "-e", &object]) {
+        Ok(_) => Ok(true),
+        Err(CloneError::Git { .. }) => Ok(false),
+        Err(e) => Err(e),
+    }
+}
+
+/// Runs `git -C <dir> <args>` and returns its stdout.
+///
+/// The clone is codecite's own, so unlike a local source's checkout its
+/// config is trusted. Git still looks no higher than the cache for a
+/// repository, so a missing clone is never mistaken for an enclosing one,
+/// and Git LFS, if installed, leaves pointer files as they are rather than
+/// downloading what they point to.
+fn run(dir: &Path, git: &GitSource, args: &[&str]) -> Result<Vec<u8>, CloneError> {
+    let mut command = Command::new("git");
+    command
+        .arg("-C")
+        .arg(dir)
+        .args(["-c", "core.fsmonitor=false"])
+        .args(args)
+        .env("GIT_TERMINAL_PROMPT", "0")
+        .env("GIT_LFS_SKIP_SMUDGE", "1")
+        // `-C` picks the repository; an inherited GIT_DIR would override it.
+        .env_remove("GIT_DIR")
+        .env_remove("GIT_WORK_TREE")
+        .env_remove("GIT_INDEX_FILE");
+    if let Some(parent) = dir.parent() {
+        command.env("GIT_CEILING_DIRECTORIES", parent);
+    }
+    let output = command
+        .output()
+        .map_err(|source| CloneError::GitSpawn { source })?;
+    if !output.status.success() {
+        return Err(CloneError::Git {
+            url: git.url.clone(),
+            command: format!("git {}", args.join(" ")),
+            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
+        });
+    }
+    Ok(output.stdout)
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
diff --git a/crates/codecite-index/src/error.rs b/crates/codecite-index/src/error.rs
index c0c0680..581ccaf 100644
--- a/crates/codecite-index/src/error.rs
+++ b/crates/codecite-index/src/error.rs
@@ -29,6 +29,11 @@ pub enum Error {
     #[error(transparent)]
     Fetch(#[from] crate::FetchError),
 
+    /// A git source could not be cloned, fetched or checked out, or its
+    /// repository has no commit with the pinned id.
+    #[error(transparent)]
+    Clone(#[from] crate::CloneError),
+
     /// A walked file could not be read.
     #[error("reading {}", path.display())]
     ReadFile {
diff --git a/crates/codecite-index/src/lib.rs b/crates/codecite-index/src/lib.rs
index 9a63332..3dc0864 100644
--- a/crates/codecite-index/src/lib.rs
+++ b/crates/codecite-index/src/lib.rs
@@ -15,6 +15,7 @@ pub use chunk::{
 };
 
 mod clone;
+pub use clone::{CloneError, Clones};
 
 mod error;
 pub use error::{Error, Result};
`````

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite-index clone`
Expected: `test result: ok. 6 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/codecite-index/src/clone.rs crates/codecite-index/src/test_repo.rs crates/codecite-index/src/lib.rs crates/codecite-index/src/error.rs
git commit -m "feat(index): keep a partial clone of each git source (#29)"
```

---

### Task 2: Index git sources (`codecite-index`, `codecite` binary)

**Files:**
- Modify: `crates/codecite-index/src/index.rs`, `crates/codecite-index/src/lib.rs`, `crates/codecite/src/main.rs`, `crates/codecite/tests/index.rs`

**Interfaces:**
- Changes: `index_corpus(store, manifest, options, fetcher, clones: &Clones)`. A git source is no longer reported as "git sources are not indexed yet".
- Consumes: Task 1's `Clones`.

- [ ] **Step 1: Write the failing tests**

The index tests cover a git source at its commit (name from the URL, `kind`, `url`, `revision`, never dirty, a later unpushed commit not indexed), a re-index at a new commit (1 written, 1 unchanged, 1 removed; the unchanged file keeps its chunk IDs), a missing commit stopping the run before any write, names from URLs, and an excluded git source never cloned. The CLI test indexes a git source, reads its citation back, fails on a missing commit, and indexes again with the remote deleted.

`````diff
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
index 7d5d440..a565541 100644
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -426,7 +426,7 @@ impl fmt::Display for IndexSummary {
 #[cfg(test)]
 mod tests {
     use super::*;
-    use crate::test_repo::Repo;
+    use crate::test_repo::{Remote, Repo};
     use crate::test_server::Server;
     use rusqlite::Connection;
     use std::path::PathBuf;
@@ -485,6 +485,30 @@ mod tests {
         source
     }
 
+    fn git(remote: &Remote, commit: &str) -> Source {
+        Source::Git(GitSource {
+            name: None,
+            url: remote.url(),
+            commit: commit.into(),
+            license: Some("Apache-2.0".into()),
+            license_class: LicenseClass::Adapt,
+            include: Vec::new(),
+            exclude: Vec::new(),
+        })
+    }
+
+    /// Each document's path and `dirty` flag, by path.
+    fn dirty(corpus: &Corpus) -> Vec<(String, bool)> {
+        let conn = corpus.sql();
+        let mut stmt = conn
+            .prepare("SELECT path, dirty FROM document ORDER BY path")
+            .unwrap();
+        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
+            .unwrap()
+            .collect::<rusqlite::Result<_>>()
+            .unwrap()
+    }
+
     fn manifest(sources: Vec<Source>) -> Manifest {
         Manifest {
             name: "test".into(),
@@ -529,7 +553,14 @@ mod tests {
         fn try_index(&self, manifest: &Manifest) -> Result<IndexSummary> {
             let mut store = Store::open_read_write(&self.path()).unwrap();
             let fetcher = Fetcher::new(self.dir.path().join("cache"));
-            index_corpus(&mut store, manifest, &WalkOptions::default(), &fetcher)
+            let clones = Clones::new(self.dir.path().join("repos"));
+            index_corpus(
+                &mut store,
+                manifest,
+                &WalkOptions::default(),
+                &fetcher,
+                &clones,
+            )
         }
 
         fn sql(&self) -> Connection {
@@ -642,18 +673,8 @@ mod tests {
         assert_eq!(after["src/lib.rs"].len(), 1);
         assert_eq!(corpus.matches("symbol:validate"), 0);
         assert_eq!(corpus.matches("symbol:refresh"), 1);
-        let dirty: Vec<(String, bool)> = {
-            let conn = corpus.sql();
-            let mut stmt = conn
-                .prepare("SELECT path, dirty FROM document ORDER BY path")
-                .unwrap();
-            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
-                .unwrap()
-                .collect::<rusqlite::Result<_>>()
-                .unwrap()
-        };
         assert_eq!(
-            dirty,
+            dirty(&corpus),
             [
                 ("README.md".to_owned(), false),
                 ("notes.txt".to_owned(), false),
@@ -744,6 +765,140 @@ mod tests {
         );
     }
 
+    #[test]
+    fn indexes_a_git_source_at_its_pinned_commit() {
+        let repo = fixture();
+        let remote = Remote::new();
+        let commit = repo.push(&remote, "main");
+        // The checkout the remote was pushed from has moved on; the pin hasn't.
+        repo.write("src/lib.rs", "pub fn unpushed() {}\n");
+        repo.commit_all(&["src/lib.rs"]);
+        let corpus = Corpus::new();
+        let summary = corpus.index(&manifest(vec![git(&remote, &commit)]));
+
+        assert_eq!((summary.written, summary.chunks), (3, 4));
+        assert!(summary.skipped_sources.is_empty());
+        let source: (String, String, String, String, String) = corpus
+            .sql()
+            .query_row(
+                "SELECT name, kind, url, revision, license_spdx FROM source",
+                [],
+                |row| {
+                    Ok((
+                        row.get(0)?,
+                        row.get(1)?,
+                        row.get(2)?,
+                        row.get(3)?,
+                        row.get(4)?,
+                    ))
+                },
+            )
+            .unwrap();
+        // The name is the URL's last segment: the remote's directory name.
+        let dir = remote.url().rsplit('/').next().unwrap().to_owned();
+        assert_eq!(
+            source,
+            (dir, "git".into(), remote.url(), commit, "Apache-2.0".into())
+        );
+        assert_eq!(corpus.matches("symbol:validate"), 1);
+        assert_eq!(corpus.matches("symbol:unpushed"), 0);
+        assert!(dirty(&corpus).iter().all(|(_, dirty)| !dirty));
+    }
+
+    #[test]
+    fn reindexing_a_git_source_at_a_new_commit_rewrites_only_what_changed() {
+        let repo = fixture();
+        let remote = Remote::new();
+        let first = repo.push(&remote, "main");
+        let corpus = Corpus::new();
+        corpus.index(&manifest(vec![git(&remote, &first)]));
+        let before = corpus.chunk_ids();
+
+        repo.write("src/lib.rs", "pub fn refresh_session() {}\n");
+        repo.git(&["rm", "-q", "notes.txt"]);
+        repo.commit_all(&["src/lib.rs"]);
+        let second = repo.push(&remote, "main");
+        let summary = corpus.index(&manifest(vec![git(&remote, &second)]));
+
+        assert_eq!(
+            (summary.written, summary.unchanged, summary.removed),
+            (1, 1, 1)
+        );
+        let after = corpus.chunk_ids();
+        assert_eq!(after["README.md"], before["README.md"]);
+        assert!(!after.contains_key("notes.txt"));
+        assert_eq!(corpus.matches("symbol:refresh"), 1);
+        assert_eq!(corpus.matches("symbol:validate"), 0);
+        let revision: String = corpus
+            .sql()
+            .query_row("SELECT revision FROM source", [], |row| row.get(0))
+            .unwrap();
+        assert_eq!(revision, second);
+        assert!(dirty(&corpus).iter().all(|(_, dirty)| !dirty));
+
+        // Pinning the first commit again brings its files back.
+        let summary = corpus.index(&manifest(vec![git(&remote, &first)]));
+        assert_eq!(
+            (summary.written, summary.unchanged, summary.removed),
+            (2, 1, 0)
+        );
+    }
+
+    #[test]
+    fn a_missing_commit_stops_the_run_before_any_write() {
+        let repo = fixture();
+        let remote = Remote::new();
+        repo.push(&remote, "main");
+        let corpus = Corpus::new();
+        let missing = "0123456789abcdef0123456789abcdef01234567";
+        let manifest = manifest(vec![local(&repo, Some("app")), git(&remote, missing)]);
+
+        let err = corpus.try_index(&manifest).unwrap_err();
+        assert!(
+            matches!(&err, Error::Clone(crate::CloneError::CommitNotFound { commit, .. })
+                if commit == missing),
+            "{err:?}"
+        );
+        let sources: i64 = corpus
+            .sql()
+            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))
+            .unwrap();
+        assert_eq!(sources, 0);
+    }
+
+    #[test]
+    fn git_sources_are_named_by_url_and_excluded_ones_are_not_cloned() {
+        let remote = Remote::new();
+        let mut source = git(&remote, "0123456789abcdef0123456789abcdef01234567");
+        let Source::Git(g) = &mut source else {
+            unreachable!()
+        };
+        for (url, name) in [
+            (
+                "https://github.com/acme/identity-model.git",
+                "identity-model",
+            ),
+            ("https://github.com/acme/identity-model/", "identity-model"),
+            ("git@github.com:acme/tools.git", "tools"),
+            ("file:///srv/git/app", "app"),
+            ("/", "git"),
+        ] {
+            g.url = url.into();
+            assert_eq!(git_name(g), name, "{url}");
+        }
+        g.name = Some("client".into());
+        assert_eq!(git_name(g), "client");
+
+        g.license_class = LicenseClass::Excluded;
+        let corpus = Corpus::new();
+        let summary = corpus.index(&manifest(vec![source]));
+        assert_eq!(
+            summary.skipped_sources,
+            [("client".to_owned(), "use = \"excluded\"")]
+        );
+        assert!(!corpus.dir.path().join("repos").exists());
+    }
+
     #[test]
     fn a_document_is_indexed_as_sections_and_steps() {
         let server = Server::start();
@@ -1018,7 +1173,7 @@ mod tests {
             downloaded: 1,
             cached: 2,
             skipped: BTreeMap::from([(SkipReason::Ignored, 3), (SkipReason::Binary, 1)]),
-            skipped_sources: vec![("client".into(), "git sources are not indexed yet")],
+            skipped_sources: vec![("client".into(), "use = \"excluded\"")],
             removed_sources: vec!["old".into()],
             elapsed: Duration::from_millis(1250),
         };
@@ -1028,7 +1183,7 @@ mod tests {
              chunks: 9 written\n\
              fetched: 1 downloaded, 2 from the cache\n\
              skipped: 3 ignored, 1 binary\n\
-             source client not indexed: git sources are not indexed yet\n\
+             source client not indexed: use = \"excluded\"\n\
              source old removed: the run no longer indexes it\n\
              elapsed: 1.25s"
         );
diff --git a/crates/codecite/tests/index.rs b/crates/codecite/tests/index.rs
index 3610b2a..f2b2169 100644
--- a/crates/codecite/tests/index.rs
+++ b/crates/codecite/tests/index.rs
@@ -89,6 +89,83 @@ fn index_writes_the_corpus_file_and_reindexes_incrementally() {
     );
 }
 
+/// The stdout of `git -C <dir> <args>`, trimmed.
+fn git_out(dir: &Path, args: &[&str]) -> String {
+    let output = Command::new("git")
+        .arg("-C")
+        .arg(dir)
+        .args(args)
+        .output()
+        .unwrap();
+    assert!(output.status.success(), "git {args:?}");
+    String::from_utf8(output.stdout)
+        .unwrap()
+        .trim_end()
+        .to_owned()
+}
+
+#[test]
+fn a_git_source_is_cloned_once_and_cited_at_its_commit() {
+    let tmp = tempfile::tempdir().unwrap();
+    let repo = tmp.path().join("app");
+    fs::create_dir_all(repo.join("src")).unwrap();
+    git(&repo, &["init", "-q"]);
+    fs::write(repo.join("src/lib.rs"), "pub fn validate_token() {}\n").unwrap();
+    git(&repo, &["add", "."]);
+    git(&repo, &["commit", "-q", "-m", "init"]);
+    // A bare remote served over file://, so the clone is a partial one.
+    let remote = tmp.path().join("remote/app.git");
+    fs::create_dir_all(&remote).unwrap();
+    git(&remote, &["init", "-q", "--bare"]);
+    git(&remote, &["config", "uploadpack.allowFilter", "true"]);
+    let url = format!("file://{}", remote.display());
+    git(&repo, &["push", "-q", &url, "HEAD:refs/heads/main"]);
+    let commit = git_out(&repo, &["rev-parse", "HEAD"]);
+
+    let work = tmp.path().join("work");
+    fs::create_dir_all(work.join("corpora")).unwrap();
+    let pin = |commit: &str| {
+        fs::write(
+            work.join("corpora/demo.toml"),
+            format!(
+                "name = \"demo\"\n\n[[source]]\nurl = {url:?}\ncommit = {commit:?}\nuse = \"adapt\"\n"
+            ),
+        )
+        .unwrap();
+    };
+    pin(&commit);
+    let first = stdout(&codecite(&work, tmp.path(), &["index", "demo"]));
+    assert!(
+        first.contains("documents: 1 written, 0 unchanged, 0 removed"),
+        "{first}"
+    );
+    let repos = tmp.path().join("cache/codecite/repos");
+    assert_eq!(fs::read_dir(&repos).unwrap().count(), 1);
+
+    let citation = format!("app@{}:src/lib.rs#L1", &commit[..7]);
+    let read = stdout(&codecite(&work, tmp.path(), &["read", "demo", &citation]));
+    assert!(read.contains("validate_token"), "{read}");
+
+    // A commit the remote doesn't have is an error naming it.
+    pin(&"0".repeat(40));
+    let missing = codecite(&work, tmp.path(), &["index", "demo"]);
+    assert!(!missing.status.success());
+    let stderr = String::from_utf8_lossy(&missing.stderr);
+    assert!(
+        stderr.contains(&format!("has no commit {}", "0".repeat(40))),
+        "{stderr}"
+    );
+
+    // With the remote gone, the clone still has the pinned commit.
+    fs::remove_dir_all(tmp.path().join("remote")).unwrap();
+    pin(&commit);
+    let offline = stdout(&codecite(&work, tmp.path(), &["index", "demo"]));
+    assert!(
+        offline.contains("documents: 0 written, 1 unchanged, 0 removed"),
+        "{offline}"
+    );
+}
+
 #[test]
 fn index_errors_name_the_problem() {
     let tmp = tempfile::tempdir().unwrap();
`````

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite-index --no-run`
Expected: 5 compile errors: `this function takes 4 arguments but 5 arguments were supplied`, `cannot find type Clones in this scope`, `cannot find struct, variant or union type GitSource in this scope`, and `cannot find function git_name in this scope` (twice).

Run: `cargo test -p codecite --test index`
Expected: `a_git_source_is_cloned_once_and_cited_at_its_commit` FAILED, since the run prints `documents: 0 written, 0 unchanged, 0 removed` and skips the git source.

- [ ] **Step 3: Index git sources**

> **Learn: grouping arguments in a struct.** Local and git sources differ only in their source row (kind, URL, license) and in how they are walked, so the writer takes one `Walked` value instead of seven arguments, which would trip clippy's `too_many_arguments` (more than 7). The struct borrows the license (`Option<&'a str>`) from the manifest and owns what the walk produced (`String`, `LocalTree`), so its lifetime parameter says it can't outlive the manifest. Each `match` arm builds a `Walked`, and arm guards (`Source::Git(git) if git.license_class != LicenseClass::Excluded`) put the excluded sources in later arms.
>
> Also used: `str::rsplit` with a `[char; 2]` pattern splits on either `/` or `:`, so one call names both `https://…/tools.git` and `git@host:acme/tools.git`; `if let Source::Git(git) = source && …` is a let chain (edition 2024).

`````diff
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
index a565541..e699c80 100644
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -1,5 +1,5 @@
-//! Indexing a corpus: walk each local source, chunk the files whose content
-//! changed, split each new document version into its sections and steps,
+//! Indexing a corpus: walk each local source and each git source's clone at
+//! its pinned commit, chunk the files whose content changed, split each new document version into its sections and steps,
 //! and write them into the store, one transaction per document.
 
 use std::collections::{BTreeMap, HashSet};
@@ -12,8 +12,9 @@ use codecite_core::{ChunkRecord, DocumentRecord, SourceRecord, SpecAliases, Stor
 use sha2::{Digest, Sha256};
 
 use crate::{
-    Chunker, DocumentSource, Error, Fetcher, Language, LicenseClass, LocalSource, LocalTree,
-    Manifest, Result, SkipReason, Source, SpecDocument, WalkOptions, split_spec, walk_local,
+    Chunker, Clones, DocumentSource, Error, Fetcher, GitSource, Language, LicenseClass,
+    LocalSource, LocalTree, Manifest, Result, SkipReason, Source, SpecDocument, WalkOptions,
+    split_spec, walk_local,
 };
 
 /// What an index run did.
@@ -41,24 +42,29 @@ pub struct IndexSummary {
     pub elapsed: Duration,
 }
 
-/// Fetches every document source in `manifest` and indexes it and every
-/// local source into `store`.
+/// Fetches every document source and git source in `manifest` and indexes
+/// them and every local source into `store`.
 ///
 /// Documents are fetched first, from `fetcher`'s cache when they are there,
 /// so a document whose bytes don't match its pinned `sha256` stops the run
-/// before anything is written. Each pinned version of a document is its own
+/// before anything is written. So is each git source's pinned commit, into
+/// its clone in `clones`, so a commit the repository lacks stops the run
+/// too. A git source is then checked out at its commit and walked like a
+/// local checkout, and its files are never `dirty`. Each pinned version of a document is its own
 /// source, split into section and step chunks when it is first indexed.
 ///
 /// A file whose content hash matches the stored one is not rewritten, and a
-/// stored file that is gone, or is now skipped, is deleted. Git sources are
-/// reported in [`IndexSummary::skipped_sources`], as are sources marked
-/// `excluded`. Every source the run did not index, a document version no
+/// stored file that is gone, or is now skipped, is deleted, so re-indexing
+/// a git source at a new commit rewrites only the files that changed between
+/// the two commits. Sources marked `excluded` are reported in
+/// [`IndexSummary::skipped_sources`]. Every source the run did not index, a document version no
 /// longer pinned included, is then deleted from the store.
 pub fn index_corpus(
     store: &mut Store,
     manifest: &Manifest,
     options: &WalkOptions,
     fetcher: &Fetcher,
+    clones: &Clones,
 ) -> Result<IndexSummary> {
     let started = Instant::now();
     let mut summary = IndexSummary::default();
@@ -76,13 +82,38 @@ pub fn index_corpus(
             documents.push((doc, fetched.bytes));
         }
     }
+    for source in &manifest.sources {
+        if let Source::Git(git) = source
+            && git.license_class != LicenseClass::Excluded
+        {
+            clones.fetch(git)?;
+        }
+    }
     let aliases = document_aliases(manifest)?;
     let mut chunker = Chunker::new()?;
     let mut indexed: Vec<String> = Vec::new();
     let mut keep: Vec<i64> = Vec::new();
     for source in &manifest.sources {
-        let local = match source {
-            Source::Local(local) if local.license_class != LicenseClass::Excluded => local,
+        let walked = match source {
+            Source::Local(local) if local.license_class != LicenseClass::Excluded => {
+                let tree = walk_local(local, options)?;
+                Walked {
+                    name: source_name(local, &tree),
+                    kind: "local",
+                    url: tree.root.to_string_lossy().into_owned(),
+                    license: local.license.as_deref(),
+                    license_class: local.license_class,
+                    tree,
+                }
+            }
+            Source::Git(git) if git.license_class != LicenseClass::Excluded => Walked {
+                name: git_name(git),
+                kind: "git",
+                url: git.url.clone(),
+                license: git.license.as_deref(),
+                license_class: git.license_class,
+                tree: walk_git(clones, git, options)?,
+            },
             Source::Local(local) => {
                 let name = local
                     .name
@@ -92,10 +123,9 @@ pub fn index_corpus(
                 continue;
             }
             Source::Git(git) => {
-                let name = git.name.clone().unwrap_or_else(|| git.url.clone());
                 summary
                     .skipped_sources
-                    .push((name, "git sources are not indexed yet"));
+                    .push((git_name(git), "use = \"excluded\""));
                 continue;
             }
             Source::Document(doc) => {
@@ -107,20 +137,11 @@ pub fn index_corpus(
                 continue;
             }
         };
-        let tree = walk_local(local, options)?;
-        let name = source_name(local, &tree);
-        if indexed.contains(&name) {
-            return Err(Error::DuplicateSource { name });
+        if indexed.contains(&walked.name) {
+            return Err(Error::DuplicateSource { name: walked.name });
         }
-        keep.push(index_local(
-            store,
-            &mut chunker,
-            local,
-            &name,
-            &tree,
-            &mut summary,
-        )?);
-        indexed.push(name);
+        keep.push(index_tree(store, &mut chunker, &walked, &mut summary)?);
+        indexed.push(walked.name);
     }
     for (doc, bytes) in &documents {
         // A document's versions share its name, but no other source may.
@@ -174,23 +195,67 @@ fn source_name(local: &LocalSource, tree: &LocalTree) -> String {
     })
 }
 
-/// Writes one walked local source into the store and returns its ID.
-fn index_local(
+/// The manifest's `name`, else the URL's last path segment without `.git`:
+/// `identity-model` for `https://github.com/acme/identity-model.git`.
+fn git_name(git: &GitSource) -> String {
+    git.name.clone().unwrap_or_else(|| {
+        let url = git.url.trim_end_matches('/');
+        let last = url.rsplit(['/', ':']).next().unwrap_or(url);
+        let stem = last.strip_suffix(".git").unwrap_or(last);
+        if stem.is_empty() { "git" } else { stem }.to_owned()
+    })
+}
+
+/// Checks a git source out at its pinned commit and walks the clone. The
+/// work tree is exactly the commit, so no file is `dirty`.
+fn walk_git(clones: &Clones, git: &GitSource, options: &WalkOptions) -> Result<LocalTree> {
+    let path = clones.checkout(git)?;
+    let checkout = LocalSource {
+        name: git.name.clone(),
+        path,
+        license: git.license.clone(),
+        license_class: git.license_class,
+        include: git.include.clone(),
+        exclude: git.exclude.clone(),
+    };
+    let mut tree = walk_local(&checkout, options)?;
+    for file in &mut tree.files {
+        file.dirty = false;
+    }
+    Ok(tree)
+}
+
+/// A local or git source, walked and ready to write.
+struct Walked<'a> {
+    /// The name results cite.
+    name: String,
+    /// `local` or `git`.
+    kind: &'static str,
+    /// The checkout's root for a local source, the repository URL for a git
+    /// source.
+    url: String,
+    license: Option<&'a str>,
+    license_class: LicenseClass,
+    /// The walk. Its HEAD is the commit results cite: a local checkout's
+    /// HEAD, or a git source's pinned commit.
+    tree: LocalTree,
+}
+
+/// Writes one walked source into the store and returns its ID.
+fn index_tree(
     store: &mut Store,
     chunker: &mut Chunker,
-    local: &LocalSource,
-    name: &str,
-    tree: &LocalTree,
+    walked: &Walked<'_>,
     summary: &mut IndexSummary,
 ) -> Result<i64> {
-    let url = tree.root.to_string_lossy();
+    let (name, tree) = (walked.name.as_str(), &walked.tree);
     let source_id = store.upsert_source(&SourceRecord {
         name,
-        kind: "local",
-        url: &url,
+        kind: walked.kind,
+        url: &walked.url,
         revision: &tree.head,
-        license_spdx: local.license.as_deref(),
-        license_class: local.license_class.as_str(),
+        license_spdx: walked.license,
+        license_class: walked.license_class.as_str(),
         is_current: true,
         obsoleted_by: None,
         updated_by: None,
diff --git a/crates/codecite-index/src/lib.rs b/crates/codecite-index/src/lib.rs
index 3dc0864..4ecabee 100644
--- a/crates/codecite-index/src/lib.rs
+++ b/crates/codecite-index/src/lib.rs
@@ -4,10 +4,12 @@
 //! local checkouts, listing the files git sees and reporting every file it
 //! skips, and splits files into chunks: code at definitions, Markdown at
 //! headings, anything else into 60-line windows. [`Fetcher`] downloads document
-//! sources and checks them against their pinned sha256, caching them by hash.
-//! [`index_corpus`] fetches a corpus's documents, runs the other steps over
-//! its local sources, splits each document into spec sections and steps,
-//! and writes the result into its store.
+//! sources and checks them against their pinned sha256, caching them by hash,
+//! and [`Clones`] keeps a partial clone of each git source's repository.
+//! [`index_corpus`] fetches a corpus's documents and pinned commits, runs the
+//! other steps over its local sources and git sources' checkouts, splits
+//! each document into spec sections and steps, and writes the result into
+//! its store.
 
 mod chunk;
 pub use chunk::{
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
index ea602fb..45ecff4 100644
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -8,7 +8,7 @@ use clap::{Parser, Subcommand};
 use codecite_core::{
     DEFAULT_MAX_LINES, ReadOptions, ReadResult, SearchQuery, SearchResult, Store, is_corpus_name,
 };
-use codecite_index::{Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};
+use codecite_index::{Clones, Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};
 
 /// Command-line arguments.
 #[derive(Parser)]
@@ -144,8 +144,15 @@ fn index(corpus: &str, manifest: Option<PathBuf>, print_hashes: bool) -> anyhow:
     }
     let mut store = Store::open_read_write(&path)
         .with_context(|| format!("opening corpus file {}", path.display()))?;
-    let summary = index_corpus(&mut store, &manifest, &WalkOptions::default(), &fetcher)
-        .with_context(|| format!("indexing corpus `{corpus}`"))?;
+    let clones = Clones::new(dirs.cache_dir().join("repos"));
+    let summary = index_corpus(
+        &mut store,
+        &manifest,
+        &WalkOptions::default(),
+        &fetcher,
+        &clones,
+    )
+    .with_context(|| format!("indexing corpus `{corpus}`"))?;
     println!("indexed corpus `{corpus}` into {}", path.display());
     println!("{summary}");
     Ok(())
@@ -296,8 +303,9 @@ fn print_document_hashes(manifest: &Manifest, fetcher: &Fetcher) -> anyhow::Resu
 }
 
 /// Where codecite keeps its files, never in a repository: corpus files in
-/// the data directory (`~/.local/share/codecite/<corpus>.db` on Linux) and
-/// fetched documents in the cache directory (`~/.cache/codecite/documents`).
+/// the data directory (`~/.local/share/codecite/<corpus>.db` on Linux), and
+/// fetched documents and git sources' clones in the cache directory
+/// (`~/.cache/codecite/documents` and `~/.cache/codecite/repos`).
 fn project_dirs() -> anyhow::Result<directories::ProjectDirs> {
     directories::ProjectDirs::from("", "", "codecite")
         .context("no home directory to keep corpus files in")
`````

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all green; `cargo test` totals 286 passed, 7 ignored.

- [ ] **Step 5: Exercise the binary by hand**

Index a git source from a bare `file://` remote under a temporary `HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME`, read a citation at its commit, then try a commit the remote lacks: the error names the URL and the commit.

- [ ] **Step 6: Index a real repository**

`/var/tmp/gitsrc-proto/real-run.sh` pins `https://github.com/BurntSushi/walkdir` at tag 2.4.0, then at the current HEAD, indexes each, searches and reads at the new commit, and finally pins a commit the remote doesn't have. Expected output (the elapsed times and dates vary):

```text
$ pin 037c5e16ec4d8b3eacb51f077cfdab7a356e8412
$ codecite index walk
indexed corpus `walk` into ~/data/codecite/walk.db
documents: 19 written, 0 unchanged, 0 removed
chunks: 281 written
elapsed: 1.04s
[exit 0]
552K	~/cache/codecite/repos/e6aa7c2b6768355e85cced21d5b2b163772a52e5b7fb72e6373f1a13b9f83676
$ pin 6fd031c82ba5a4204b4ce6eae73dacb00dc072ec
$ codecite index walk  (newer commit)
indexed corpus `walk` into ~/data/codecite/walk.db
documents: 5 written, 15 unchanged, 0 removed
chunks: 71 written
elapsed: 0.44s
[exit 0]
$ codecite index walk  (again)
indexed corpus `walk` into ~/data/codecite/walk.db
documents: 0 written, 20 unchanged, 0 removed
chunks: 0 written
elapsed: 0.14s
[exit 0]
$ codecite search walk "follow symlinks" --limit 3
1. walkdir@6fd031c82ba5a4204b4ce6eae73dacb00dc072ec:src/tests/recursive.rs#L419-L434
   function broken_sym_root_dir_root_is_always_followed · score 9.91 · adapt · indexed 2026-10-20T20:02:42Z
     fn broken_sym_root_dir_root_is_always_followed() {
         let dir = Dir::tmp();
         dir.symlink_dir("broken", "a-link");
2. walkdir@6fd031c82ba5a4204b4ce6eae73dacb00dc072ec:src/tests/recursive.rs#L402-L416
   function broken_sym_root_dir_follow_and_root_nofollow · score 7.68 · adapt · indexed 2026-10-20T20:02:42Z
     fn broken_sym_root_dir_follow_and_root_nofollow() {
         let dir = Dir::tmp();
         dir.symlink_dir("broken", "a-link");
3. walkdir@6fd031c82ba5a4204b4ce6eae73dacb00dc072ec:src/tests/recursive.rs#L1040-L1072
   function same_file_system · score 6.57 · adapt · indexed 2026-10-20T20:02:42Z
     fn same_file_system() {
         use std::path::Path;
     
[exit 0]
$ codecite read walk walkdir@6fd031c:src/lib.rs#L1-L5
walkdir@6fd031c82ba5a4204b4ce6eae73dacb00dc072ec:src/lib.rs#L1-L5
code · adapt · indexed 2026-10-20T20:02:42Z

/*!
Crate `walkdir` provides an efficient and cross platform implementation
of recursive directory traversal. Several options are exposed to control
iteration, such as whether to follow symbolic links (default off), limit the
maximum number of simultaneous open file descriptors and the ability to
[exit 0]
$ pin 0123456789abcdef0123456789abcdef01234567
$ codecite index walk  (commit the remote lacks)
Error: indexing corpus `walk`

Caused by:
    git source https://github.com/BurntSushi/walkdir has no commit 0123456789abcdef0123456789abcdef01234567; check the manifest's `commit`, or push the commit if it only exists locally
[exit 1]
```

The second pin rewrites 5 of 20 files; the third run writes nothing. The clone is about 0.5 MiB.

- [ ] **Step 7: Commit**

```bash
git add crates/codecite-index/src/index.rs crates/codecite-index/src/lib.rs crates/codecite/src/main.rs crates/codecite/tests/index.rs
git commit -m "feat(index): index git sources at their pinned commit (#29)"
```

---

### Task 3: Describe git sources in the design

**Files:**
- Modify: `docs/design.md`, `README.md`

- [ ] **Step 1: Update the Fetch, Walk and Write steps, Freshness and the README status**

Freshness said re-indexing diffs the old commit against the new one. The code compares content hashes instead, which writes the same documents, so the doc changes to match the code.

`````diff
diff --git a/README.md b/README.md
index 91e787b..6f8f032 100644
--- a/README.md
+++ b/README.md
@@ -12,7 +12,8 @@ from memory.
 
 Pre-alpha. `codecite index <corpus>` indexes the local git checkouts a corpus
 manifest (`corpora/<corpus>.toml`) lists into `~/.local/share/codecite/<corpus>.db`
-on Linux, and downloads the documents it lists, checking each against its
+on Linux, and the git repositories it lists at their pinned commits, from a
+partial clone kept in `~/.cache/codecite/repos`, and downloads the documents it lists, checking each against its
 pinned sha256 and caching it in `~/.cache/codecite/documents`, so later runs
 work offline, then indexes their sections and numbered steps.
 `codecite index <corpus> --print-hashes` prints each document's current hash,
diff --git a/docs/design.md b/docs/design.md
index 27cf187..c149458 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -192,11 +192,11 @@ BM25 over code-aware terms does most of the work; vectors and graph signals refi
 
 **Indexing pipeline**
 
-1. **Fetch:** `git clone --filter=blob:none` at the manifest's pinned commit, read a local working tree, or fetch a document by URL and check it against its pinned `sha256`. Every document is fetched before anything is written, so changed bytes stop the run with an error naming the document and both hashes, and the store is untouched until the pin is updated on purpose. Fetched documents are cached outside the repo, in `<cache dir>/codecite/documents/<sha256>` (`~/.cache` on Linux): a document whose pinned hash is in the cache is never fetched again, so indexing works offline after the first run. A cache file whose bytes no longer match its name is deleted and fetched again. A download is plain HTTP(S) GET (`ureq`, rustls with Mozilla's roots, the standard proxy variables), capped at 16 MiB and 60 seconds; the hash, not the transport or an ETag, is what pins the bytes. A document marked `excluded` is never fetched.
-2. **Walk:** list the files git sees (`git ls-files`, so `.gitignore`, `.git/info/exclude` and the global excludes apply and tracked files are never ignored), apply the manifest's `include`/`exclude` globs, and skip deny-listed secrets, symlinks, files over 1 MiB and binaries (a NUL byte in the first 8000 bytes, git's rule; extensions are never trusted). Every skipped path is reported with its reason; an ignored directory counts once. A path under a symlinked directory is skipped as a symlink. The repository's config is not trusted, since it can name programs for git to run: the walk turns `core.fsmonitor` off, allows git no transport (an empty `GIT_ALLOW_PROTOCOL`, so a partial clone never fetches a missing object from its promisor remote), and never runs `git status`; `dirty` compares HEAD's tree, the index and `git hash-object --no-filters` of the kept files only.
+1. **Fetch:** clone a git source and check it out at the manifest's pinned commit, read a local working tree, or fetch a document by URL and check it against its pinned `sha256`. Every document and every pinned commit is fetched before anything is written, so changed bytes stop the run with an error naming the document and both hashes, a commit the repository doesn't have stops it with an error naming the URL and the commit, and the store is untouched until the pin is updated on purpose. A git source's clone lives in `<cache dir>/codecite/repos/<sha256 of the URL>` and is made once, with `git clone --filter=blob:none --no-checkout`: a partial clone holds every commit and tree but fetches a file's contents only when a checkout needs them. A later run fetches the pinned commit by its id (`git fetch origin <commit>`) only when the clone lacks it, so a pinned commit indexes offline after the first run. The clone is then checked out at the commit (`checkout --force --detach`) and cleaned (`clean -ffdx`), so its work tree is exactly the commit; a cache directory that isn't a clone is deleted and cloned again. Git LFS pointers are indexed as they are, never downloaded. Fetched documents are cached outside the repo, in `<cache dir>/codecite/documents/<sha256>` (`~/.cache` on Linux): a document whose pinned hash is in the cache is never fetched again, so indexing works offline after the first run. A cache file whose bytes no longer match its name is deleted and fetched again. A download is plain HTTP(S) GET (`ureq`, rustls with Mozilla's roots, the standard proxy variables), capped at 16 MiB and 60 seconds; the hash, not the transport or an ETag, is what pins the bytes. A document marked `excluded` is never fetched.
+2. **Walk:** list the files git sees (`git ls-files`, so `.gitignore`, `.git/info/exclude` and the global excludes apply and tracked files are never ignored), apply the manifest's `include`/`exclude` globs, and skip deny-listed secrets, symlinks, files over 1 MiB and binaries (a NUL byte in the first 8000 bytes, git's rule; extensions are never trusted). Every skipped path is reported with its reason; an ignored directory counts once. A path under a symlinked directory is skipped as a symlink. The repository's config is not trusted, since it can name programs for git to run: the walk turns `core.fsmonitor` off, allows git no transport (an empty `GIT_ALLOW_PROTOCOL`, so a partial clone never fetches a missing object from its promisor remote), and never runs `git status`; `dirty` compares HEAD's tree, the index and `git hash-object --no-filters` of the kept files only. A git source's checkout is walked the same way, and its files are never `dirty`.
 3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174). Spec HTML comes in three layouts: xml2rfc v3 RFCs (RFC 8650 and later), the older RFC HTML on rfc-editor.org, and the xml2rfc-generated OpenID pages. `scraper` parses it (html5ever repairs malformed HTML as a browser does), and one iterative walk renders it to plain text, whose lines a spec chunk's line range counts: a line per paragraph, list item or table row, `<pre>` kept as written, and page headers, footers and ¶ links dropped. A section runs from its heading to the next heading, numbered or not, so its line range holds its own text and not its subsections'. Front matter, the table of contents, the references and unnumbered parts such as the authors' addresses are not sections. A step is an item of a numbered `<ol>` that sits in no other list, or in an older RFC's plain text a paragraph numbered `1.`, `2.`, … at one indent; it keeps the number the document writes, lists nested in it stay part of its text, and a later list in the same section that reuses a number gives no step for it. Requirement keywords are found in core (`requirement_keywords`), so `cite` can scan a claim the same way: whole uppercase words only, with `MUST NOT`, `SHALL NOT`, `SHOULD NOT` and `NOT RECOMMENDED` read as one keyword across any whitespace. An RFC's "Obsoletes", "Updates", "Obsoleted by" and "Updated by" header fields are read from its HTML (the v3 identifiers block, or the older plain-text header) as `rfc<N>` names. html5ever's parse is quadratic in nesting depth, which is fine for the pinned pages the owner chose but slow for a hostile one.
 4. **Terms:** split camelCase, snake_case and kebab-case identifiers into lowercase parts and keep the original, so `validateToken` matches "validate token" and the exact name. Any character that is not a letter or digit separates parts, an acronym ends before the capital that starts the next word (`HTTPServer` → `http server`), and digits stay with the part before them (`sha256`, `rfc9449`).
-5. **Write:** chunks, full-text rows and symbols for one document in a single transaction. A document whose sha256 matches the stored one is not rewritten; only its `dirty` flag is updated. A document source holds one document, stored under the source's name: a `section` chunk per section and a `step` chunk per step, each step after its section and naming it as its parent, and each with its number, the section's anchor and title, and its requirement keywords as `strength` (`MUST,SHOULD`). A version already stored is never split again, since its revision is its content's hash; a page that isn't UTF-8 or in a known layout stops the run with an error naming the document. A stored document whose file is gone, or is now skipped, is deleted with everything stored for it, and so is every source the run did not index (one removed from the manifest, or marked `excluded`). A chunk's `terms` and its full-text `terms` column hold the identifier terms of every word in its text; the `symbol` column holds its symbol name's terms and the `path` column its path's. A local source without a `name` is named after its checkout directory, and two sources with one name are an error, unless they are versions of one document; a manifest that pins one version twice is refused. The run ends with a summary: documents written, unchanged and removed, chunks written, skipped files by reason, and the elapsed time.
+5. **Write:** chunks, full-text rows and symbols for one document in a single transaction. A document whose sha256 matches the stored one is not rewritten; only its `dirty` flag is updated. A document source holds one document, stored under the source's name: a `section` chunk per section and a `step` chunk per step, each step after its section and naming it as its parent, and each with its number, the section's anchor and title, and its requirement keywords as `strength` (`MUST,SHOULD`). A version already stored is never split again, since its revision is its content's hash; a page that isn't UTF-8 or in a known layout stops the run with an error naming the document. A stored document whose file is gone, or is now skipped, is deleted with everything stored for it, and so is every source the run did not index (one removed from the manifest, or marked `excluded`). A chunk's `terms` and its full-text `terms` column hold the identifier terms of every word in its text; the `symbol` column holds its symbol name's terms and the `path` column its path's. A local source without a `name` is named after its checkout directory, a git source after its URL's last path segment without `.git` (`identity-model` for `https://github.com/acme/identity-model.git`), and two sources with one name are an error, unless they are versions of one document; a manifest that pins one version twice is refused. The run ends with a summary: documents written, unchanged and removed, chunks written, skipped files by reason, and the elapsed time.
 6. **Providers:** SCIP, CodeGraph and Graphify run after the base pass and add symbols and edges.
 7. **Embed (optional):** a small local model through `fastembed`, 384 dimensions, CPU only.
 
@@ -211,7 +211,7 @@ BM25 over code-aware terms does most of the work; vectors and graph signals refi
 
 **Freshness**
 
-- Each document stores a content hash. Re-indexing a git source diffs the old commit against the new one and rewrites only changed documents.
+- Each document stores a content hash. Re-indexing a git source at a new commit checks the new commit out and rewrites only the documents whose content hash changed, which are the files that changed between the two commits; a file the new commit deletes is removed.
 - Each corpus has a maximum age (default 30 days). An index run stores it in `meta.max_age_days` beside `meta.indexed_at`, so a query needs no manifest. Older indexes are refused unless the caller passes `--allow-stale`, and every result carries its source's `indexed_at`.
 
 **Initial targets, to validate in milestone 4:** index 100k lines of code in under 60 seconds without SCIP on a laptop, and answer BM25 queries over 1 million chunks in under 100 ms at p95.
`````

- [ ] **Step 2: Commit**

```bash
git add docs/design.md README.md
git commit -m "docs: describe git sources and their clone cache (#29)"
```
