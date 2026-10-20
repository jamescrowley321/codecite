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

    /// A SQLite call failed.
    #[error("SQLite error")]
    Sqlite(#[from] rusqlite::Error),

    /// The corpus was written by a newer codecite, with a schema this one
    /// doesn't know.
    #[error(
        "corpus at {} has schema version {found}, but this codecite supports up to {supported}; upgrade codecite",
        path.display()
    )]
    SchemaTooNew {
        /// Where the corpus is.
        path: PathBuf,
        /// The schema version stored in the corpus.
        found: u32,
        /// The newest schema version this build supports.
        supported: u32,
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

    #[test]
    fn sqlite_error_converts_and_is_the_source() {
        let inner = rusqlite::Error::InvalidQuery;
        let msg = inner.to_string();
        let err = Error::from(inner);
        assert!(matches!(err, Error::Sqlite(_)));
        assert_eq!(err.to_string(), "SQLite error");
        assert_eq!(err.source().unwrap().to_string(), msg);
    }

    #[test]
    fn schema_too_new_display_names_path_and_versions() {
        let err = Error::SchemaTooNew {
            path: "/tmp/corpus.db".into(),
            found: 7,
            supported: 1,
        };
        assert_eq!(
            err.to_string(),
            "corpus at /tmp/corpus.db has schema version 7, but this codecite supports up to 1; upgrade codecite"
        );
        assert!(err.source().is_none());
    }
}
