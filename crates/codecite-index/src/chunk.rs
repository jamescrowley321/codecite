//! Chunking: splits a file into the spans that search returns. Code in a
//! supported language splits at definitions with tree-sitter, Markdown at
//! headings, and everything else into 60-line windows.

mod language;
mod text;

pub use language::Language;

/// The longest window, in lines.
pub const WINDOW_LINES: u32 = 60;

/// What a chunk covers, stored in `chunk.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// A function or method.
    Function,
    /// A class, struct, enum, trait or interface.
    Class,
    /// A Markdown heading and the text under it.
    Section,
    /// A run of at most [`WINDOW_LINES`] lines.
    Window,
}

impl ChunkKind {
    /// The value stored in `chunk.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Class => "class",
            Self::Section => "section",
            Self::Window => "window",
        }
    }
}

/// What kind of definition a symbol is, stored in `symbol.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A free function.
    Function,
    /// A function inside a class, struct, trait, interface or `impl` block.
    Method,
    /// A class or record.
    Class,
    /// A struct or union.
    Struct,
    /// An enum.
    Enum,
    /// A trait.
    Trait,
    /// An interface.
    Interface,
}

impl SymbolKind {
    /// The value stored in `symbol.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Interface => "interface",
        }
    }
}

/// A definition found by tree-sitter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// The name as written, e.g. `validate`.
    pub name: String,
    /// What kind of definition it is.
    pub kind: SymbolKind,
    /// The SCIP descriptors inside the file, e.g. `Client#validate().`.
    pub descriptor: String,
}

/// One span of a file, borrowing its text from the file's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk<'a> {
    /// What the chunk covers.
    pub kind: ChunkKind,
    /// The first line, counting from 1.
    pub start_line: u32,
    /// The last line, inclusive.
    pub end_line: u32,
    /// The chunk's lines, without the final newline.
    pub text: &'a str,
    /// The definition, for a function or class chunk.
    pub symbol: Option<Symbol>,
    /// The heading text, for a section chunk.
    pub title: Option<String>,
}

/// Splits files into chunks.
#[derive(Debug, Default)]
pub struct Chunker {}

impl Chunker {
    /// A chunker for Markdown and windows.
    pub fn new() -> Self {
        Self {}
    }

    /// Splits `text`, the contents of `path`, into chunks ordered by start
    /// line, then longest first. Lines outside every section, which is every
    /// line in a file that isn't Markdown, become windows.
    pub fn chunk<'a>(&mut self, path: &str, text: &'a str) -> Vec<Chunk<'a>> {
        let lines = text::Lines::new(text);
        let mut chunks = match Language::detect(path, text) {
            Some(Language::Markdown) => text::sections(&lines),
            _ => Vec::new(),
        };
        let mut covered = vec![false; lines.len()];
        for c in &chunks {
            covered[c.start_line as usize - 1..c.end_line as usize].fill(true);
        }
        chunks.extend(text::windows(&lines, &covered));
        chunks.sort_by_key(|c| (c.start_line, std::cmp::Reverse(c.end_line)));
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each chunk as (kind, start, end, symbol descriptor or section title).
    fn outline(path: &str, text: &str) -> Vec<(&'static str, u32, u32, String)> {
        let mut chunker = Chunker::new();
        chunker
            .chunk(path, text)
            .into_iter()
            .map(|c| {
                let label = match (&c.symbol, &c.title) {
                    (Some(s), _) => s.descriptor.clone(),
                    (None, Some(t)) => t.clone(),
                    (None, None) => String::new(),
                };
                (c.kind.as_str(), c.start_line, c.end_line, label)
            })
            .collect()
    }

    fn expect(rows: &[(&'static str, u32, u32, &str)]) -> Vec<(&'static str, u32, u32, String)> {
        rows.iter()
            .map(|&(k, s, e, l)| (k, s, e, l.to_string()))
            .collect()
    }

    #[test]
    fn markdown_splits_at_headings_and_windows_the_intro() {
        let text = "Intro.\n\n# Title\nText.\n\n## Use\nRun it.\n";
        assert_eq!(
            outline("README.md", text),
            expect(&[
                ("window", 1, 1, ""),
                ("section", 3, 4, "Title"),
                ("section", 6, 7, "Use"),
            ])
        );
    }

    #[test]
    fn markdown_without_headings_and_other_text_fall_back_to_windows() {
        let text: String = (1..=61).map(|i| format!("line {i}\n")).collect();
        let want = expect(&[("window", 1, 60, ""), ("window", 61, 61, "")]);
        assert_eq!(outline("notes.md", &text), want);
        assert_eq!(outline("notes.txt", &text), want);
    }
}
