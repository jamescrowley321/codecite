# Tree-sitter Chunking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #20: split files into chunks in `codecite-index`. Code in Rust, Python, TypeScript (and TSX), JavaScript, Go and C# splits at definitions with tree-sitter, each definition yielding a symbol with a SCIP-shaped local ID; Markdown splits at headings; anything else into 60-line windows (`docs/design.md#indexing-and-retrieval`, step 3).

**Architecture:** A new `chunk` module in `codecite-index` with three private submodules. `language` detects the language from the extension, then the shebang. `text` holds `Lines` (a file's text plus line-start offsets, so chunks borrow whole lines without copying), 60-line windows and Markdown ATX sections. `code` runs one tree-sitter query per language (`chunk/queries/<language>.scm`); the query captures each definition as `@function` or `@type` with its `@name` (plus `@scope` for a Rust `impl` block and `@receiver` for a Go method), and Rust code turns captures into chunks and SCIP descriptors by walking node ancestry and matching on node kinds. The public `Chunker` owns one `tree_sitter::Parser` and the compiled queries, so the index pipeline (#21) builds it once and reuses it for every file. `Chunker::chunk` is infallible: a file tree-sitter cannot parse, a file in another language, and every line outside a definition or section all fall back to windows, so no text is unsearchable. Nothing here touches the store; #21 writes chunks, symbols and terms.

**Tech Stack:** Rust 1.99.1 (edition 2024). New dependencies, all MIT, in `[workspace.dependencies]` and checked by `cargo deny` (prototyped: no duplicate versions, deny passes): `tree-sitter` 0.27, `tree-sitter-rust` 0.24, `tree-sitter-python` 0.25, `tree-sitter-typescript` 0.23, `tree-sitter-javascript` 0.25, `tree-sitter-go` 0.25, `tree-sitter-c-sharp` 0.23, `streaming-iterator` 0.1 (tree-sitter's `QueryCursor::matches` returns a `StreamingIterator`). The grammar crates compile C with `cc`, which the existing toolchain already provides.

**Spec:** Issue #20; `docs/design.md#data-model` (the `chunk` and `symbol` tables, SCIP symbol IDs) and `#indexing-and-retrieval` step 3.

## Global Constraints

- One chunk per function, method, class, struct, enum, trait or interface, with 1-based inclusive line ranges and the symbol's name. Chunk kinds are the schema's: definitions of functions and methods are `function`; classes, structs, enums, traits, interfaces are `class`; Markdown sections `section`; everything else `window`.
- Definitions nest: a method chunk lies inside its class chunk, and both are returned. Chunks come back sorted by start line, then longest first.
- A Rust `impl` block is a `class` chunk with no symbol: it holds the `impl Display for Client` text and names the methods inside it (`Client#fmt().`).
- Python decorators belong to the definition they decorate: the chunk starts at the first decorator.
- Every symbol has a SCIP descriptor inside its file (`Client#validate().`, `helper().inner().`, Go `Client#Validate().` from the receiver). `Symbol::scip_id(source, path)` makes the full ID: `codecite . <source> . <path segments as namespaces><descriptor>`, e.g. `` codecite . identity-model . src/`client.rs`/Client#validate(). ``. SCIP grammar: names outside `[A-Za-z0-9_+$-]` go in backticks with backticks doubled; spaces in the package name are doubled.
- IDs are unique within a file: a later function with the same descriptor gets a SCIP disambiguator (`new(+1).`); a later type with the same name (`#[cfg]` twins) keeps its chunk but gets no symbol.
- Markdown: ATX headings `#` to `######` (up to three leading spaces; a space, tab or end of line after the `#`s; an optional closing `#` run). Headings inside fenced code (```` ``` ```` or `~~~`, closed only by the same marker at least as long) don't count. A section runs to the line before the next heading of any level, trailing blank lines dropped. Setext headings are not recognised (known limit, see Review Focus).
- Windows: every run of lines outside all definition and section chunks, cut every 60 lines. A window never starts or ends on a line with no letter or digit, so a lone `}` or blank run makes no window.
- Language detection: extension first (case-insensitive; `rs`, `py`/`pyi`, `ts`/`mts`/`cts`, `tsx`, `js`/`mjs`/`cjs`/`jsx`, `go`, `cs`, `md`/`markdown`; a dotfile like `.rs` has no extension), then a shebang on line 1 (`python*`, `node`/`nodejs`, `ts-node`, directly or through `env`, skipping `env` flags such as `-S`).
- No byte slicing except at line starts recorded from `\n` positions and at ASCII marker runs, so multibyte text never panics. Chunk text excludes the last line's `\n` or `\r\n`.
- No panics on any input: tree-sitter line rows are clamped to the file's lines, and a parent/child pair with the same span can't index forwards.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- Every query compiles against its grammar: `Chunker::new()` returns `ChunkerError` otherwise, and `every_query_compiles` (Task 3) checks it.
- Per-language fixtures pin names, kinds, descriptors and line ranges: `rust_fixture`, `python_fixture`, `typescript_fixture` (also run as TSX), `javascript_fixture`, `go_fixture`, `csharp_fixture` (Task 3).
- Duplicate names never repeat an ID: `duplicate_names_keep_ids_unique` (Task 3).
- No text goes missing: lines outside definitions become windows (`window` rows in every fixture test); a file with a syntax error still chunks (`broken_or_empty_code_still_chunks`, Task 3).
- Markdown splits at headings and ignores `#` in fences: `markdown_splits_at_headings`, `a_fence_closes_only_with_its_own_marker` (Task 2). A long text file splits into 60-line windows: `long_text_splits_into_sixty_line_windows` (Task 2).
- Known limits, not bugs: Setext headings (`Title` over `===`) are not sections, since front matter's closing `---` would read as one; sections are not cut at 60 lines; doc comments and attributes above a definition land in a window, not in its chunk (a Python decorator is the exception, being part of the definition's syntax); C# namespaces and Rust `mod`s are not part of the descriptor (the path already scopes it, and the disambiguator keeps IDs unique); a JavaScript or TypeScript arrow function counts only when assigned with `const`/`let`/`var` to a name.

Prototyped in full before planning (`/var/tmp/ts-proto`): every test below passes, clippy and fmt are clean on 1.99.1, and `Chunker` ran over about 28,000 real files (Python, Rust, TypeScript, TSX, Go, C#, JavaScript, Markdown) from `~/repos` and `~/.cargo/registry` without a panic.

---

### Task 1: Detect a file's language

**Files:**
- Create: `crates/codecite-index/src/chunk.rs` (module doc and `mod language; pub use language::Language;` only)
- Create: `crates/codecite-index/src/chunk/language.rs`
- Modify: `crates/codecite-index/src/lib.rs` (`mod chunk; pub use chunk::Language;`)

**Interfaces:**
- Produces: `pub enum Language { Rust, Python, TypeScript, Tsx, JavaScript, Go, CSharp, Markdown }` (`#[non_exhaustive]`), `Language::detect(path: &str, text: &str) -> Option<Language>`, `Language::name(self) -> &'static str` (the `document.language` value).

- [ ] **Step 1: Write the failing tests** at the bottom of `language.rs`

```rust
use super::*;

#[test]
fn extensions_pick_the_language() {
    let cases = [
        ("src/lib.rs", Language::Rust),
        ("app/main.py", Language::Python),
        ("types.pyi", Language::Python),
        ("src/index.ts", Language::TypeScript),
        ("src/util.mts", Language::TypeScript),
        ("src/App.tsx", Language::Tsx),
        ("lib/a.js", Language::JavaScript),
        ("lib/a.cjs", Language::JavaScript),
        ("lib/View.jsx", Language::JavaScript),
        ("cmd/main.go", Language::Go),
        ("Client.cs", Language::CSharp),
        ("README.md", Language::Markdown),
        ("docs/Guide.MARKDOWN", Language::Markdown),
    ];
    for (path, language) in cases {
        assert_eq!(Language::detect(path, ""), Some(language), "{path}");
    }
}

#[test]
fn unknown_or_hidden_files_have_no_language() {
    for path in [
        "notes.txt",
        "Makefile",
        ".rs",
        "dir.rs/README",
        "archive.tar.gz",
    ] {
        assert_eq!(Language::detect(path, "fn main() {}"), None, "{path}");
    }
}

#[test]
fn shebang_names_the_language_of_a_file_without_an_extension() {
    let cases = [
        ("#!/usr/bin/env python3\nprint(1)\n", Some(Language::Python)),
        ("#!/usr/bin/python3.12\n", Some(Language::Python)),
        ("#!/usr/bin/env node\n", Some(Language::JavaScript)),
        (
            "#!/usr/bin/env -S node --no-warnings\n",
            Some(Language::JavaScript),
        ),
        ("#!/usr/local/bin/ts-node\n", Some(Language::TypeScript)),
        ("#!/bin/sh\necho hi\n", None),
        ("#!/usr/bin/env\n", None),
        ("print(1)\n#!/usr/bin/env python3\n", None),
    ];
    for (text, language) in cases {
        assert_eq!(Language::detect("bin/tool", text), language, "{text:?}");
    }
}

#[test]
fn extension_wins_over_shebang() {
    assert_eq!(
        Language::detect("build.rs", "#!/usr/bin/env python3\n"),
        Some(Language::Rust)
    );
}
```

(inside `#[cfg(test)] mod tests { ... }`, as in every task below.)

- [ ] **Step 2: Run them and watch them fail to compile**

Run: `cargo test -p codecite-index chunk::language`
Expected: error, `Language` not found.

- [ ] **Step 3: Implement `language.rs`**

```rust
//! Language detection by file extension, then by shebang.

/// A language the chunker recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Language {
    /// Rust (`.rs`).
    Rust,
    /// Python (`.py`, `.pyi`, or a `python` shebang).
    Python,
    /// TypeScript (`.ts`, `.mts`, `.cts`, or a `ts-node` shebang).
    TypeScript,
    /// TypeScript with JSX (`.tsx`).
    Tsx,
    /// JavaScript, JSX included (`.js`, `.mjs`, `.cjs`, `.jsx`, or a `node` shebang).
    JavaScript,
    /// Go (`.go`).
    Go,
    /// C# (`.cs`).
    CSharp,
    /// Markdown (`.md`, `.markdown`).
    Markdown,
}

impl Language {
    /// Detects the language of `path` from its extension, or, for a file
    /// with no known extension, from the shebang on the first line of `text`.
    pub fn detect(path: &str, text: &str) -> Option<Self> {
        Self::from_extension(path).or_else(|| Self::from_shebang(text))
    }

    /// The name stored in `document.language`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::CSharp => "csharp",
            Self::Markdown => "markdown",
        }
    }

    fn from_extension(path: &str) -> Option<Self> {
        let file = path.rsplit('/').next().unwrap_or(path);
        let (stem, ext) = file.rsplit_once('.')?;
        if stem.is_empty() {
            return None; // `.rs` alone is a hidden file, not a Rust file
        }
        Some(match ext.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "go" => Self::Go,
            "cs" => Self::CSharp,
            "md" | "markdown" => Self::Markdown,
            _ => return None,
        })
    }

    fn from_shebang(text: &str) -> Option<Self> {
        let line = text.lines().next()?.strip_prefix("#!")?;
        let mut words = line.split_whitespace();
        let mut program = words.next()?.rsplit('/').next()?;
        if program == "env" {
            // `#!/usr/bin/env -S node --flag`: the first word that isn't a flag.
            program = words.find(|w| !w.starts_with('-'))?;
        }
        match program {
            "node" | "nodejs" => Some(Self::JavaScript),
            "ts-node" => Some(Self::TypeScript),
            p if p.starts_with("python") => Some(Self::Python),
            _ => None,
        }
    }
}
```

`chunk.rs` for now:

```rust
//! Chunking: splits a file into the spans that search returns. Code in a
//! supported language splits at definitions with tree-sitter, Markdown at
//! headings, and everything else into 60-line windows.

