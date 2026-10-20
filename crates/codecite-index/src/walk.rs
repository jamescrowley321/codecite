//! Walking a local source: list the checkout's files the way git sees them,
//! skip what must not be indexed, and report every skip with its reason.

mod git;
#[cfg(test)]
mod test_repo;

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::LocalSource;

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
    #[error(
        "{path}: not the top of its git work tree ({toplevel}); point `path` at the checkout's root"
    )]
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

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// How many leading bytes the binary sniff reads: git's own limit.
const SNIFF_LEN: usize = 8000;

/// File-name endings of private keys and certificates.
const DENIED_EXTENSIONS: [&str; 9] = [
    ".pem",
    ".key",
    ".p12",
    ".pfx",
    ".crt",
    ".cer",
    ".der",
    ".jks",
    ".keystore",
];

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
#[derive(Debug)]
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
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut real_dirs = HashSet::new();
    for path in git::files(&root)? {
        let path = match path {
            Ok(path) => path,
            Err(lossy) => {
                skipped.push(SkippedFile {
                    path: lossy,
                    reason: SkipReason::NonUtf8Path,
                });
                continue;
            }
        };
        match check(&root, &path, &filter, options, &mut real_dirs)? {
            Check::Keep { size } => files.push(LocalFile {
                path,
                size,
                dirty: true,
            }),
            Check::Skip(reason) => skipped.push(SkippedFile { path, reason }),
        }
    }
    mark_dirty(&root, &mut files)?;
    for path in git::ignored(&root)? {
        let path = path.unwrap_or_else(|lossy| lossy);
        skipped.push(SkippedFile {
            path,
            reason: SkipReason::Ignored,
        });
    }
    Ok(LocalTree {
        root,
        head,
        files,
        skipped,
    })
}

/// Sets `dirty` on each kept file: false only when HEAD has the file and both
/// the index and the work tree hold HEAD's blob. Only kept files are hashed,
/// so git never opens a deny-listed one.
fn mark_dirty(root: &Path, files: &mut [LocalFile]) -> Result<(), WalkError> {
    let head = git::head_blobs(root)?;
    let index = git::index_blobs(root)?;
    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let work = git::hash_files(root, &paths)?;
    // A file git printed no id for keeps `dirty: true`.
    for (file, work) in files.iter_mut().zip(&work) {
        let committed = head.get(&file.path);
        file.dirty = committed != Some(work) || index.get(&file.path) != committed;
    }
    Ok(())
}

/// What [`check`] decided for one path.
enum Check {
    Keep { size: u64 },
    Skip(SkipReason),
}

/// Runs the per-path checks in order; the first that fails is the reason.
fn check(
    root: &Path,
    path: &str,
    filter: &PathFilter,
    options: &WalkOptions,
    real_dirs: &mut HashSet<String>,
) -> Result<Check, WalkError> {
    if !filter.allows(path) {
        return Ok(Check::Skip(SkipReason::Excluded));
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    if is_deny_listed(name) {
        return Ok(Check::Skip(SkipReason::DenyListed));
    }
    if let Some(reason) = check_ancestors(root, path, real_dirs)? {
        return Ok(Check::Skip(reason));
    }
    let full = root.join(path);
    let io_error = |source| WalkError::Io {
        path: full.clone(),
        source,
    };
    let meta = match fs::symlink_metadata(&full) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Check::Skip(SkipReason::Deleted));
        }
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

