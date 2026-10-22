//! Schema and query engine for codecite: owns the per-corpus SQLite store
//! and answers searches against it. It also parses and formats the
//! canonical code and spec citations, and parses the spec citation forms
//! people write into unresolved references. It also splits identifiers
//! into the search terms the index stores, and finds the BCP 14
//! requirement keywords in spec text. Depends on no other codecite crate.

mod citation;
pub use citation::{
    Citation, CitationError, CodeCitation, CommitSha, ContentHash, LineRange, SectionNumber,
    SpecAliases, SpecCitation, SpecReference, SpecTarget,
};

mod error;
pub use error::{Error, Result};

mod keyword;
pub use keyword::{Keyword, UnknownKeyword, requirement_keywords};

mod role;
pub use role::Role;

mod store;
pub use store::{
    ChunkRecord, Corpora, CorpusInfo, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_LINES, DocumentRecord,
    Freshness, MAX_SNIPPET_LINES, ReadOptions, ReadResult, SCHEMA_VERSION, SearchQuery,
    SearchResult, SourceRecord, Store, SymbolRecord, corpora, is_corpus_name,
};

mod terms;
pub use terms::{identifier_terms, split_identifier, text_terms};
