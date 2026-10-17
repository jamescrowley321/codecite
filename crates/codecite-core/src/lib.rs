//! Schema and query engine for codecite: owns the per-corpus SQLite store
//! and answers searches against it. Depends on no other codecite crate.

mod error;
pub use error::{Error, Result};
