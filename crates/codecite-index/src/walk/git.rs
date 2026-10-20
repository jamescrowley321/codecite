//! The `git` commands the walk runs against a local checkout.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::WalkError;

/// Runs `git -C <root> <args>` and returns its stdout.
///
/// The repository's config is not trusted: it can name programs for git to
/// run (`core.fsmonitor`, clean filters). The walk runs only commands that
/// read the index, the object store and directory listings, never `status`
/// or `diff`, and turns the file-system monitor off for each of them.
///
/// The walk never touches the network. A partial clone fetches missing
/// objects from its promisor remote on demand, and the remote's config can
/// name the command that serves them (`remote.<name>.uploadpack`,
/// `core.sshCommand`, an `ext::` URL). An empty `GIT_ALLOW_PROTOCOL` allows
/// no transport at all and overrides any `protocol.*.allow` in the config,
/// so a missing object fails the command instead. `GIT_NO_LAZY_FETCH`
/// says the same thing to git 2.44 and later.
pub(super) fn run(root: &Path, args: &[&str]) -> Result<Vec<u8>, WalkError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        // Read-only: don't let `status` refresh and rewrite the index.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_ALLOW_PROTOCOL", "")
        .env("GIT_NO_LAZY_FETCH", "1")
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

/// Each file in HEAD's tree with its blob id.
pub(super) fn head_blobs(root: &Path) -> Result<HashMap<String, String>, WalkError> {
    let out = run(root, &["ls-tree", "-r", "-z", "--full-tree", "HEAD"])?;
    // Each record is `<mode> <type> <id>\t<path>`.
    Ok(split_paths(&out)
        .filter_map(Result::ok)
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let id = meta.rsplit(' ').next()?;
            Some((path.to_owned(), id.to_owned()))
        })
        .collect())
}

/// Each path staged in the index with its blob id. A path with a merge
/// conflict has no stage-0 entry, so it is absent.
pub(super) fn index_blobs(root: &Path) -> Result<HashMap<String, String>, WalkError> {
    let out = run(root, &["ls-files", "-z", "--stage"])?;
    // Each record is `<mode> <id> <stage>\t<path>`.
    Ok(split_paths(&out)
        .filter_map(Result::ok)
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let mut fields = meta.split(' ');
            let id = fields.nth(1)?;
            (fields.next()? == "0").then(|| (path.to_owned(), id.to_owned()))
        })
        .collect())
}

/// How many paths go on one `hash-object` command line.
const HASH_BATCH: usize = 500;

/// The blob id of each work-tree file in `paths`, in order, hashed as stored
/// on disk (`--no-filters`, so no clean filter runs). Only these paths are
/// opened.
pub(super) fn hash_files(root: &Path, paths: &[&str]) -> Result<Vec<String>, WalkError> {
    let mut ids = Vec::with_capacity(paths.len());
    for batch in paths.chunks(HASH_BATCH) {
        let mut args = vec!["hash-object", "--no-filters", "--"];
        args.extend_from_slice(batch);
        let out = run(root, &args)?;
        ids.extend(String::from_utf8_lossy(&out).lines().map(str::to_owned));
    }
    Ok(ids)
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
    fn blob_ids_from_head_the_index_and_the_work_tree() {
        let repo = Repo::new();
        repo.write("a.rs", "a\n");
        repo.write("dir/b.rs", "b\n");
        repo.commit_all(&["."]);
        repo.write("a.rs", "a2\n");
        repo.git(&["add", "a.rs"]);
        repo.write("a.rs", "a3\n");

        let head = head_blobs(repo.path()).unwrap();
        let index = index_blobs(repo.path()).unwrap();
        let work = hash_files(repo.path(), &["a.rs", "dir/b.rs"]).unwrap();
        let id = |rev: &str| {
            let out = run(repo.path(), &["rev-parse", rev]).unwrap();
            String::from_utf8(out).unwrap().trim_end().to_owned()
        };
        assert_eq!(head.len(), 2);
        assert_eq!(head["a.rs"], id("HEAD:a.rs"));
        assert_eq!(head["dir/b.rs"], id("HEAD:dir/b.rs"));
        assert_eq!(index["a.rs"], id(":a.rs"));
        assert_ne!(index["a.rs"], head["a.rs"]);
        assert_eq!(work.len(), 2);
        assert!(work[0] != index["a.rs"] && work[0] != head["a.rs"]);
        assert_eq!(work[1], head["dir/b.rs"]);
    }
}
