//! Git sources: one partial clone per repository URL, kept in the cache,
//! fetched until it holds the manifest's pinned commit and checked out at it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::GitSource;
use crate::fetch::hex;
use crate::manifest::is_lower_hex;

/// Keeps a partial clone of each git source's repository in a cache
/// directory and checks it out at the pinned commit.
///
/// A clone is made with `--filter=blob:none`: it holds every commit and tree
/// but fetches a file's contents only when a checkout needs them, so pinning
/// a large repository costs the files at the pinned commit, not its whole
/// history. A clone is never made twice: when the pinned commit moves, the
/// clone is fetched, and when the clone already has the commit, nothing is
/// fetched at all, so indexing works offline after the first run.
///
/// Every change to a clone happens under a lock on the file next to it
/// (`<clone>.lock`), so index runs in other processes that share a
/// repository never change one clone at the same time.
#[derive(Debug, Clone)]
pub struct Clones {
    dir: PathBuf,
}

/// A clone checked out at a pinned commit, holding the clone's lock so no
/// other run checks out another commit while this one reads the files.
/// Dropping it releases the lock.
#[derive(Debug)]
pub struct Checkout {
    path: PathBuf,
    // Never read: the open file holds the lock until it is closed.
    _lock: fs::File,
}

impl Checkout {
    /// The clone's work tree, exactly the pinned commit.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Why a git source could not be cloned, fetched or checked out.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CloneError {
    /// The pinned `commit` is not 40 lowercase hex digits. [`crate::Manifest::parse`]
    /// rejects these; this guards a [`GitSource`] built any other way, since
    /// the commit becomes an argument to git.
    #[error("git source {url} pins commit {commit:?}, which is not 40 lowercase hex digits")]
    BadPin {
        /// The repository URL.
        url: String,
        /// The pin as given.
        commit: String,
    },