mod language;

pub use language::Language;
```

In `lib.rs`, add `mod chunk;` and `pub use chunk::Language;` above `mod error;`.

> Learn: `rsplit_once`, `let ... else`, and `Option::or_else` keep detection to a few lines with no indexing; `match` on `ext.to_ascii_lowercase().as_str()` turns an owned `String` into a `&str` you can match string literals against, and `return None` inside a match arm leaves the whole function early. The `p if p.starts_with("python")` arm is a match guard.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test -p codecite-index chunk::language`
Expected: 4 passed.

- [ ] **Step 5: Gates and commit**

Run the gates. Commit `feat(index): detect a file's language by extension and shebang (#20)`.

---

### Task 2: Markdown sections and 60-line windows

**Files:**
- Create: `crates/codecite-index/src/chunk/text.rs`
- Modify: `crates/codecite-index/src/chunk.rs` (the chunk types and a `Chunker` without tree-sitter)
- Modify: `crates/codecite-index/src/lib.rs` (re-export the new types)

**Interfaces:**
- Consumes: `Language` (Task 1).
- Produces: `pub const WINDOW_LINES: u32 = 60`, `pub enum ChunkKind { Function, Class, Section, Window }` with `as_str`, `pub enum SymbolKind { Function, Method, Class, Struct, Enum, Trait, Interface }` with `as_str`, `pub struct Symbol { name, kind, descriptor }`, `pub struct Chunk<'a> { kind, start_line, end_line, text: &'a str, symbol: Option<Symbol>, title: Option<String> }`, and `Chunker::new() -> Chunker` / `Chunker::chunk<'a>(&mut self, path: &str, text: &'a str) -> Vec<Chunk<'a>>`. Task 3 changes `new` to return `Result<Chunker, ChunkerError>`.

