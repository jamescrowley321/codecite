//! Index pipeline for codecite: walks a corpus, chunks it with tree-sitter
//! and imports provider outputs (SCIP, CodeGraph, Graphify) into the store.
//! It also reads corpus manifests, which say what each corpus indexes, walks
//! local checkouts, listing the files git sees and reporting every file it
//! skips, and splits files into chunks: code at definitions, Markdown at
//! headings, anything else into 60-line windows.

mod chunk;
pub use chunk::{
    Chunk, ChunkKind, Chunker, ChunkerError, Language, Symbol, SymbolKind, WINDOW_LINES,
};

mod error;
pub use error::{Error, Result};

mod manifest;
pub use manifest::{
    DocumentSource, GitSource, LicenseClass, LocalSource, Manifest, ManifestError, Source,
};

mod walk;
pub use walk::{
    DEFAULT_MAX_FILE_SIZE, LocalFile, LocalTree, SkipReason, SkippedFile, WalkError, WalkOptions,
    walk_local,
};