    /// The repository has no commit with the pinned id, even after fetching.
    #[error(
        "git source {url} has no commit {commit}; check the manifest's `commit`, \
         or push the commit if it only exists locally"
    )]
    CommitNotFound {
        /// The repository URL.
        url: String,
        /// The pinned commit.
        commit: String,
    },

    /// `git` could not be started (not installed, or not on `PATH`).
    #[error("could not run git")]
    GitSpawn {
        /// Why the process did not start.
        source: io::Error,
    },

    /// A `git` command exited with an error.
    #[error("`{command}` failed for git source {url}: {stderr}")]
    Git {
        /// The repository URL.
        url: String,
        /// The command line, without `-C <dir>`.
        command: String,
        /// What git printed on stderr, trimmed.
        stderr: String,
    },

    /// Creating or removing a directory in the clone cache failed.
    #[error("clone cache {}", path.display())]
    Cache {
        /// The directory.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

impl Clones {
    /// Clones kept under `dir`, created on first use.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Clones { dir: dir.into() }
    }

    /// The clone of `url`: a directory named by the URL's sha256, so any URL
    /// is a safe directory name and two URLs never share a clone.
    pub fn path(&self, url: &str) -> PathBuf {
        self.dir.join(hex(&Sha256::digest(url)))
    }

    /// Makes sure the clone of `git.url` holds `git.commit`: clones the
    /// repository when there is no clone yet, else fetches when the commit
    /// is missing. Returns the clone's directory.
    ///
    /// A commit the remote doesn't have is [`CloneError::CommitNotFound`].
    /// Waits while another run holds the clone's lock.
    pub fn fetch(&self, git: &GitSource) -> Result<PathBuf, CloneError> {
        let _lock = self.lock(git)?;
        self.fetch_locked(git)
    }

    /// Checks out `git.commit` in its clone, after fetching it as
    /// [`Clones::fetch`] does, and removes every file the commit doesn't
    /// track, so the work tree is exactly the commit. Fetches the files'
    /// contents that the clone doesn't have yet.
    ///
    /// The returned [`Checkout`] holds the clone's lock: keep it until the
    /// last file is read.
    pub fn checkout(&self, git: &GitSource) -> Result<Checkout, CloneError> {
        let lock = self.lock(git)?;
        let path = self.fetch_locked(git)?;
        run(
            &path,
            git,
            &["checkout", "--quiet", "--force", "--detach", &git.commit],
        )?;
        run(&path, git, &["clean", "--quiet", "-ffdx"])?;
        Ok(Checkout { path, _lock: lock })
    }

    /// Takes the lock on `git.url`'s clone, waiting while another run holds
    /// it. The lock is the open file: closing it, on drop or when the
    /// process exits, releases it.
    fn lock(&self, git: &GitSource) -> Result<fs::File, CloneError> {
        if !is_lower_hex(&git.commit, 40) {
            return Err(CloneError::BadPin {
                url: git.url.clone(),
                commit: git.commit.clone(),
            });
        }
        let mut path = self.path(&git.url).into_os_string();
        path.push(".lock");
        let path = PathBuf::from(path);
        let cache_error = |source| CloneError::Cache {
            path: path.clone(),
            source,
        };
        fs::create_dir_all(&self.dir).map_err(cache_error)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(cache_error)?;
        file.lock().map_err(cache_error)?;
        Ok(file)
    }

    /// [`Clones::fetch`], with the clone's lock already held.
    fn fetch_locked(&self, git: &GitSource) -> Result<PathBuf, CloneError> {
        let path = self.path(&git.url);
        if !self.is_clone(&path, git)? {
            self.clone_into(&path, git)?;
        }
        if !has_commit(&path, git)? {
            // A server can refuse a commit no branch or tag reaches; then
            // the commit is not there to pin.
            let fetched = run(
                &path,
                git,
                &[
                    "fetch",
                    "--quiet",
                    "--filter=blob:none",
                    "origin",
                    &git.commit,
                ],
            );
            match fetched {
                Ok(_) => {}
                Err(CloneError::Git { ref stderr, .. }) if refused(stderr) => {}
                Err(e) => return Err(e),
            }
            if !has_commit(&path, git)? {
                return Err(CloneError::CommitNotFound {
                    url: git.url.clone(),
                    commit: git.commit.clone(),
                });
            }
        }
        Ok(path)
    }

    /// Whether `path` is a clone. A directory that isn't one, left by a
    /// clone that was cut short, is deleted.
    fn is_clone(&self, path: &Path, git: &GitSource) -> Result<bool, CloneError> {
        if !path.exists() {
            return Ok(false);
        }
        match run(path, git, &["rev-parse", "--show-toplevel"]) {
            Ok(_) => return Ok(true),
            Err(CloneError::Git { .. }) => {}
            Err(e) => return Err(e),
        }
        fs::remove_dir_all(path).map_err(|source| CloneError::Cache {
            path: path.to_owned(),
            source,
        })?;
        Ok(false)
    }

    /// Clones `git.url` into `path` without checking anything out. The clone
    /// is made in a temporary directory and renamed into place, so a clone
    /// cut short never looks like a finished one.
    fn clone_into(&self, path: &Path, git: &GitSource) -> Result<(), CloneError> {
        let cache_error = |source| CloneError::Cache {
            path: self.dir.clone(),
            source,
        };
        fs::create_dir_all(&self.dir).map_err(cache_error)?;
        let tmp = tempfile::Builder::new()
            .prefix(".clone-")
            .tempdir_in(&self.dir)
            .map_err(cache_error)?;
        run(
            tmp.path(),
            git,
            &[
                "clone",
                "--quiet",
                "--no-checkout",
                "--filter=blob:none",
                "--",
                &git.url,
                ".",
            ],
        )?;
        fs::rename(tmp.keep(), path).map_err(cache_error)
    }
}

/// Whether git's `stderr` from a fetch says the remote has no such commit,
/// or won't serve it, rather than that the remote couldn't be reached.
fn refused(stderr: &str) -> bool {
    [
        "not our ref",
        "couldn't find remote ref",
        "no such remote ref",
    ]
    .iter()
    .any(|refusal| stderr.contains(refusal))
}