- [ ] **Step 1: Write the failing tests**

In `text.rs`:

```rust
use super::*;

fn spans(chunks: &[Chunk<'_>]) -> Vec<(u32, u32)> {
    chunks.iter().map(|c| (c.start_line, c.end_line)).collect()
}

#[test]
fn lines_slice_without_line_endings() {
    let lines = Lines::new("one\r\ntwo\n\nfour");
    assert_eq!(lines.len(), 4);
    assert_eq!(lines.line(1), "one");
    assert_eq!(lines.line(3), "");
    assert_eq!(lines.span(1, 2), "one\r\ntwo");
    assert_eq!(lines.span(4, 4), "four");
    // A final newline doesn't add an empty line.
    assert_eq!(Lines::new("a\nb\n").len(), 2);
    assert_eq!(Lines::new("").len(), 0);
}

#[test]
fn long_text_splits_into_sixty_line_windows() {
    let text: String = (1..=130).map(|i| format!("line {i}\n")).collect();
    let lines = Lines::new(&text);
    let chunks = windows(&lines, &vec![false; lines.len()]);
    assert_eq!(spans(&chunks), [(1, 60), (61, 120), (121, 130)]);
    assert!(chunks.iter().all(|c| c.kind == ChunkKind::Window));
    assert_eq!(chunks[2].text.lines().next(), Some("line 121"));
    assert!(chunks[2].text.ends_with("line 130"));
}

#[test]
fn windows_skip_covered_lines_and_trim_lines_without_words() {
    let text = "a\n\n}\nb\nc\n\n";
    let lines = Lines::new(text);
    let covered = [false, false, false, true, false, false];
    assert_eq!(spans(&windows(&lines, &covered)), [(1, 1), (5, 5)]);
    assert!(windows(&Lines::new("\n  \n}\n"), &[false; 3]).is_empty());
}

#[test]
fn markdown_splits_at_headings() {
    let text = "Intro.\n\n# Title\n\nText.\n\n## Install ##\n\n```sh\n# not a heading\n```\n\n#hashtag\n### Use\nRun it.\n";
    let lines = Lines::new(text);
    let chunks = sections(&lines);
    let got: Vec<_> = chunks
        .iter()
        .map(|c| (c.start_line, c.end_line, c.title.as_deref().unwrap()))
        .collect();
    assert_eq!(got, [(3, 5, "Title"), (7, 13, "Install"), (14, 15, "Use")]);
    assert!(chunks.iter().all(|c| c.kind == ChunkKind::Section));
    assert_eq!(chunks[2].text, "### Use\nRun it.");
}

#[test]
fn atx_heading_rules() {
    assert_eq!(atx_heading("# A").as_deref(), Some("A"));
    assert_eq!(atx_heading("###### Six").as_deref(), Some("Six"));
    assert_eq!(atx_heading("####### Seven"), None);
    assert_eq!(atx_heading("#NoSpace"), None);
    assert_eq!(atx_heading("#").as_deref(), Some(""));
    assert_eq!(atx_heading("## C# ##").as_deref(), Some("C#"));
    assert_eq!(atx_heading("## Issue #12").as_deref(), Some("Issue #12"));
}

