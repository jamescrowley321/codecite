//! Chunking: splits a file into the spans that search returns. Code in a
//! supported language splits at definitions with tree-sitter, Markdown at
//! headings, and everything else into 60-line windows.

mod code;
mod language;
mod text;

pub use code::ChunkerError;
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

impl Symbol {
    /// The SCIP-shaped ID for this symbol in `path` of the source named
    /// `source`: `codecite . <source> . <path segments>/<descriptor>`.
    pub fn scip_id(&self, source: &str, path: &str) -> String {
        let mut id = format!("codecite . {} . ", source.replace(' ', "  "));
        for segment in path.split('/') {
            id.push_str(&code::escape_name(segment));
            id.push('/');
        }
        id.push_str(&self.descriptor);
        id
    }
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

/// Splits files into chunks. Holds a parser and the compiled queries, so
/// build one and reuse it for every file.
pub struct Chunker {
    code: code::CodeChunker,
}

impl Chunker {
    /// Compiles the tree-sitter query of every supported language.
    pub fn new() -> Result<Self, ChunkerError> {
        Ok(Self {
            code: code::CodeChunker::new()?,
        })
    }

    /// Splits `text`, the contents of `path`, into chunks ordered by start
    /// line, then longest first. Lines outside every definition and every
    /// section, which is every line in a file of another language, become
    /// windows.
    pub fn chunk<'a>(&mut self, path: &str, text: &'a str) -> Vec<Chunk<'a>> {
        let lines = text::Lines::new(text);
        let mut chunks = match Language::detect(path, text) {
            Some(Language::Markdown) => text::sections(&lines),
            Some(language) => self.code.chunk(language, &lines),
            None => Vec::new(),
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
        let mut chunker = Chunker::new().unwrap();
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

    #[test]
    fn every_query_compiles() {
        Chunker::new().unwrap();
    }

    #[test]
    fn rust_fixture() {
        let text = include_str!("../tests/fixtures/chunk/sample.rs");
        assert_eq!(
            outline("src/sample.rs", text),
            expect(&[
                ("window", 1, 5, ""),
                ("class", 6, 8, "Client#"),
                ("class", 10, 18, ""),
                ("function", 11, 13, "Client#new()."),
                ("function", 15, 17, "Client#validate()."),
                ("class", 20, 24, ""),
                ("function", 21, 23, "Client#fmt()."),
                ("class", 26, 29, "Grant#"),
                ("class", 31, 33, "Verifier#"),
                ("function", 32, 32, "Verifier#verify()."),
                ("function", 35, 40, "helper()."),
                ("function", 36, 38, "helper().inner()."),
            ])
        );
    }

    #[test]
    fn python_fixture() {
        let text = include_str!("../tests/fixtures/chunk/sample.py");
        assert_eq!(
            outline("app/sample.py", text),
            expect(&[
                ("window", 1, 4, ""),
                ("class", 7, 13, "Client#"),
                ("function", 8, 9, "Client#__init__()."),
                ("function", 11, 13, "Client#validate()."),
                ("function", 16, 20, "helper()."),
                ("function", 17, 18, "helper().inner()."),
            ])
        );
    }

    #[test]
    fn typescript_fixture() {
        let text = include_str!("../tests/fixtures/chunk/sample.ts");
        let want = expect(&[
            ("window", 1, 1, ""),
            ("class", 3, 5, "Claims#"),
            ("class", 7, 10, "Grant#"),
            ("class", 12, 18, "Client#"),
            ("function", 13, 13, "Client#constructor()."),
            ("function", 15, 17, "Client#validate()."),
            ("function", 20, 22, "helper()."),
            ("function", 24, 26, "parse()."),
        ]);
        assert_eq!(outline("src/sample.ts", text), want);
        // The TSX grammar parses the same file the same way.
        assert_eq!(outline("src/sample.tsx", text), want);
    }

    #[test]
    fn javascript_fixture() {
        let text = include_str!("../tests/fixtures/chunk/sample.js");
        assert_eq!(
            outline("lib/sample.js", text),
            expect(&[
                ("window", 1, 2, ""),
                ("class", 4, 8, "Client#"),
                ("function", 5, 7, "Client#validate()."),
                ("function", 10, 12, "helper()."),
                ("window", 14, 14, ""),
            ])
        );
    }

    #[test]
    fn go_fixture() {
        let text = include_str!("../tests/fixtures/chunk/sample.go");
        assert_eq!(
            outline("sample/sample.go", text),
            expect(&[
                ("window", 1, 5, ""),
                ("class", 6, 8, "Client#"),
                ("class", 10, 12, "Verifier#"),
                ("function", 11, 11, "Verifier#Verify()."),
                ("function", 14, 16, "NewClient()."),
                ("function", 18, 20, "Client#Validate()."),
            ])
        );
    }

    #[test]
    fn csharp_fixture() {
        let text = include_str!("../tests/fixtures/chunk/sample.cs");
        assert_eq!(
            outline("src/Sample.cs", text),
            expect(&[
                ("window", 1, 3, ""),
                ("class", 5, 8, "IVerifier#"),
                ("function", 7, 7, "IVerifier#Verify()."),
                ("class", 10, 14, "Grant#"),
                ("class", 16, 29, "Client#"),
                ("function", 18, 21, "Client#Client()."),
                ("function", 25, 28, "Client#Validate()."),
            ])
        );
    }

    #[test]
    fn symbols_carry_name_and_kind() {
        let text = include_str!("../tests/fixtures/chunk/sample.go");
        let mut chunker = Chunker::new().unwrap();
        let symbols: Vec<_> = chunker
            .chunk("sample.go", text)
            .into_iter()
            .filter_map(|c| c.symbol)
            .map(|s| (s.name, s.kind))
            .collect();
        assert_eq!(
            symbols,
            [
                ("Client".to_string(), SymbolKind::Struct),
                ("Verifier".to_string(), SymbolKind::Interface),
                ("Verify".to_string(), SymbolKind::Method),
                ("NewClient".to_string(), SymbolKind::Function),
                ("Validate".to_string(), SymbolKind::Method),
            ]
        );
    }

    #[test]
    fn chunk_text_borrows_whole_lines() {
        let text = include_str!("../tests/fixtures/chunk/sample.py");
        let mut chunker = Chunker::new().unwrap();
        let chunks = chunker.chunk("sample.py", text);
        let validate = chunks.iter().find(|c| c.start_line == 11).unwrap();
        assert_eq!(
            validate.text,
            "    @staticmethod\n    def validate(token):\n        return bool(token)"
        );
    }

    #[test]
    fn duplicate_names_keep_ids_unique() {
        let text = "struct Dup;\n#[cfg(test)]\nstruct Dup;\nmod m { fn new() {} }\nfn new() {}\nimpl<T> Wrap<T> { fn new() {} }\nimpl a::Path { fn new() {} }\n";
        assert_eq!(
            outline("dup.rs", text),
            expect(&[
                ("class", 1, 1, "Dup#"),
                ("window", 2, 2, ""),
                ("class", 3, 3, ""),
                ("function", 4, 4, "new()."),
                ("function", 5, 5, "new(+1)."),
                ("class", 6, 6, ""),
                ("function", 6, 6, "Wrap#new()."),
                ("class", 7, 7, ""),
                ("function", 7, 7, "Path#new()."),
            ])
        );
    }

    #[test]
    fn deep_nesting_stops_at_the_depth_limit() {
        // 20,000 nested functions, 180 KB: quadratic memory would need gigabytes.
        let depth = 20_000;
        let text = "fn f() {\n".repeat(depth) + &"}\n".repeat(depth);
        let mut chunker = Chunker::new().unwrap();
        let chunks = chunker.chunk("deep.rs", &text);
        assert_eq!(chunks.len(), code::MAX_DEPTH);
        let deepest = chunks.last().unwrap().symbol.as_ref().unwrap();
        assert_eq!(deepest.descriptor, "f().".repeat(code::MAX_DEPTH));
    }

    #[test]
    fn long_names_get_no_symbol_and_their_members_none_either() {
        // A long impl target with many methods: copying its name into every
        // method's descriptor would take memory quadratic in the file.
        let name = "N".repeat(code::MAX_DESCRIPTOR);
        let text = format!(
            "impl {name} {{\n{}}}\nfn after() {{}}\n",
            "fn a() {}\n".repeat(1_000)
        );
        let mut chunker = Chunker::new().unwrap();
        let chunks = chunker.chunk("long.rs", &text);
        assert_eq!(chunks.len(), 1_002);
        let symbols: Vec<_> = chunks.iter().filter_map(|c| c.symbol.as_ref()).collect();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].descriptor, "after().");
        // A type whose own descriptor fits keeps it.
        let fits = "N".repeat(code::MAX_DESCRIPTOR - 1);
        let got = outline("fits.rs", &format!("struct {fits};\n"));
        assert_eq!(got, expect(&[("class", 1, 1, &format!("{fits}#"))]));
    }

    #[test]
    fn descriptors_stop_when_the_file_budget_is_spent() {
        // Each method's descriptor is about 500 bytes; the budget runs out
        // part way through, and later definitions get no symbol.
        let name = "N".repeat(490);
        let methods: String = (0..40_000).map(|i| format!("fn a{i}() {{}}\n")).collect();
        let text = format!("impl {name} {{\n{methods}}}\n");
        let mut chunker = Chunker::new().unwrap();
        let chunks = chunker.chunk("wide.rs", &text);
        assert_eq!(chunks.len(), 40_001);
        let bytes: usize = chunks
            .iter()
            .filter_map(|c| c.symbol.as_ref())
            .map(|s| s.descriptor.len())
            .sum();
        assert!(bytes <= code::DESCRIPTOR_BUDGET, "{bytes}");
        assert!(
            bytes > code::DESCRIPTOR_BUDGET - 2 * code::MAX_DESCRIPTOR,
            "{bytes}"
        );
        assert!(chunks.last().unwrap().symbol.is_none());
    }

    #[test]
    fn impl_targets_behind_references_and_dyn_name_their_type() {
        let text = "impl<'a> IntoIterator for &'a Foo { fn into_iter() {} }\nimpl dyn Any { fn is() {} }\nimpl Bar for *const Baz { fn get() {} }\n";
        assert_eq!(
            outline("refs.rs", text),
            expect(&[
                ("class", 1, 1, ""),
                ("function", 1, 1, "Foo#into_iter()."),
                ("class", 2, 2, ""),
                ("function", 2, 2, "Any#is()."),
                ("class", 3, 3, ""),
                ("function", 3, 3, "Baz#get()."),
            ])
        );
    }

    #[test]
    fn interface_methods_without_bodies() {
        assert_eq!(
            outline("i.ts", "interface I {\n  run(): void;\n}\n"),
            expect(&[("class", 1, 3, "I#"), ("function", 2, 2, "I#run().")])
        );
        assert_eq!(
            outline("i.go", "package p\n\ntype I interface {\n\tRun() bool\n}\n"),
            expect(&[
                ("window", 1, 1, ""),
                ("class", 3, 5, "I#"),
                ("function", 4, 4, "I#Run()."),
            ])
        );
    }

    #[test]
    fn named_class_expressions() {
        let text = "const C = class Client {\n  validate() {}\n};\n";
        let want = expect(&[
            ("class", 1, 3, "Client#"),
            ("function", 2, 2, "Client#validate()."),
        ]);
        assert_eq!(outline("c.js", text), want);
        assert_eq!(outline("c.ts", text), want);
    }

    #[test]
    fn markdown_inline_code_is_not_a_fence() {
        let text = "```inline```\n# Real\ntext\n";
        assert_eq!(
            outline("a.md", text),
            expect(&[("window", 1, 1, ""), ("section", 2, 3, "Real")])
        );
    }

    #[test]
    fn broken_or_empty_code_still_chunks() {
        assert!(outline("empty.rs", "").is_empty());
        // tree-sitter recovers from syntax errors; the text stays searchable.
        let got = outline("broken.rs", "fn ok() {}\nfn broken( {\nlet x = ;\n");
        assert_eq!(got[0], ("function", 1, 1, "ok().".to_string()));
        assert!(got.iter().any(|c| c.2 == 3), "{got:?}");
    }

    #[test]
    fn crlf_line_endings() {
        let got = outline("crlf.rs", "fn a() {}\r\n\r\nfn b() {\r\n}");
        assert_eq!(
            got,
            expect(&[("function", 1, 1, "a()."), ("function", 3, 4, "b().")])
        );
    }

    #[test]
    fn scip_id_prefixes_source_and_path() {
        let symbol = Symbol {
            name: "validate".into(),
            kind: SymbolKind::Method,
            descriptor: "Client#validate().".into(),
        };
        assert_eq!(
            symbol.scip_id("identity-model", "src/client.rs"),
            "codecite . identity-model . src/`client.rs`/Client#validate()."
        );
        assert_eq!(
            symbol.scip_id("my repo", "a`b/c"),
            "codecite . my  repo . `a``b`/c/Client#validate()."
        );
    }

    #[test]
    fn escape_name_follows_scip() {
        assert_eq!(code::escape_name("new"), "new");
        assert_eq!(code::escape_name("$el-x+1"), "$el-x+1");
        assert_eq!(code::escape_name("größe"), "`größe`");
        assert_eq!(
            code::escape_name("[Symbol.iterator]"),
            "`[Symbol.iterator]`"
        );
        assert_eq!(code::escape_name(""), "``");
    }
}
