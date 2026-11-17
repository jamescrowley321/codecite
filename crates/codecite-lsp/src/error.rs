//! Errors returned by the LSP live provider.

use std::process::ExitStatus;
use std::time::Duration;

/// An error from `codecite-lsp`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The store or query engine failed.
    #[error(transparent)]
    Core(#[from] codecite_core::Error),

    /// Talking to the language server process failed.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// A frame's header block has no `Content-Length`.
    #[error("LSP frame has no Content-Length header")]
    MissingContentLength,

    /// A frame's `Content-Length` is not a non-negative integer.
    #[error("LSP frame has an invalid Content-Length: {value:?}")]
    InvalidContentLength {
        /// The header's value as received.
        value: String,
    },

    /// A header line is not `Name: value`.
    #[error("LSP frame has a malformed header line: {line:?}")]
    MalformedHeader {
        /// The line as received, lossily decoded.
        line: String,
    },

    /// A frame's header block is longer than the cap.
    #[error("LSP frame header is over the {limit}-byte cap")]
    HeaderTooLarge {
        /// The cap, in bytes.
        limit: usize,
    },

    /// A frame's body is longer than the cap.
    #[error("LSP frame body of {length} bytes is over the {limit}-byte cap")]
    BodyTooLarge {
        /// The declared `Content-Length`.
        length: u64,
        /// The cap, in bytes.
        limit: usize,
    },

    /// The stream ended inside a frame.
    #[error("LSP stream ended inside a frame")]
    TruncatedFrame,

    /// A frame's body is not JSON, or not a JSON-RPC 2.0 message.
    #[error("LSP frame body is not a JSON-RPC message: {reason}")]
    InvalidMessage {
        /// What was wrong with it.
        reason: String,
    },

    /// The server answered a request with a JSON-RPC error.
    #[error("{method} failed with JSON-RPC error {code}: {message}")]
    Rpc {
        /// The request's method.
        method: String,
        /// The error's `code`.
        code: i64,
        /// The error's `message`.
        message: String,
        /// The error's `data`, if any.
        data: Option<serde_json::Value>,
    },

    /// The server cancelled the request (`RequestCancelled`, -32800). Retryable.
    #[error("{method} was cancelled by the server")]
    RequestCancelled {
        /// The request's method.
        method: String,
    },

    /// The document changed under the request (`ContentModified`, -32801).
    /// Retryable.
    #[error("{method} was abandoned because the content changed")]
    ContentModified {
        /// The request's method.
        method: String,
    },

    /// The request's deadline passed; `$/cancelRequest` was sent.
    #[error("{method} timed out after {elapsed:?}")]
    Timeout {
        /// The request's method.
        method: String,
        /// How long the client waited.
        elapsed: Duration,
    },

    /// The connection to the server is closed: its output ended, a frame was
    /// malformed, or a write failed. Every request fails with this from then on.
    #[error("connection to the language server is closed: {reason}")]
    Closed {
        /// Why the connection closed.
        reason: String,
    },

    /// The server's command is not on `PATH`. Callers fall back to the index.
    #[error("language server {command:?} not found")]
    ServerNotFound {
        /// The command as given.
        command: String,
    },

    /// The server did not finish starting within the startup timeout.
    #[error("language server not ready after {elapsed:?}; stderr: {stderr}")]
    ServerNotReady {
        /// How long codecite waited.
        elapsed: Duration,
        /// The tail of the server's stderr.
        stderr: String,
    },

    /// The server process exited.
    #[error("language server exited ({status}); stderr: {stderr}")]
    ServerExited {
        /// Its exit status.
        status: ExitStatus,
        /// The tail of the server's stderr.
        stderr: String,
    },
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

    #[test]
    fn timeout_names_the_method_and_elapsed_time() {
        let err = Error::Timeout {
            method: "textDocument/definition".into(),
            elapsed: Duration::from_millis(250),
        };
        assert_eq!(
            err.to_string(),
            "textDocument/definition timed out after 250ms"
        );
    }
}
