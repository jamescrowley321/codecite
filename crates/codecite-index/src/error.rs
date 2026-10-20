//! Errors returned by the index pipeline.

/// An error from `codecite-index`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The store or query engine failed.
    #[error(transparent)]
    Core(#[from] codecite_core::Error),

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
}

/// `Result` with [`Error`] as the default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

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