#[test]
fn a_fence_closes_only_with_its_own_marker() {
    let text = "~~~\n```\n# inside\n~~~\n# Outside\n";
    let chunks = sections(&Lines::new(text));
    assert_eq!(spans(&chunks), [(5, 5)]);
}
```

In `chunk.rs`:

```rust
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
```

- [ ] **Step 2: Run them and watch them fail to compile**

Run: `cargo test -p codecite-index chunk::`
Expected: errors, `Lines`, `windows`, `sections`, `Chunker` not found.

- [ ] **Step 3: Implement `text.rs`**

```rust
//! Line bookkeeping, 60-line windows and Markdown heading sections.

use super::{Chunk, ChunkKind, WINDOW_LINES};

/// A file's text and the byte offset where each line starts. Every offset
/// sits just after a `\n`, so slicing at them never splits a character.
pub(super) struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        // A final newline ends the last line; it doesn't start an empty one.
        if starts.last() == Some(&text.len()) {
            starts.pop();
        }
        Self { text, starts }
    }

    /// The number of lines.
    pub(super) fn len(&self) -> usize {
        self.starts.len()
    }

    /// Line `n` (from 1), without its line ending.
    pub(super) fn line(&self, n: u32) -> &'a str {
        self.span(n, n)
    }

    /// Lines `start..=end` (from 1), without the last line's ending.
    pub(super) fn span(&self, start: u32, end: u32) -> &'a str {
        let from = self.starts[start as usize - 1];
        let to = self
            .starts
            .get(end as usize)
            .copied()
            .unwrap_or(self.text.len());
        let span = &self.text[from..to];
        let span = span.strip_suffix('\n').unwrap_or(span);
        span.strip_suffix('\r').unwrap_or(span)
    }

    fn is_blank(&self, n: u32) -> bool {
        self.line(n).trim().is_empty()
    }

    /// True when line `n` has no letter or digit: blank, or only braces
    /// and punctuation.
    fn has_no_words(&self, n: u32) -> bool {
        !self.line(n).chars().any(char::is_alphanumeric)
    }
}

/// Windows over every run of lines that `covered` marks false, cut every
/// [`WINDOW_LINES`] lines. A window never starts or ends on a line without
/// a letter or digit, so a lone `}` between definitions makes no window.
pub(super) fn windows<'a>(lines: &Lines<'a>, covered: &[bool]) -> Vec<Chunk<'a>> {
    let mut out = Vec::new();
    let mut n = 1;
    let last = lines.len() as u32;
    while n <= last {
        if covered[n as usize - 1] || lines.has_no_words(n) {
            n += 1;
            continue;
        }
        // `n` starts a window: take up to WINDOW_LINES uncovered lines,
        // then drop the trailing lines without words.
        let mut end = n;
        while end < last && end - n + 1 < WINDOW_LINES && !covered[end as usize] {
            end += 1;
        }
        let next = end + 1;
        while lines.has_no_words(end) {
            end -= 1;
        }
        out.push(Chunk {
            kind: ChunkKind::Window,
            start_line: n,
            end_line: end,
            text: lines.span(n, end),
            symbol: None,
            title: None,
        });
        n = next;
    }
    out
}

/// One section per ATX heading (`#` to `######`), running to the line
/// before the next heading. Headings inside fenced code blocks don't count.
/// Trailing blank lines are dropped from each section.
pub(super) fn sections<'a>(lines: &Lines<'a>) -> Vec<Chunk<'a>> {
    let mut headings = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for n in 1..=lines.len() as u32 {
        let line = lines.line(n);
        let indented = line.len() - line.trim_start_matches(' ').len();
        let body = line.trim_start_matches(' ');
        if indented > 3 {
            continue;
        }
        if let Some(marker @ ('`' | '~')) = body.chars().next() {
            let run = body.chars().take_while(|&c| c == marker).count();
            if run >= 3 {
                match fence {
                    None => fence = Some((marker, run)),
                    Some((open, len))
                        if open == marker && run >= len && body[run..].trim().is_empty() =>
                    {
                        fence = None
                    }
                    Some(_) => {}
                }
                continue;
            }
        }
        if fence.is_none()
            && let Some(title) = atx_heading(body)
        {
            headings.push((n, title));
        }
    }

    let mut out = Vec::new();
    for (i, (start, title)) in headings.iter().enumerate() {
        let mut end = headings
            .get(i + 1)
            .map_or(lines.len() as u32, |&(next, _)| next - 1);
        while end > *start && lines.is_blank(end) {
            end -= 1;
        }
        out.push(Chunk {
            kind: ChunkKind::Section,
            start_line: *start,
            end_line: end,
            text: lines.span(*start, end),
            symbol: None,
            title: Some(title.clone()),
        });
    }
    out
}

/// The text of an ATX heading line, or `None` when `line` isn't one.
fn atx_heading(line: &str) -> Option<String> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let rest = &line[level..];
    if !(1..=6).contains(&level) || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    // A closing run of `#`s is dropped when a space precedes it.
    let rest = rest.trim();
    let without_close = rest.trim_end_matches('#');
    let title = if without_close.is_empty() || without_close.ends_with([' ', '\t']) {
        without_close.trim_end()
    } else {
        rest
    };
    Some(title.to_string())
}
```

> Learn: `Lines<'a>` and `Chunk<'a>` carry a lifetime because they borrow the file's text instead of copying it. `'a` says "this lives no longer than the `&'a str` it was built from"; the compiler then refuses any use of a chunk after its text is dropped. Methods return `&'a str` (tied to the text), not `&str` (tied to `&self`), so a chunk can outlive the temporary `Lines` that sliced it. `strip_suffix(...).unwrap_or(span)` removes a line ending only when it is there.

> Learn: `` Some(marker @ ('`' | '~')) `` binds the matched char to `marker` while matching either value; `if fence.is_none() && let Some(title) = atx_heading(body)` is a let chain (edition 2024).

- [ ] **Step 4: Replace `chunk.rs` with the chunk types and a Markdown-and-windows `Chunker`**

