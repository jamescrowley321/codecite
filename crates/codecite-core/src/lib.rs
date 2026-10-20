//! Schema and query engine for codecite: owns the per-corpus SQLite store
//! and answers searches against it. It also parses and formats the
//! canonical code and spec citations, and parses the spec citation forms
//! people write into unresolved references. It also splits identifiers
//! into the search terms the index stores. Depends on no other codecite
//! crate.

mod citation;
pub use citation::{
    Citation, CitationError, CodeCitation, CommitSha, ContentHash, LineRange, SectionNumber,
    SpecAliases, SpecCitation, SpecReference, SpecTarget,
};

mod error;
pub use error::{Error, Result};

mod store;
pub use store::{ChunkRecord, DocumentRecord, SCHEMA_VERSION, SourceRecord, Store, SymbolRecord};

mod terms;
pub use terms::{identifier_terms, split_identifier, text_terms};
