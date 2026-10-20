//! Chunking: splits a file into the spans that search returns. Code in a
//! supported language splits at definitions with tree-sitter, Markdown at
//! headings, and everything else into 60-line windows.

mod language;

pub use language::Language;
