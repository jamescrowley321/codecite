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
}
