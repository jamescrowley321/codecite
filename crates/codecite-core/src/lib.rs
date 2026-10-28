//! Schema and query engine for codecite: owns the per-corpus SQLite store
//! and answers searches and greps against it. It also parses and formats the
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

mod imports;
pub use imports::{imports, imports_dependency};

mod invisible;
pub use invisible::is_invisible_format;

mod language;
pub use language::Language;

mod keyword;
pub use keyword::{Keyword, UnknownKeyword, requirement_keywords};

mod role;
pub use role::Role;

mod store;
pub use store::{
    ChunkRecord, CiteOptions, CiteResult, Context, ContextQuery, ContextTarget, Corpora,
    CorpusInfo, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_LINES, DEFAULT_SEARCH_LIMIT, Definition,
    Definitions, DocumentRecord, Finding, Freshness, GrepMatch, GrepQuery, GrepResults,
    MAX_CONTEXT_BYTES, MAX_READ_BYTES, MAX_READ_LINES, MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES,
    MAX_SNIPPET_LINES, QUOTE_INSTRUCTION, QUOTE_TRUNCATED_INSTRUCTION, ReadOptions, ReadResult,
    Rule, RuleScope, SCHEMA_VERSION, Scope, SearchQuery, SearchResult, SearchResults, SourceRecord,
    Store, SymbolRecord, corpora, is_corpus_name,
};

mod terms;
pub use terms::{identifier_terms, split_identifier, text_terms};
