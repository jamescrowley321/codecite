//! What an index run writes with: the full-text tokenizer, beside the
//! chunker and terms versions each document records.

/// The full-text tokenizer `chunk_fts` splits terms with, as its schema
/// names it.
pub const FTS_TOKENIZER: &str = "unicode61";
