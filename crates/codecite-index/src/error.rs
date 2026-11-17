//! Errors returned by the index pipeline.

/// An error from `codecite-index`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The store or query engine failed.
    #[error(transparent)]
    Core(#[from] codecite_core::Error),

    /// An index run failed, and so did rolling it back or recording it as
    /// failed. The run's own error is the source.
    #[error(
        "the index run failed, and so did recording it: {}",
        crate::error::error_chain(recording)
    )]
    RunNotRecorded {
        /// Why the run failed.
        #[source]
        run: Box<Error>,
        /// Why rolling it back or recording it failed.
        recording: codecite_core::Error,
    },

    /// Reading or writing a file in the corpus failed.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// A corpus manifest is invalid.
    #[error(transparent)]
    Manifest(#[from] crate::ManifestError),

    /// A local source could not be walked.
    #[error(transparent)]
    Walk(#[from] crate::WalkError),

    /// A bundled tree-sitter grammar or query failed to load.
    #[error(transparent)]
    Chunker(#[from] crate::ChunkerError),

    /// A document source could not be fetched, or its bytes don't match
    /// the manifest's pinned `sha256`.
    #[error(transparent)]
    Fetch(#[from] crate::FetchError),

    /// A git source could not be cloned, fetched or checked out, or its
    /// repository has no commit with the pinned id.
    #[error(transparent)]
    Clone(#[from] crate::CloneError),

    /// A walked file could not be read.
    #[error("reading {}", path.display())]
    ReadFile {
        /// The file's full path.
        path: std::path::PathBuf,
        /// Why reading failed.
        source: std::io::Error,
    },

    /// Two sources in the manifest have the same name, and they aren't
    /// versions of one document.
    #[error("two sources are named `{name}`; give one of them a different `name`")]
    DuplicateSource {
        /// The shared name.
        name: String,
    },

    /// One alias names two documents, so a citation using it would be a
    /// guess.
    #[error("alias {alias:?} names both `{first}` and `{second}`; keep it on one")]
    DuplicateAlias {
        /// The alias as the manifest writes it.
        alias: String,
        /// The document it names first.
        first: String,
        /// The other document it names.
        second: String,
    },

    /// A document source's bytes are not UTF-8, so they can't be HTML a
    /// spec is split from.
    #[error("document `{name}` is not UTF-8 text")]
    DocumentNotUtf8 {
        /// The document's name.
        name: String,
    },

    /// A guidance document can't be split into rules, or a rule's scope
    /// comment is invalid.
    #[error("guidance `{name}` can't be split into rules")]
    Guidance {
        /// The guidance source's name.
        name: String,
        /// Why splitting failed.
        source: crate::GuidanceError,
    },

    /// A guidance file read from `path` no longer has the hash its `sha256`
    /// pins, so a rule changed that nobody pinned.
    #[error(
        "guidance `{name}`: {} has sha256 {actual}, not the pinned {expected}; review the change and update the pin",
        path.display()
    )]
    GuidanceChanged {
        /// The guidance source's name.
        name: String,
        /// The file read.
        path: std::path::PathBuf,
        /// The manifest's pin.
        expected: String,
        /// The hash of the file as read.
        actual: String,
    },

    /// A source's SCIP index couldn't be read or decoded.
    #[error("source `{name}`: can't read the SCIP index {}", path.display())]
    Scip {
        /// The source's name.
        name: String,
        /// The index the manifest names.
        path: std::path::PathBuf,
        /// Why reading it failed.
        source: crate::ScipError,
    },

    /// A SCIP index no longer has the hash its `scip_sha256` pins.
    #[error(
        "source `{name}`: the SCIP index {} has sha256 {actual}, not the pinned {expected}; rebuild or review it and update the pin",
        path.display()
    )]
    ScipChanged {
        /// The source's name.
        name: String,
        /// The index the manifest names.
        path: std::path::PathBuf,
        /// The manifest's pin.
        expected: String,
        /// The hash of the file as read.
        actual: String,
    },

    /// A manifest names another corpus than the one asked for.
    #[error("manifest {} is for corpus `{found}`, not `{expected}`", path.display())]
    WrongCorpus {
        /// The manifest.
        path: std::path::PathBuf,
        /// The corpus asked for.
        expected: String,
        /// The corpus the manifest names.
        found: String,
    },

    /// A guidance source's `rules` names a rule its document doesn't have.
    #[error("guidance `{name}`: `rules.\"{number}\"` names no rule in the document")]
    UnknownRule {
        /// The guidance source's name.
        name: String,
        /// The rule number as the manifest writes it.
        number: String,
    },
}

/// `Result` with [`Error`] as the default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error's message followed by each of its sources', joined by `: `,
/// so a failed run records "SQLite error: database or disk is full" rather
/// than only the outermost "SQLite error".
pub(crate) fn error_chain(err: &dyn std::error::Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // A transparent wrapper repeats its source's message.
        if !message.ends_with(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::io;

    fn open_corpus() -> Result<()> {
        Err(codecite_core::Error::InvalidCorpus {
            path: "/tmp/corpus".into(),
            reason: "not a codecite store".into(),
        })?;
        Ok(())
    }

    #[test]
    fn core_error_converts_and_displays_unchanged() {
        let err = open_corpus().unwrap_err();
        assert!(matches!(err, Error::Core(_)));
        assert_eq!(
            err.to_string(),
            "invalid corpus at /tmp/corpus: not a codecite store"
        );
    }

    #[test]
    fn io_error_converts_and_keeps_source() {
        let err = Error::from(io::Error::other("pipe closed"));
        assert!(matches!(err, Error::Io(_)));
        assert_eq!(err.to_string(), "I/O error");
        assert_eq!(err.source().unwrap().to_string(), "pipe closed");
    }
}