/// Whether the clone at `path` holds `git.commit` as a commit.
fn has_commit(path: &Path, git: &GitSource) -> Result<bool, CloneError> {
    let object = format!("{}^{{commit}}", git.commit);
    match run(path, git, &["cat-file", "-e", &object]) {
        Ok(_) => Ok(true),
        Err(CloneError::Git { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Runs `git -C <dir> <args>` and returns its stdout.
///
/// The clone is codecite's own, so unlike a local source's checkout its
/// config is trusted. Git still looks no higher than the cache for a
/// repository, so a missing clone is never mistaken for an enclosing one,
/// and Git LFS, if installed, leaves pointer files as they are rather than
/// downloading what they point to.
fn run(dir: &Path, git: &GitSource, args: &[&str]) -> Result<Vec<u8>, CloneError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        // `-C` picks the repository; an inherited GIT_DIR would override it.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    if let Some(parent) = dir.parent() {
        command.env("GIT_CEILING_DIRECTORIES", parent);
    }
    let output = command
        .output()
        .map_err(|source| CloneError::GitSpawn { source })?;
    if !output.status.success() {
        return Err(CloneError::Git {
            url: git.url.clone(),
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LicenseClass;
    use crate::test_repo::{Remote, Repo};

    fn source(remote: &Remote, commit: &str) -> GitSource {
        GitSource {
            name: None,
            url: remote.url(),
            commit: commit.into(),
            license: None,
            license_class: LicenseClass::Adapt,
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }

    fn git_out(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    }

    /// A remote with one commit holding `a.rs`, and a clone cache.
    fn setup() -> (Repo, Remote, String, tempfile::TempDir) {
        let repo = Repo::new();
        repo.write("a.rs", "fn a() {}\n");
        repo.commit_all(&["a.rs"]);
        let remote = Remote::new();
        let commit = repo.push(&remote, "main");
        (repo, remote, commit, tempfile::tempdir().unwrap())
    }

    #[test]
    fn checkout_makes_a_partial_clone_at_the_commit() {
        let (_repo, remote, commit, cache) = setup();
        let clones = Clones::new(cache.path().join("repos"));
        let checkout = clones.checkout(&source(&remote, &commit)).unwrap();
        let path = checkout.path();

        assert_eq!(path, clones.path(&remote.url()));
        assert_eq!(git_out(path, &["rev-parse", "HEAD"]), commit);
        assert_eq!(
            git_out(path, &["config", "remote.origin.partialclonefilter"]),
            "blob:none"
        );
        assert_eq!(
            fs::read_to_string(path.join("a.rs")).unwrap(),
            "fn a() {}\n"
        );
        // Only the clone and its lock are left in the cache, no temporary
        // directory.
        let mut entries: Vec<_> = fs::read_dir(cache.path().join("repos"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        entries.sort();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(entries, [name.to_owned(), format!("{name}.lock")]);
    }

    #[test]
    fn a_new_commit_is_fetched_into_the_same_clone() {
        let (repo, remote, first, cache) = setup();
        let clones = Clones::new(cache.path());
        let path = clones
            .checkout(&source(&remote, &first))
            .unwrap()
            .path()
            .to_owned();
        // A marker in the clone's git directory survives a fetch, not a re-clone.
        fs::write(path.join(".git/marker"), "").unwrap();

        repo.write("b.rs", "fn b() {}\n");
        repo.commit_all(&["b.rs"]);
        // A side branch: fetching by id must reach commits no clone saw.
        let second = repo.push(&remote, "side");
        assert_eq!(
            clones.checkout(&source(&remote, &second)).unwrap().path(),
            path
        );
        assert!(path.join(".git/marker").exists());
        assert_eq!(git_out(&path, &["rev-parse", "HEAD"]), second);
        assert!(path.join("b.rs").exists());

        // Back to the first commit: b.rs goes, and so does a stray file.
        fs::write(path.join("stray.txt"), "left behind\n").unwrap();
        clones.checkout(&source(&remote, &first)).unwrap();
        assert!(!path.join("b.rs").exists());
        assert!(!path.join("stray.txt").exists());
    }

    #[test]
    fn a_clone_with_the_commit_needs_no_remote() {
        let (_repo, remote, commit, cache) = setup();
        let clones = Clones::new(cache.path());
        let git = source(&remote, &commit);
        clones.checkout(&git).unwrap();

        drop(remote);
        let checkout = clones.checkout(&git).unwrap();
        assert_eq!(git_out(checkout.path(), &["rev-parse", "HEAD"]), commit);
    }

    #[test]
    fn a_commit_the_remote_lacks_is_a_typed_error() {
        let (_repo, remote, _commit, cache) = setup();
        let clones = Clones::new(cache.path());
        let missing = "0123456789abcdef0123456789abcdef01234567";
        let err = clones.fetch(&source(&remote, missing)).unwrap_err();
        assert!(
            matches!(&err, CloneError::CommitNotFound { url, commit }
                if *url == remote.url() && commit == missing),
            "{err:?}"
        );
        assert!(
            err.to_string().contains("has no commit 0123456789ab"),
            "{err}"
        );
    }

    #[test]
    fn a_pin_that_is_not_a_commit_id_never_reaches_git() {
        let (_repo, remote, _commit, cache) = setup();
        let clones = Clones::new(cache.path().join("repos"));
        for pin in ["--upload-pack=touch /tmp/x", "HEAD", "0123456"] {
            let err = clones.fetch(&source(&remote, pin)).unwrap_err();
            assert!(matches!(err, CloneError::BadPin { .. }), "{err:?}");
        }
        assert!(!cache.path().join("repos").exists());
    }

    #[test]
    fn a_broken_clone_is_replaced() {
        let (_repo, remote, commit, cache) = setup();
        let clones = Clones::new(cache.path());
        let path = clones.path(&remote.url());
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("half"), "").unwrap();

        clones.checkout(&source(&remote, &commit)).unwrap();
        assert!(!path.join("half").exists());
        assert_eq!(git_out(&path, &["rev-parse", "HEAD"]), commit);
    }

    #[test]
    fn a_checkout_keeps_other_runs_off_the_clone_until_dropped() {
        let (repo, remote, first, cache) = setup();
        repo.write("a.rs", "fn a2() {}\n");
        repo.commit_all(&["a.rs"]);
        let second = repo.push(&remote, "side");
        let clones = Clones::new(cache.path());
        let held = clones.checkout(&source(&remote, &first)).unwrap();

        // Another run, as another process would, waits for the lock rather
        // than switching the clone to its own commit under this one.
        let (done_tx, done) = std::sync::mpsc::channel();
        let other = {
            let (clones, git) = (clones.clone(), source(&remote, &second));
            std::thread::spawn(move || {
                let checkout = clones.checkout(&git).unwrap();
                done_tx.send(()).unwrap();
                git_out(checkout.path(), &["rev-parse", "HEAD"])
            })
        };
        assert!(
            done.recv_timeout(std::time::Duration::from_millis(500))
                .is_err(),
            "the other checkout ran while this one was held"
        );
        assert_eq!(git_out(held.path(), &["rev-parse", "HEAD"]), first);
        assert_eq!(
            fs::read_to_string(held.path().join("a.rs")).unwrap(),
            "fn a() {}\n"
        );

        drop(held);
        assert_eq!(other.join().unwrap(), second);
    }

    #[test]
    fn a_fetch_that_cannot_reach_the_remote_keeps_gits_error() {
        let (repo, remote, first, cache) = setup();
        let clones = Clones::new(cache.path());
        clones.fetch(&source(&remote, &first)).unwrap();
        repo.write("b.rs", "fn b() {}\n");
        repo.commit_all(&["b.rs"]);
        let second = repo.push(&remote, "main");
        let git = source(&remote, &second);

        // The remote is gone: that is not the commit missing from it.
        drop(remote);
        let err = clones.fetch(&git).unwrap_err();
        assert!(
            matches!(&err, CloneError::Git { command, .. } if command.starts_with("git fetch")),
            "{err:?}"
        );
    }
}
