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
        return Err(WalkError::NotRepoRoot {
            path: root,
            toplevel,
        });
    }
    match run(
        &root,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    ) {
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
    let out = run(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
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
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
        ],
    )?;
    let mut paths: Vec<_> = split_paths(&out).collect();
    paths.sort();
    Ok(paths)
}

/// Paths that differ from HEAD: modified, staged, deleted or untracked.
pub(super) fn dirty(root: &Path) -> Result<HashSet<String>, WalkError> {
    let out = run(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--no-renames",
        ],
    )?;
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
        assert!(
            err.to_string().contains("not the top of its git work tree"),
            "{err}"
        );
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