```rust
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
```

Re-export from `lib.rs`: `pub use chunk::{Chunk, ChunkKind, Chunker, Language, Symbol, SymbolKind, WINDOW_LINES};`.

> Learn: `Chunker::chunk` takes `text: &'a str` and returns `Vec<Chunk<'a>>`, but `path: &str` gets no `'a`: the output borrows from `text` only, and the signature says so. `covered[a..b].fill(true)` marks a slice in one call; `sort_by_key` with `std::cmp::Reverse` sorts one key ascending and the other descending.

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo test -p codecite-index chunk::`
Expected: 12 passed (4 language, 6 text, 2 chunk).

- [ ] **Step 6: Gates and commit**

Run the gates. Commit `feat(index): split Markdown at headings and other text into 60-line windows (#20)`.

---

### Task 3: Chunk code with tree-sitter

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`), `crates/codecite-index/Cargo.toml`
- Create: `crates/codecite-index/src/chunk/code.rs`
- Create: `crates/codecite-index/src/chunk/queries/{rust,python,typescript,javascript,go,csharp}.scm`
- Create: `crates/codecite-index/tests/fixtures/chunk/sample.{rs,py,ts,js,go,cs}`
- Modify: `crates/codecite-index/src/chunk.rs`, `crates/codecite-index/src/chunk/text.rs` (add `Lines::text`), `crates/codecite-index/src/lib.rs`, `crates/codecite-index/src/error.rs`
- Modify: `docs/design.md` (data model bullet on symbol IDs; indexing step 3)

**Interfaces:**
- Consumes: Tasks 1 and 2.
- Produces: `Chunker::new() -> Result<Chunker, ChunkerError>`, `pub enum ChunkerError { Grammar { language, source: tree_sitter::LanguageError }, Query { language, source: tree_sitter::QueryError } }`, `Symbol::scip_id(&self, source: &str, path: &str) -> String`, `Error::Chunker(#[from] ChunkerError)` in the crate error. #21 consumes all of this.

- [ ] **Step 1: Add the dependencies**

In the root `Cargo.toml`, under `[workspace.dependencies]`:

```toml
tree-sitter = "0.27"
tree-sitter-rust = "0.24"
tree-sitter-python = "0.25"
tree-sitter-typescript = "0.23"
tree-sitter-javascript = "0.25"
tree-sitter-go = "0.25"
tree-sitter-c-sharp = "0.23"
streaming-iterator = "0.1"
```

In `crates/codecite-index/Cargo.toml` `[dependencies]`, each as `<name>.workspace = true`. Run `cargo deny check`: expect `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 2: Add the fixtures** under `crates/codecite-index/tests/fixtures/chunk/` (a subdirectory of `tests/`, so Cargo does not treat them as integration tests). Byte-exact, since the tests pin line numbers:

`sample.rs`:

```rust
//! A sample module.

use std::fmt;

/// A client.
pub struct Client {
    issuer: String,
}

impl Client {
    pub fn new(issuer: &str) -> Self {
        Self { issuer: issuer.into() }
    }

    pub fn validate(&self, token: &str) -> bool {
        !token.is_empty()
    }
}

impl fmt::Display for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.issuer)
    }
}

pub enum Grant {
    Code,
    Implicit,
}

pub trait Verifier {
    fn verify(&self) -> bool;
}

fn helper() -> u32 {
    fn inner() -> u32 {
        1
    }
    inner()
}
```

`sample.py`:

```python
#!/usr/bin/env python3
"""A sample module."""

import os


class Client:
    def __init__(self, issuer):
        self.issuer = issuer

    @staticmethod
    def validate(token):
        return bool(token)


def helper():
    def inner():
        return 1

    return inner()
```

`sample.ts`:

```typescript
import { readFile } from "fs";

export interface Claims {
  sub: string;
}

export enum Grant {
  Code,
  Implicit,
}

export class Client {
  constructor(private issuer: string) {}

  validate(token: string): boolean {
    return token.length > 0;
  }
}

export function helper(): number {
  return 1;
}

export const parse = (raw: string): Claims => {
  return JSON.parse(raw);
};
```

`sample.js`:

```javascript
#!/usr/bin/env node
"use strict";

class Client {
  validate(token) {
    return token.length > 0;
  }
}

function helper() {
  return 1;
}

module.exports = { Client, helper };
```

`sample.go`:

```go
package sample

import "strings"

// Client validates tokens.
type Client struct {
	Issuer string
}

type Verifier interface {
	Verify() bool
}

func NewClient(issuer string) *Client {
	return &Client{Issuer: issuer}
}

func (c *Client) Validate(token string) bool {
	return strings.TrimSpace(token) != ""
}
```

`sample.cs`:

```csharp
using System;

namespace Sample
{
    public interface IVerifier
    {
        bool Verify();
    }

    public enum Grant
    {
        Code,
        Implicit,
    }

    public class Client
    {
        public Client(string issuer)
        {
            Issuer = issuer;
        }

        public string Issuer { get; }

