//! Chunking: splits a file into the spans that search returns. Code in a
//! supported language splits at definitions with tree-sitter, Markdown and
//! HTML at headings, and everything else into 60-line windows.

mod code;
mod text;

pub use code::ChunkerError;
pub use codecite_core::Language;
pub(crate) use text::{Heading, Lines, Slugs, markdown_headings, sections, sections_at};

/// The longest window, in lines.
pub const WINDOW_LINES: u32 = 60;

/// What a chunk covers, stored in `chunk.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// A function or method.
    Function,
    /// A class, struct, enum, trait or interface.
    Class,
    /// A top-level variable or constant.
    Variable,
    /// A type alias.
    TypeAlias,
    /// A Markdown or HTML heading and the text under it.
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
            Self::Variable => "variable",
            Self::TypeAlias => "type_alias",
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
    /// A top-level variable or constant: Python `NAME = ...`, a TypeScript
    /// or JavaScript `const`, a Rust `const` or `static`, a Go `const` or
    /// `var`.
    Variable,
    /// A type alias: Python `type NAME = ...`, TypeScript and Rust `type`,
    /// and a Go type that is neither a struct nor an interface. A Python
    /// alias assigned like a variable (`CurrentUser = Annotated[...]`) is a
    /// [`SymbolKind::Variable`].
    TypeAlias,
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
            Self::Variable => "variable",
            Self::TypeAlias => "type_alias",
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
    /// The heading's GitHub-style anchor, for a section chunk.
    pub anchor: Option<String>,
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
        let chunks = match Language::detect(path, text) {
            Some(Language::Markdown) => text::sections(&lines),
            Some(Language::Html) => text::html_sections(&lines),
            Some(language) => self.code.chunk(language, &lines),
            None => Vec::new(),
        };
        with_windows(&lines, chunks)
    }
}

