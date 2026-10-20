//! Schema and query engine for codecite: owns the per-corpus SQLite store
//! and answers searches against it. It also parses and formats the
//! canonical code and spec citations, and parses the spec citation forms
//! people write into unresolved references. Depends on no other codecite
//! crate.

mod citation;
pub use citation::{
    Citation, CitationError, CodeCitation, CommitSha, ContentHash, LineRange, SectionNumber,
    SpecAliases, SpecCitation, SpecReference, SpecTarget,
};

mod error;
pub use error::{Error, Result};

mod store;
pub use store::{SCHEMA_VERSION, Store};
