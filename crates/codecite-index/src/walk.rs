//! Walking a local source: list the checkout's files the way git sees them,
//! skip what must not be indexed, and report every skip with its reason.

// The helpers have no caller outside tests until `walk_local` lands.
#![allow(dead_code)]

mod git;
#[cfg(test)]
mod test_repo;

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
}
