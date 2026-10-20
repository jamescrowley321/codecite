//! Schema and query engine for codecite: owns the per-corpus SQLite store
//! and answers searches against it. It also parses and formats the
//! canonical code and spec citations. Depends on no other codecite crate.

mod citation;
pub use citation::{
    Citation, CitationError, CodeCitation, CommitSha, ContentHash, LineRange, SectionNumber,
    SpecCitation,
};

mod error;
pub use error::{Error, Result};

mod store;
pub use store::{SCHEMA_VERSION, Store};