        public bool Validate(string token)
        {
            return token.Length > 0;
        }
    }
}
```

- [ ] **Step 3: Write the failing tests** in `chunk.rs` (keep Task 2's tests; change `outline`'s first line to `let mut chunker = Chunker::new().unwrap();`)

```rust
// added to the existing `mod tests`

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
```

- [ ] **Step 4: Run them and watch them fail**

Run: `cargo test -p codecite-index chunk::`
Expected: compile errors (`Chunker::new` returns no `Result`, `scip_id` and `code` missing).

- [ ] **Step 5: Write the queries** in `crates/codecite-index/src/chunk/queries/`

> Learn: a tree-sitter query is an S-expression pattern over the syntax tree. `(function_item name: (identifier) @name) @function` matches a `function_item` node whose `name` field is an `identifier`, and captures the identifier as `@name` and the whole item as `@function`. `[a b]` is an alternation, `(_)` any named node, `;` a comment. Node and field names come from each grammar's `node-types.json`; `Query::new` rejects any name the grammar lacks, which is what `every_query_compiles` checks. TSX reuses `typescript.scm`.

`rust.scm`:

```scheme
; Definitions that become chunks: @function or @type, each with its @name.
; An `impl` block is a @scope: its methods are named after its type.
(function_item name: (identifier) @name) @function
(function_signature_item name: (identifier) @name) @function
(struct_item name: (type_identifier) @name) @type
(enum_item name: (type_identifier) @name) @type
(union_item name: (type_identifier) @name) @type
(trait_item name: (type_identifier) @name) @type
(impl_item type: (_) @name) @scope
```

`python.scm`:

```scheme
; Definitions that become chunks: @function or @type, each with its @name.
(function_definition name: (identifier) @name) @function
(class_definition name: (identifier) @name) @type
```

`typescript.scm`:

```scheme
; Definitions that become chunks: @function or @type, each with its @name.
(function_declaration name: (identifier) @name) @function
(generator_function_declaration name: (identifier) @name) @function
(function_signature name: (identifier) @name) @function
(method_definition name: (_) @name) @function
(abstract_method_signature name: (_) @name) @function
(variable_declarator
  name: (identifier) @name
  value: [(arrow_function) (function_expression)]) @function
(class_declaration name: (_) @name) @type
(abstract_class_declaration name: (_) @name) @type
(interface_declaration name: (_) @name) @type
(enum_declaration name: (_) @name) @type
```

`javascript.scm`:

```scheme
; Definitions that become chunks: @function or @type, each with its @name.
(function_declaration name: (identifier) @name) @function
(generator_function_declaration name: (identifier) @name) @function
(method_definition name: (_) @name) @function
(variable_declarator
  name: (identifier) @name
  value: [(arrow_function) (function_expression)]) @function
(class_declaration name: (_) @name) @type
```

`go.scm`:

```scheme
; Definitions that become chunks: @function or @type, each with its @name.
; A method's @receiver names the type it belongs to.
(function_declaration name: (identifier) @name) @function
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [(type_identifier) @receiver
             (pointer_type (type_identifier) @receiver)
             (generic_type type: (type_identifier) @receiver)
             (pointer_type (generic_type type: (type_identifier) @receiver))]))
  name: (field_identifier) @name) @function
(type_spec name: (type_identifier) @name type: [(struct_type) (interface_type)]) @type
```

`csharp.scm`:

```scheme
; Definitions that become chunks: @function or @type, each with its @name.
(method_declaration name: (identifier) @name) @function
(constructor_declaration name: (identifier) @name) @function
(local_function_statement name: (identifier) @name) @function
(class_declaration name: (identifier) @name) @type
(struct_declaration name: (identifier) @name) @type
(interface_declaration name: (identifier) @name) @type
(enum_declaration name: (identifier) @name) @type
(record_declaration name: (identifier) @name) @type
```

- [ ] **Step 6: Implement `code.rs`**

```rust
//! Code chunking with tree-sitter: one chunk per definition that a
//! language's query (`queries/<language>.scm`) captures.

use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

use super::text::Lines;
use super::{Chunk, ChunkKind, Language, Symbol, SymbolKind};

/// A bundled grammar or query failed to load: a bug in codecite, or a
/// grammar crate built for an incompatible tree-sitter.
#[derive(Debug, thiserror::Error)]
pub enum ChunkerError {
    /// The grammar's ABI doesn't match the tree-sitter library.
    #[error("cannot load the {language} grammar")]
    Grammar {
        /// The language whose grammar failed.
        language: &'static str,
        /// What tree-sitter reported.
        #[source]
        source: tree_sitter::LanguageError,
    },
    /// The language's query doesn't compile against its grammar.
    #[error("invalid {language} query")]
    Query {
        /// The language whose query failed.
        language: &'static str,
        /// What tree-sitter reported.
        #[source]
        source: tree_sitter::QueryError,
    },
}

/// Loads a grammar from its crate.
type Grammar = fn() -> tree_sitter::Language;

/// The code languages, each with its grammar and query. TSX shares the
/// TypeScript query: its grammar has the same node kinds, plus JSX.
const LANGUAGES: [(Language, Grammar, &str); 7] = [
    (
        Language::Rust,
        || tree_sitter_rust::LANGUAGE.into(),
        include_str!("queries/rust.scm"),
    ),
    (
        Language::Python,
        || tree_sitter_python::LANGUAGE.into(),
        include_str!("queries/python.scm"),
    ),
    (
        Language::TypeScript,
        || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        include_str!("queries/typescript.scm"),
    ),
    (
        Language::Tsx,
        || tree_sitter_typescript::LANGUAGE_TSX.into(),
        include_str!("queries/typescript.scm"),
    ),
    (
        Language::JavaScript,
        || tree_sitter_javascript::LANGUAGE.into(),
        include_str!("queries/javascript.scm"),
    ),
    (
        Language::Go,
        || tree_sitter_go::LANGUAGE.into(),
        include_str!("queries/go.scm"),
    ),
    (
        Language::CSharp,
        || tree_sitter_c_sharp::LANGUAGE.into(),
        include_str!("queries/csharp.scm"),
    ),
];

pub(super) struct CodeChunker {
    parser: Parser,
    /// Each code language's grammar and compiled query.
    grammars: HashMap<Language, (tree_sitter::Language, Query)>,
}

