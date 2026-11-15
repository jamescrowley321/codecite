//! Throwaway git repositories for the walk's, the clone cache's and the
//! index's tests: a work tree to commit in and a bare remote to push to.

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
            .args([
                "-c",
                "user.name=codecite",
                "-c",
                "user.email=codecite@example.com",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            // Under a git hook these point at the developer's own repository.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
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

    /// Pushes HEAD to `branch` on `remote` and returns HEAD's commit.
    pub(crate) fn push(&self, remote: &Remote, branch: &str) -> String {
        self.git(&[
            "push",
            "-q",
            &remote.url(),
            &format!("HEAD:refs/heads/{branch}"),
        ]);
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_owned()
    }
}

/// A bare repository in a temporary directory, served to clones over
/// `file://` so they behave as they would over the network: a plain path
/// would make a local clone, which ignores `--filter`.
pub(crate) struct Remote {
    repo: Repo,
}

impl Remote {
    /// `git init --bare`, allowing partial clones.
    pub(crate) fn new() -> Self {
        let repo = Repo {
            dir: tempfile::tempdir().unwrap(),
        };
        repo.git(&["init", "-q", "--bare"]);
        repo.git(&["config", "uploadpack.allowFilter", "true"]);
        Remote { repo }
    }

    /// Points the remote's `HEAD`, its default branch, at `branch`.
    pub(crate) fn set_head(&self, branch: &str) {
        self.repo
            .git(&["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")]);
    }

    /// The remote's `file://` URL.
    pub(crate) fn url(&self) -> String {
        format!("file://{}", self.repo.path().display())
    }
}
