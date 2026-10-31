//! Index pipeline for codecite: walks a corpus, chunks it with tree-sitter
//! and imports provider outputs (SCIP, CodeGraph, Graphify) into the store.
//! It also reads corpus manifests, which say what each corpus indexes, walks
//! local checkouts, listing the files git sees and reporting every file it
//! skips, and splits files into chunks: code at definitions, Markdown at
//! headings, anything else into 60-line windows. [`Fetcher`] downloads document
//! sources and checks them against their pinned sha256, caching them by hash,
//! and [`Clones`] keeps a partial clone of each git source's repository.
//! [`index_corpus`] fetches a corpus's documents and pinned commits, runs the
//! other steps over its local sources and git sources' checkouts, splits
//! each document into spec sections and steps, and writes the result into
//! its store.

mod chunk;
pub use chunk::{
    Chunk, ChunkKind, Chunker, ChunkerError, Language, Symbol, SymbolKind, WINDOW_LINES,
};

mod clone;
pub use clone::{Checkout, CloneError, Clones};

mod document;

mod error;
pub use error::{Error, Result};

mod fetch;
pub use fetch::{FetchError, Fetched, Fetcher, MAX_DOCUMENT_SIZE};

mod guidance;
pub use guidance::{GuidanceError, GuidanceRule, split_guidance};

mod index;
pub use index::{IndexSummary, index_corpus};

mod manifest;
pub use manifest::{
    DocumentSource, GitSource, GuidanceSource, LicenseClass, LocalSource, Manifest, ManifestError,
    Source,
};

#[cfg(test)]
mod test_repo;

#[cfg(test)]
mod test_server;

mod spec;
pub use spec::{
    SpecDocument, SpecError, SpecFormat, SpecRelations, SpecSection, SpecStep, split_spec,
};

mod walk;
pub use walk::{
    DEFAULT_MAX_FILE_SIZE, LocalFile, LocalTree, SkipReason, SkippedFile, WalkError, WalkOptions,
    walk_local,
};