impl CodeChunker {
    pub(super) fn new() -> Result<Self, ChunkerError> {
        let mut parser = Parser::new();
        let mut grammars = HashMap::new();
        for (language, grammar, source) in LANGUAGES {
            let name = language.name();
            let grammar = grammar();
            // Setting each grammar once checks its ABI up front.
            parser
                .set_language(&grammar)
                .map_err(|source| ChunkerError::Grammar {
                    language: name,
                    source,
                })?;
            let query = Query::new(&grammar, source).map_err(|source| ChunkerError::Query {
                language: name,
                source,
            })?;
            grammars.insert(language, (grammar, query));
        }
        Ok(Self { parser, grammars })
    }

    /// One chunk per captured definition or `impl` block, ordered by
    /// position. A language without a grammar (Markdown), or a file the
    /// parser gives up on, gives no chunks, so the file falls back to
    /// windows.
    pub(super) fn chunk<'a>(&mut self, language: Language, lines: &Lines<'a>) -> Vec<Chunk<'a>> {
        let Some((grammar, query)) = self.grammars.get(&language) else {
            return Vec::new();
        };
        if self.parser.set_language(grammar).is_err() {
            return Vec::new();
        }
        let source = lines.text().as_bytes();
        let Some(tree) = self.parser.parse(source, None) else {
            return Vec::new();
        };

        let defs = definitions(query, tree.root_node(), source, lines.len() as u32);
        // Each definition's full descriptor, built from its innermost
        // parent's, which comes earlier in `defs`.
        let mut full: Vec<String> = Vec::with_capacity(defs.len());
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut out = Vec::new();
        for def in &defs {
            let parent = def.parents.last().map(|&p| (&full[p], defs[p].kind));
            let mut prefix = parent.map_or_else(String::new, |(d, _)| d.clone());
            if let Some(receiver) = &def.receiver {
                prefix.push_str(&escape_name(receiver));
                prefix.push('#');
            }
            let Some(kind) = def.kind else {
                // An `impl` block: a chunk that names the methods inside it
                // but has no symbol of its own.
                full.push(prefix + &def.descriptor_part(0));
                out.push(Chunk {
                    kind: ChunkKind::Class,
                    start_line: def.start_line,
                    end_line: def.end_line,
                    text: lines.span(def.start_line, def.end_line),
                    symbol: None,
                    title: None,
                });
                continue;
            };
            let in_type = def.receiver.is_some()
                || parent.is_some_and(|(_, k)| {
                    !matches!(k, Some(SymbolKind::Function | SymbolKind::Method))
                });
            let kind = match kind {
                SymbolKind::Function if in_type => SymbolKind::Method,
                k => k,
            };
            let first = prefix.clone() + &def.descriptor_part(0);
            let n = *seen
                .entry(first.clone())
                .and_modify(|n| *n += 1)
                .or_insert(0);
            let descriptor = match (kind, n) {
                (_, 0) => Some(first.clone()),
                (SymbolKind::Function | SymbolKind::Method, n) => {
                    Some(prefix.clone() + &def.descriptor_part(n))
                }
                // A second type with the same name (behind a `cfg`, say) keeps
                // its chunk but gets no symbol, so IDs stay unique.
                _ => None,
            };
            full.push(descriptor.clone().unwrap_or(first));
            out.push(Chunk {
                kind: match kind {
                    SymbolKind::Function | SymbolKind::Method => ChunkKind::Function,
                    _ => ChunkKind::Class,
                },
                start_line: def.start_line,
                end_line: def.end_line,
                text: lines.span(def.start_line, def.end_line),
                symbol: descriptor.map(|descriptor| Symbol {
                    name: def.name.clone(),
                    kind,
                    descriptor,
                }),
                title: None,
            });
        }
        out
    }
}

/// A captured definition or scope.
struct Definition {
    /// `None` for a Rust `impl` block, which names the methods inside it
    /// but defines no symbol.
    kind: Option<SymbolKind>,
    name: String,
    /// A Go method's receiver type.
    receiver: Option<String>,
    start_line: u32,
    end_line: u32,
    /// Indices of the enclosing definitions and scopes, outermost first.
    parents: Vec<usize>,
}

impl Definition {
    /// This definition's own SCIP descriptor; `disambiguator` > 0 marks the
    /// n-th later function with the same name, e.g. `new(+1).`.
    fn descriptor_part(&self, disambiguator: u32) -> String {
        let name = escape_name(&self.name);
        match self.kind {
            Some(SymbolKind::Function | SymbolKind::Method) if disambiguator > 0 => {
                format!("{name}(+{disambiguator}).")
            }
            Some(SymbolKind::Function | SymbolKind::Method) => format!("{name}()."),
            _ => format!("{name}#"),
        }
    }
}

fn definitions(query: &Query, root: Node<'_>, source: &[u8], last: u32) -> Vec<Definition> {
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);
    let mut found: Vec<(Node<'_>, Definition)> = Vec::new();
    while let Some(m) = matches.next() {
        let mut node = None;
        let mut is_scope = false;
        let mut name = None;
        let mut receiver = None;
        for capture in m.captures() {
            match names[capture.index as usize] {
                "function" | "type" => node = Some(capture.node),
                "scope" => {
                    node = Some(capture.node);
                    is_scope = true;
                }
                "name" => name = Some(capture.node),
                "receiver" => receiver = Some(capture.node),
                _ => {}
            }
        }
        let (Some(node), Some(name)) = (node, name) else {
            continue;
        };
        let text = |n: Node<'_>| n.utf8_text(source).unwrap_or_default().to_string();
        let name = if is_scope { type_name(name) } else { name };
        // A Python decorator belongs to the definition it decorates.
        let span = match node.parent() {
            Some(p) if p.kind() == "decorated_definition" => p,
            _ => node,
        };
        let (start_line, end_line) = line_range(span, last);
        found.push((
            node,
            Definition {
                kind: if is_scope {
                    None
                } else {
                    Some(symbol_kind(node))
                },
                name: text(name),
                receiver: receiver.map(text),
                start_line,
                end_line,
                parents: Vec::new(),
            },
        ));
    }
    // Sort by position so every parent comes before what it encloses.
    found.sort_by_key(|(n, _)| (n.start_byte(), std::cmp::Reverse(n.end_byte())));
    let ids: HashMap<usize, usize> = found
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (n.id(), i))
        .collect();
    let mut defs = Vec::with_capacity(found.len());
    for (index, (node, mut def)) in found.into_iter().enumerate() {
        let mut parent = node.parent();
        while let Some(p) = parent {
            // `i < index` holds for every real parent; checking it keeps a
            // parent and child with the same span from pointing forwards.
            if let Some(&i) = ids.get(&p.id())
                && i < index
            {
                def.parents.push(i);
            }
            parent = p.parent();
        }
        def.parents.reverse();
        defs.push(def);
    }
    defs
}

