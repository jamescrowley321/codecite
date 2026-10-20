//! Index pipeline for codecite: walks a corpus, chunks it with tree-sitter
//! and imports provider outputs (SCIP, CodeGraph, Graphify) into the store.
//! It also reads corpus manifests, which say what each corpus indexes, and
//! walks local checkouts, listing the files git sees and reporting every file
//! it skips.

mod error;
pub use error::{Error, Result};

mod manifest;
pub use manifest::{
    DocumentSource, GitSource, LicenseClass, LocalSource, Manifest, ManifestError, Source,
};

mod walk;
pub use walk::WalkError;
