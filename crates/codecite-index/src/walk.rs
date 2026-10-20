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
