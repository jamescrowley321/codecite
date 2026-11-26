//! Schema and query engine for codecite: owns the per-corpus SQLite store
//! and answers searches and greps against it. It also parses and formats the
//! canonical code, document and spec citations, and references to a heading by its anchor, and parses the spec citation forms
//! people write into unresolved references. It also splits identifiers
//! into the search terms the index stores, and finds the BCP 14
//! requirement keywords in spec text. Depends on no other codecite crate.

mod citation;
pub use citation::{
    AnchorCitation, Citation, CitationError, CodeCitation, CommitSha, ContentHash,
    DocumentCitation, LineRange, Pin, SectionNumber, SpecAliases, SpecCitation, SpecReference,
    SpecTarget, WrittenCitation, find_citations,
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

mod normative;
pub use normative::{
    Level, NormativeStatement, StatementKind, normative_level, normative_statements,
};

mod role;
pub use role::{Role, RoleGlobs, RoleRules};

mod store;
pub use store::{
    APPROACH_ONLY_INSTRUCTION, ChunkRecord, CiteOptions, CiteResult, ColumnUnit, Context,
    ContextQuery, ContextTarget, Corpora, CorpusInfo, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_LINES,
    DEFAULT_SEARCH_LIMIT, Definition, Definitions, DiffOptions, DiffResult, Direction,
    DocumentRecord, FTS_TOKENIZER, Finding, Freshness, Graph, GraphNode, GraphQuery, GrepMatch,
    GrepQuery, GrepResults, IndexRun, MAX_CONFIDENCE, MAX_CONTEXT_BYTES, MAX_GRAPH_DEPTH,
    MAX_READ_BYTES, MAX_READ_LINES, MAX_SCIP_NAME_BYTES, MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES,
    MAX_SNIPPET_LINES, ManifestPin, PinReader, QUOTE_INSTRUCTION, QUOTE_TRUNCATED_INSTRUCTION,
    ROLE_DEFINITION, ReadOptions, ReadResult, Reference, References, Requirement,
    RequirementSection, Requirements, RequirementsOptions, Rule, RuleScope, RunCounts,
    SCHEMA_VERSION, SCIP_CONFIDENCE, ScipDefinition, ScipDocument, ScipOccurrence, ScipSummary,
    Scope, SearchQuery, SearchResult, SearchResults, SourceRecord, StalePin, Store, StoredDocument,
    SymbolRecord, VersionChange, corpora, corpora_for, corpus_info, defuse_instruction,
    is_corpus_name, symbol_name,
};
pub use store::{CheckOptions, CitationChecker, CitationProblem, Problem, TextCheck};

mod terms;
pub use terms::{TERMS_VERSION, identifier_terms, split_identifier, text_terms};
