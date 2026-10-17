//! Errors returned by the store and the query engine.

use std::path::PathBuf;

/// An error from `codecite-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Reading or writing a file failed.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// The path does not hold a usable corpus.
    #[error("invalid corpus at {}: {reason}", path.display())]
    InvalidCorpus {
        /// Where the corpus was expected.
        path: PathBuf,
        /// Why it can't be used.
        reason: String,
    },
}

/// `Result` with [`Error`] as the default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::io;

    fn read_missing() -> Result<()> {
        std::fs::read("/nonexistent/codecite-test")?;
        Ok(())
    }

    #[test]
    fn io_error_converts_with_question_mark() {
        assert!(matches!(read_missing(), Err(Error::Io(_))));
    }

    #[test]
    fn io_display_omits_source() {
        let err = Error::from(io::Error::other("disk on fire"));
        assert_eq!(err.to_string(), "I/O error");
    }

    #[test]
    fn io_source_is_the_io_error() {
        let err = Error::from(io::Error::other("disk on fire"));
        assert_eq!(err.source().unwrap().to_string(), "disk on fire");
    }

    #[test]
    fn invalid_corpus_display_names_path_and_reason() {
        let err = Error::InvalidCorpus {
            path: "/tmp/corpus".into(),
            reason: "not a codecite store".into(),
        };
        assert_eq!(
            err.to_string(),
            "invalid corpus at /tmp/corpus: not a codecite store"
        );
        assert!(err.source().is_none());
    }
}