/// Checks each directory above `path`, so a tracked directory swapped for a
/// symlink can't lead the walk out of the checkout. `real_dirs` holds the
/// directories already found to be real ones.
fn check_ancestors(
    root: &Path,
    path: &str,
    real_dirs: &mut HashSet<String>,
) -> Result<Option<SkipReason>, WalkError> {
    for (end, _) in path.match_indices('/') {
        let dir = &path[..end];
        if real_dirs.contains(dir) {
            continue;
        }
        let full = root.join(dir);
        let meta = match fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Some(SkipReason::Deleted));
            }
            Err(source) => return Err(WalkError::Io { path: full, source }),
        };
        if meta.file_type().is_symlink() {
            return Ok(Some(SkipReason::Symlink));
        }
        if !meta.is_dir() {
            // The directory was replaced by a file, so `path` is gone.
            return Ok(Some(SkipReason::Deleted));
        }
        real_dirs.insert(dir.to_owned());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_list() {
        for name in [
            ".env",
            ".env.local",
            ".envrc",
            ".env.example",
            "server.pem",
            "SERVER.PEM",
            "tls.key",
            "bundle.p12",
            "store.pfx",
            "ca.crt",
            "ca.cer",
            "key.der",
            "app.jks",
            "release.keystore",
            "id_rsa",
            "id_ed25519",
            "id_ecdsa_sk.pub",
        ] {
            assert!(is_deny_listed(name), "{name} should be deny-listed");
        }
        for name in [
            "id_token.rs",
            "id_token_logic.py",
            "id_",
            "keys.rs",
            "pem.rs",
            "environment.ts",
            "my.env",
            "monkey.txt",
            "README.md",
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
        assert!(
            matches!(err, WalkError::Glob { ref pattern, .. } if pattern == "src/["),
            "{err:?}"
        );
        assert_eq!(err.to_string(), "invalid glob \"src/[\"");
    }

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
        tree.files
            .iter()
            .map(|f| (f.path.as_str(), f.dirty))
            .collect()
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

        let reason = |path: &str| {
            tree.skipped
                .iter()
                .find(|s| s.path == path)
                .map(|s| s.reason)
        };
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
        assert_eq!(
            tree.skip_counts(),
            BTreeMap::from([(SkipReason::Excluded, 2)])
        );
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
                SkippedFile {
                    path: "gone.rs".into(),
                    reason: SkipReason::Deleted
                },
                SkippedFile {
                    path: "medium.rs".into(),
                    reason: SkipReason::Oversized
                },
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_skips_paths_under_a_symlinked_directory() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("credentials"), "secret\n").unwrap();
        let repo = Repo::new();
        repo.write("src/credentials", "tracked\n");
        repo.write("src/lib.rs", "lib\n");
        repo.commit_all(&["."]);
        // The tracked directory is swapped for a link that leaves the checkout.
        std::fs::remove_dir_all(repo.path().join("src")).unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.path().join("src")).unwrap();

        let tree = walk_local(&local(&repo), &WalkOptions::default()).unwrap();
        assert_eq!(paths(&tree), []);
        // The two tracked paths under the link, and the untracked link itself.
        let skipped: Vec<_> = tree
            .skipped
            .iter()
            .map(|s| (s.path.as_str(), s.reason))
            .collect();
        assert_eq!(
            skipped,
            [
                ("src", SkipReason::Symlink),
                ("src/credentials", SkipReason::Symlink),
                ("src/lib.rs", SkipReason::Symlink),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_never_runs_commands_from_the_repository_config() {
        use std::os::unix::fs::PermissionsExt;

        let repo = Repo::new();
        repo.write(".gitattributes", "* filter=probe\n");
        repo.write("a.rs", "a\n");
        repo.write(".env", "SECRET=1\n");
        repo.commit_all(&["."]);

        let marker = repo.path().join(".git/ran");
        let hook = repo.path().join(".git/probe.sh");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let hook = hook.to_str().unwrap();
        repo.git(&["config", "core.fsmonitor", hook]);
        repo.git(&["config", "filter.probe.clean", hook]);
        // New contents and sizes, so git must look inside to tell them apart.
        repo.write("a.rs", "changed\n");
        repo.write(".env", "SECRET=2\n");

        let tree = walk_local(&local(&repo), &WalkOptions::default()).unwrap();
        assert_eq!(paths(&tree), [(".gitattributes", false), ("a.rs", true)]);
        assert!(
            !marker.exists(),
            "git ran a command from the repository's config"
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_never_fetches_missing_objects_from_a_promisor_remote() {
        use std::os::unix::fs::PermissionsExt;

        let origin = Repo::new();
        origin.write("a.rs", "a\n");
        origin.commit_all(&["."]);
        origin.git(&["config", "uploadpack.allowFilter", "true"]);

        // A treeless partial clone: HEAD's commit is here, its tree is not.
        let clone = tempfile::tempdir().unwrap();
        let clone_path = clone.path().join("clone");
        let clone_str = clone_path.to_str().unwrap();
        let url = format!("file://{}", origin.path().display());
        origin.git(&[
            "clone",
            "-q",
            "--no-checkout",
            "--filter=tree:0",
            &url,
            clone_str,
        ]);

        let marker = clone.path().join("ran");
        let probe = clone.path().join("probe.sh");
        std::fs::write(
            &probe,
            format!(
                "#!/bin/sh\ntouch '{}'\nexec git-upload-pack \"$@\"\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let probe = probe.to_str().unwrap();
        origin.git(&["-C", clone_str, "config", "remote.origin.uploadpack", probe]);
        origin.git(&["-C", clone_str, "config", "protocol.allow", "always"]);

        let mut source = local(&origin);
        source.path = clone_path;
        let err = walk_local(&source, &WalkOptions::default()).unwrap_err();
        assert!(matches!(err, WalkError::Git { .. }), "{err:?}");
        assert!(
            !marker.exists(),
            "git fetched from the promisor remote and ran its upload-pack"
        );
    }

    #[test]
    fn walk_marks_staged_and_reverted_files_dirty() {
        let repo = Repo::new();
        repo.write("clean.rs", "c\n");
        repo.write("staged.rs", "s\n");
        repo.write("reverted.rs", "r\n");
        repo.commit_all(&["."]);
        repo.write("staged.rs", "s2\n");
        repo.write("reverted.rs", "r2\n");
        repo.git(&["add", "staged.rs", "reverted.rs"]);
        // Staged a change, then put HEAD's contents back in the work tree.
        repo.write("reverted.rs", "r\n");

        let tree = walk_local(&local(&repo), &WalkOptions::default()).unwrap();
        assert_eq!(
            paths(&tree),
            [
                ("clean.rs", false),
                ("reverted.rs", true),
                ("staged.rs", true)
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
}