/// `chunks` plus a window over every run of lines no chunk covers, ordered
/// by start line, then longest first.
pub(crate) fn with_windows<'a>(lines: &Lines<'a>, mut chunks: Vec<Chunk<'a>>) -> Vec<Chunk<'a>> {
    let mut covered = vec![false; lines.len()];
    for c in &chunks {
        covered[c.start_line as usize - 1..c.end_line as usize].fill(true);
    }
    chunks.extend(text::windows(lines, &covered));
    chunks.sort_by_key(|c| (c.start_line, std::cmp::Reverse(c.end_line)));
    chunks
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
    fn html_splits_at_heading_tags_and_windows_the_head() {
        let text = "<html>\n<head><title>Guide</title></head>\n<body>\n<h1>Guide</h1>\n<p>Text.</p>\n<h2>Use</h2>\n<p>Run it.</p>\n</body>\n</html>\n";
        assert_eq!(
            outline("docs/guide.html", text),
            expect(&[
                ("window", 1, 3, ""),
                ("section", 4, 5, "Guide"),
                ("section", 6, 9, "Use"),
            ])
        );
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
                ("window", 1, 3, ""),
                ("class", 5, 8, "Client#"),
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
                ("window", 1, 3, ""),
                ("class", 5, 8, "Client#"),
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
    fn top_level_variables_and_type_aliases_are_definitions() {
        let python = "CurrentUser = Annotated[ClaimsPrincipal, Depends(get_user)]\n\
                      TIMEOUT: int = 30\n\
                      type Claims = dict[str, str]\n\
                      a, b = 1, 2\n\
                      def f():\n    local = 1\n";
        assert_eq!(
            outline("app/deps.py", python),
            expect(&[
                ("variable", 1, 1, "CurrentUser."),
                ("variable", 2, 2, "TIMEOUT."),
                ("type_alias", 3, 3, "Claims#"),
                ("window", 4, 4, ""),
                ("function", 5, 6, "f()."),
            ])
        );
        let ts = "export type Claims = Record<string, string>;\n\
                  export const TIMEOUT = 30;\n\
                  const parse = (raw: string) => raw;\n\
                  let a = 1, b = 2;\n\
                  function f() { const local = 1; }\n";
        let want = expect(&[
            ("type_alias", 1, 1, "Claims#"),
            ("variable", 2, 2, "TIMEOUT."),
            ("function", 3, 3, "parse()."),
            ("variable", 4, 4, "a."),
            ("variable", 4, 4, "b."),
            ("function", 5, 5, "f()."),
        ]);
        assert_eq!(outline("src/deps.ts", ts), want);
        let js = "export const TIMEOUT = 30;\nvar Client = class {};\nconst f = function () {};\n";
        assert_eq!(
            outline("lib/deps.js", js),
            expect(&[
                ("variable", 1, 1, "TIMEOUT."),
                ("variable", 2, 2, "Client."),
                ("function", 3, 3, "f()."),
            ])
        );
        let rust = "pub const TIMEOUT: u32 = 30;\nstatic NAME: &str = \"x\";\npub type Claims = Vec<String>;\nimpl Client { const MAX: u8 = 1; }\n";
        assert_eq!(
            outline("src/deps.rs", rust),
            expect(&[
                ("variable", 1, 1, "TIMEOUT."),
                ("variable", 2, 2, "NAME."),
                ("type_alias", 3, 3, "Claims#"),
                ("class", 4, 4, ""),
                ("variable", 4, 4, "Client#MAX."),
            ])
        );
        let go = "package deps\n\
                  const Timeout = 30\n\
                  var (\n\tA = 1\n\tB = 2\n)\n\
                  type Mode int\n\
                  type Handler = func()\n\
                  type Client struct{}\n\
                  func f() { const local = 1 }\n";
        assert_eq!(
            outline("deps/deps.go", go),
            expect(&[
                ("window", 1, 1, ""),
                ("variable", 2, 2, "Timeout."),
                ("window", 3, 3, ""),
                ("variable", 4, 4, "A."),
                ("variable", 5, 5, "B."),
                ("type_alias", 7, 7, "Mode#"),
                ("type_alias", 8, 8, "Handler#"),
                ("class", 9, 9, "Client#"),
                ("function", 10, 10, "f()."),
            ])
        );
        // A grouped `const` holds its specs directly; only `var` has a list.
        let grouped = "package deps\nconst (\n\tX = 1\n\tY = 2\n)\n";
        assert_eq!(
            outline("deps/consts.go", grouped),
            expect(&[
                ("window", 1, 2, ""),
                ("variable", 3, 3, "X."),
                ("variable", 4, 4, "Y."),
            ])
        );
    }

    #[test]
    fn doc_comments_and_attributes_join_their_definition() {
        // Doc comments and attributes directly above, in any order; a blank
        // line, a plain comment or a module's `//!` doc stops the run.
        let rust = "//! Module docs.\n\
                    fn first() {}\n\
                    \n\
                    /// Fetches the metadata.\n\
                    #[inline]\n\
                    /** More. */\n\
                    pub fn fetch() {}\n\
                    \n\
                    /// Detached.\n\
                    \n\
                    // Not documentation.\n\
                    struct Plain;\n\
                    impl Plain {\n    /// Validates.\n    fn validate() {}\n}\n";
        assert_eq!(
            outline("doc.rs", rust),
            expect(&[
                ("window", 1, 1, ""),
                ("function", 2, 2, "first()."),
                ("function", 4, 7, "fetch()."),
                ("window", 9, 11, ""),
                ("class", 12, 12, "Plain#"),
                ("class", 13, 16, ""),
                ("function", 14, 15, "Plain#validate()."),
            ])
        );
        // Every comment directly above, through `export`, `type`, `const`
        // and `var` keywords; never a comment that ends a line of code.
        let go = "package p\n\
                  \n\
                  // FetchConfiguration fetches the discovery metadata.\n\
                  //\n\
                  // It caches nothing.\n\
                  func FetchConfiguration() {}\n\
                  \n\
                  // Client is a client.\n\
                  type Client struct{}\n\
                  \n\
                  /* Timeout is in seconds. */\n\
                  const Timeout = 30\n\
                  \n\
                  var (\n\
                  \t// A is first.\n\
                  \tA = 1 // one\n\
                  \tB = 2\n\
                  )\n\
                  \n\
                  // Validate checks a token.\n\
                  func (c *Client) Validate() {}\n";
        assert_eq!(
            outline("p/p.go", go),
            expect(&[
                ("window", 1, 1, ""),
                ("function", 3, 6, "FetchConfiguration()."),
                ("class", 8, 9, "Client#"),
                ("variable", 11, 12, "Timeout."),
                ("window", 14, 14, ""),
                ("variable", 15, 16, "A."),
                ("variable", 17, 17, "B."),
                ("function", 20, 21, "Client#Validate()."),
            ])
        );
        // JSDoc blocks and decorators, exported or not; a plain comment
        // stays out.
        let ts = "/** Fetches the metadata. */\n\
                  export function fetch() {}\n\
                  \n\
                  // Not documentation.\n\
                  function plain() {}\n\
                  \n\
                  /**\n * The client.\n */\n\
                  @Injectable()\n\
                  export class Client {\n\
                  \x20 /** Validates. */\n\
                  \x20 @Log()\n\
                  \x20 validate() {}\n\
                  }\n\
                  \n\
                  /** Parses. */\n\
                  export const parse = (raw: string) => raw;\n";
        let want = expect(&[
            ("function", 1, 2, "fetch()."),
            ("window", 4, 4, ""),
            ("function", 5, 5, "plain()."),
            ("class", 7, 15, "Client#"),
            ("function", 12, 14, "Client#validate()."),
            ("function", 17, 18, "parse()."),
        ]);
        assert_eq!(outline("src/doc.ts", ts), want);
        assert_eq!(outline("src/doc.tsx", ts), want);
        let js = "/** Fetches. */\n\
                  function fetch() {}\n\
                  \n\
                  class Client {\n\
                  \x20 /** Validates. */\n\
                  \x20 validate() {}\n\
                  }\n";
        assert_eq!(
            outline("lib/doc.js", js),
            expect(&[
                ("function", 1, 2, "fetch()."),
                ("class", 4, 7, "Client#"),
                ("function", 5, 6, "Client#validate()."),
            ])
        );
        // XML doc comments (`///`) and `/** */`; a plain comment stays out.
        let cs = "/// <summary>The client.</summary>\n\
                  public class Client\n\
                  {\n\
                  \x20   /** Validates. */\n\
                  \x20   [Obsolete]\n\
                  \x20   public bool Validate() { return true; }\n\
                  \n\
                  \x20   // Not documentation.\n\
                  \x20   public void Plain() {}\n\
                  }\n";
        assert_eq!(
            outline("src/Doc.cs", cs),
            expect(&[
                ("class", 1, 10, "Client#"),
                ("function", 4, 6, "Client#Validate()."),
                ("function", 9, 9, "Client#Plain()."),
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
                ("class", 2, 3, ""),
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