/// The kind of a captured definition node.
fn symbol_kind(node: Node<'_>) -> SymbolKind {
    match node.kind() {
        "method_definition"
        | "method_declaration"
        | "abstract_method_signature"
        | "constructor_declaration" => SymbolKind::Method,
        "struct_item" | "union_item" | "struct_declaration" => SymbolKind::Struct,
        "enum_item" | "enum_declaration" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "interface_declaration" => SymbolKind::Interface,
        "class_definition"
        | "class_declaration"
        | "abstract_class_declaration"
        | "record_declaration" => SymbolKind::Class,
        // Go: `type Foo struct {...}` or `type Foo interface {...}`.
        "type_spec" => match node.child_by_field_name("type").map(|t| t.kind()) {
            Some("interface_type") => SymbolKind::Interface,
            _ => SymbolKind::Struct,
        },
        _ => SymbolKind::Function,
    }
}

/// The bare type name of a Rust `impl` target: `Foo` for `Foo<T>` or `a::Foo`.
fn type_name(mut node: Node<'_>) -> Node<'_> {
    loop {
        match node.kind() {
            "generic_type" => match node.child_by_field_name("type") {
                Some(t) => node = t,
                None => return node,
            },
            "scoped_type_identifier" => match node.child_by_field_name("name") {
                Some(n) => node = n,
                None => return node,
            },
            _ => return node,
        }
    }
}

/// A node's lines, from 1, inclusive, and never past `last`. A node that
/// ends at the start of a line ends on the line before.
fn line_range(node: Node<'_>, last: u32) -> (u32, u32) {
    let start = node.start_position().row;
    let end = node.end_position();
    let end_row = if end.column == 0 && end.row > start {
        end.row - 1
    } else {
        end.row
    };
    let start = (start as u32 + 1).min(last);
    (start, (end_row as u32 + 1).clamp(start, last))
}

/// A SCIP descriptor name: bare when it's only ASCII letters, digits, `_`, `+`,
/// `-` and `$`, otherwise in backticks with any backtick doubled.
pub(super) fn escape_name(name: &str) -> String {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '$'))
    {
        name.to_string()
    } else {
        format!("`{}`", name.replace('`', "``"))
    }
}
```

> Learn: `symbol_kind` is a `match` on `node.kind()`, the grammar's node-type string, with `|` joining the kinds of several languages into one arm; Go's `type_spec` looks one level down with `child_by_field_name`. `QueryCursor::matches` returns a `StreamingIterator` (each match borrows the cursor, so it is `while let Some(m) = matches.next()`, not a `for` loop). `Node<'tree>` borrows the `Tree`, which is why `definitions` copies names into `String`s before the tree is dropped. `type Grammar = fn() -> tree_sitter::Language` is a function-pointer type; the non-capturing closures in `LANGUAGES` coerce to it, which lets the table be a `const`.

In `text.rs`, add to `impl Lines`:

```rust
    /// The whole text.
    pub(super) fn text(&self) -> &'a str {
        self.text
    }
```

- [ ] **Step 7: Replace `chunk.rs`'s code above the tests with the final version**

```rust
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
```

- [ ] **Step 8: Wire up the crate**

`lib.rs`, top of the file (the walk and manifest lines stay as they are):

```rust
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
```

`error.rs`, a new variant after `Walk`:

```rust
    /// A bundled tree-sitter grammar or query failed to load.
    #[error(transparent)]
    Chunker(#[from] crate::ChunkerError),
```

- [ ] **Step 9: Run the tests and watch them pass**

Run: `cargo test -p codecite-index chunk::`
Expected: 26 passed.

- [ ] **Step 10: Update `docs/design.md`**

In `## Data model`, replace the Symbol IDs bullet's second sentence with: "Symbols found only by tree-sitter get SCIP-shaped local IDs, `codecite . <source> . <path segments>/<descriptors>` (e.g. `` codecite . identity-model . src/`client.rs`/Client#validate(). ``), with a SCIP disambiguator on a later same-named function in the file (`new(+1).`); they are replaced when a SCIP index for the same source arrives."

In `## Indexing and retrieval`, replace step 3's first two sentences with: "**Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit." Keep the spec-document sentence.

Check `README.md` and `AGENTS.md` for any statement about chunking that now disagrees, and fix it in this commit.

- [ ] **Step 11: Adversarial check**

Build a throwaway example (not committed) that runs `Chunker::chunk` over every `git ls-files` path of this repo and of a checkout with TypeScript, Go or C# under `~/repos`, reading with `fs::read_to_string` and skipping non-UTF-8 files. Expect no panic. Try a file that is only `}`, a file with no final newline, and a CRLF Markdown file.

- [ ] **Step 12: Gates and commit**

Run the gates. Commit `feat(index): chunk Rust, Python, TypeScript, Go and C# with tree-sitter (#20)`.
