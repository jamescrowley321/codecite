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

/// The deepest a definition can sit inside other captured definitions and
/// still get its own chunk. Each nested chunk repeats its parents' lines and
/// descriptors, so without a limit a file of nested functions costs memory
/// quadratic in its depth; deeper definitions stay in their parent's chunk.
pub(super) const MAX_DEPTH: usize = 32;

/// The longest descriptor a definition can have and still get a symbol.
/// Each descriptor copies its parent's, so a long name would otherwise be
/// copied into every definition inside it. A definition whose descriptor is
/// longer keeps its chunk, and neither it nor anything inside it gets a
/// symbol.
pub(super) const MAX_DESCRIPTOR: usize = 512;

/// The most descriptor bytes one file can hold. With [`MAX_DESCRIPTOR`] this
/// bounds a file's descriptor memory by its size; once the budget is spent,
/// later definitions keep their chunks but get no symbol.
pub(super) const DESCRIPTOR_BUDGET: usize = 8 << 20;

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

        let defs = definitions(
            language,
            query,
            tree.root_node(),
            source,
            lines.len() as u32,
        );
        // Each definition's full descriptor, built from its innermost
        // parent's, which comes earlier in `defs`; `None` when it doesn't
        // fit (see [`MAX_DESCRIPTOR`]).
        let mut full: Vec<Option<String>> = Vec::with_capacity(defs.len());
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut budget = DESCRIPTOR_BUDGET;
        let mut out = Vec::new();
        for def in &defs {
            let parent = def.parent.map(|p| (full[p].as_deref(), defs[p].kind));
            // Inside a definition without a descriptor, nothing gets one.
            let prefix = match parent {
                Some((None, _)) => None,
                Some((Some(d), _)) => Some(d.to_string()),
                None => Some(String::new()),
            };
            let prefix = prefix.map(|mut prefix| {
                if let Some(receiver) = &def.receiver {
                    prefix.push_str(&escape_name(receiver));
                    prefix.push('#');
                }
                prefix
            });
            let Some(kind) = def.kind else {
                // An `impl` block: a chunk that names the methods inside it
                // but has no symbol of its own.
                let descriptor = prefix.map(|p| p + &def.descriptor_part(0));
                full.push(descriptor.and_then(|d| fit(d, &mut budget)));
                out.push(Chunk {
                    kind: ChunkKind::Class,
                    start_line: def.start_line,
                    end_line: def.end_line,
                    text: lines.span(def.start_line, def.end_line),
                    symbol: None,
                    title: None,
                    anchor: None,
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
            // The descriptor, and whether it names a symbol.
            let named = prefix.and_then(|prefix| {
                let first = prefix.clone() + &def.descriptor_part(0);
                if first.len() > MAX_DESCRIPTOR.min(budget) {
                    return None;
                }
                let n = *seen
                    .entry(first.clone())
                    .and_modify(|n| *n += 1)
                    .or_insert(0);
                Some(match (kind, n) {
                    (_, 0) => (first, true),
                    (SymbolKind::Function | SymbolKind::Method, n) => {
                        (prefix + &def.descriptor_part(n), true)
                    }
                    // A second type with the same name (behind a `cfg`, say)
                    // keeps its chunk but gets no symbol, so IDs stay unique.
                    _ => (first, false),
                })
            });
            let named = named.and_then(|(d, symbol)| Some((fit(d, &mut budget)?, symbol)));
            full.push(named.as_ref().map(|(d, _)| d.clone()));
            out.push(Chunk {
                kind: match kind {
                    SymbolKind::Function | SymbolKind::Method => ChunkKind::Function,
                    SymbolKind::Variable => ChunkKind::Variable,
                    SymbolKind::TypeAlias => ChunkKind::TypeAlias,
                    _ => ChunkKind::Class,
                },
                start_line: def.start_line,
                end_line: def.end_line,
                text: lines.span(def.start_line, def.end_line),
                symbol: named.and_then(|(descriptor, symbol)| {
                    symbol.then(|| Symbol {
                        name: def.name.clone(),
                        kind,
                        descriptor,
                    })
                }),
                title: None,
                anchor: None,
            });
        }
        out
    }
}

/// `descriptor` when it is at most [`MAX_DESCRIPTOR`] bytes and fits in
/// what is left of the file's `budget`, which it then takes from.
fn fit(descriptor: String, budget: &mut usize) -> Option<String> {
    if descriptor.len() > MAX_DESCRIPTOR.min(*budget) {
        return None;
    }
    *budget -= descriptor.len();
    Some(descriptor)
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
    /// The index of the innermost enclosing definition or scope.
    parent: Option<usize>,
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
            Some(SymbolKind::Variable) => format!("{name}."),
            _ => format!("{name}#"),
        }
    }
}

fn definitions(
    language: Language,
    query: &Query,
    root: Node<'_>,
    source: &[u8],
    last: u32,
) -> Vec<Definition> {
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);
    let mut found: Vec<(Node<'_>, Definition)> = Vec::new();
    // A Python decorator belongs to the definition it decorates: each
    // decorated definition's node id, mapped to the decorated span.
    let mut decorated: HashMap<usize, Node<'_>> = HashMap::new();
    while let Some(m) = matches.next() {
        let mut node = None;
        let mut is_scope = false;
        // The kind a @variable or @alias capture sets, whatever its node.
        let mut captured_kind = None;
        let mut name = None;
        let mut receiver = None;
        let mut defined_elsewhere = false;
        for capture in m.captures() {
            match names[capture.index as usize] {
                "function" | "type" => node = Some(capture.node),
                "variable" => {
                    node = Some(capture.node);
                    captured_kind = Some(SymbolKind::Variable);
                }
                "alias" => {
                    node = Some(capture.node);
                    captured_kind = Some(SymbolKind::TypeAlias);
                }
                "value" => {
                    let value = capture.node;
                    defined_elsewhere = DEFINED_ELSEWHERE.contains(&value.kind())
                        || (value.kind() == "class" && value.child_by_field_name("name").is_some());
                }
                "scope" => {
                    node = Some(capture.node);
                    is_scope = true;
                }
                "name" => name = Some(capture.node),
                "receiver" => receiver = Some(capture.node),
                "decorated" => {
                    if let Some(d) = capture.node.child_by_field_name("definition") {
                        decorated.insert(d.id(), capture.node);
                    }
                }
                _ => {}
            }
        }
        let (Some(node), Some(name)) = (node, name) else {
            continue;
        };
        if defined_elsewhere {
            continue;
        }
        let text = |n: Node<'_>| n.utf8_text(source).unwrap_or_default().to_string();
        let name = if is_scope { type_name(name) } else { name };
        found.push((
            node,
            Definition {
                kind: if is_scope {
                    None
                } else {
                    Some(captured_kind.unwrap_or_else(|| symbol_kind(node)))
                },
                name: text(name),
                receiver: receiver.map(text),
                start_line: 0,
                end_line: 0,
                parent: None,
            },
        ));
    }
    for (node, def) in &mut found {
        let span = decorated.get(&node.id()).unwrap_or(node);
        (def.start_line, def.end_line) = line_range(*span, last);
        def.start_line = def
            .start_line
            .min(documented_start(language, *span, source) as u32 + 1);
    }
    // Sort by position so every parent comes before what it encloses.
    found.sort_by_key(|(n, _)| (n.start_byte(), std::cmp::Reverse(n.end_byte())));
    // Each found definition's depth and, if kept, its index in `defs`.
    let mut kept: Vec<(usize, Option<usize>)> = Vec::with_capacity(found.len());
    // The enclosing definitions of the current one, as (end byte, index in
    // `found`), innermost last. A stack over the sorted spans, because
    // `Node::parent` walks down from the root and costs the node's depth.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut defs = Vec::with_capacity(found.len());
    for (index, (node, mut def)) in found.into_iter().enumerate() {
        while open.last().is_some_and(|&(end, _)| end < node.end_byte()) {
            open.pop();
        }
        let parent = open.last().map(|&(_, i)| i);
        open.push((node.end_byte(), index));
        let depth = parent.map_or(1, |i| kept[i].0 + 1);
        if depth > MAX_DEPTH {
            kept.push((depth, None));
            continue;
        }
        // A kept definition's parent is shallower, so it was kept too.
        def.parent = parent.and_then(|i| kept[i].1);
        kept.push((depth, Some(defs.len())));
        defs.push(def);
    }
    defs
}

/// The nodes that wrap a definition without naming it: `export`, and the
/// `type`, `const`, `var` or `let` keyword that holds a single spec. A
/// comment above the wrapper documents the definition inside it.
const WRAPPERS: [&str; 6] = [
    "export_statement",
    "lexical_declaration",
    "variable_declaration",
    "type_declaration",
    "const_declaration",
    "var_declaration",
];

/// The row a definition's chunk starts on once it takes in the doc comments
/// and attributes directly above it, without a blank line between. A comment
/// that ends a line of code belongs to that code, so it stops the run.
fn documented_start(language: Language, node: Node<'_>, source: &[u8]) -> usize {
    let mut node = node;
    while let Some(parent) = node.parent() {
        let alone = WRAPPERS.contains(&parent.kind())
            && (0..parent.named_child_count()).all(|i| {
                parent
                    .named_child(i as u32)
                    .is_none_or(|c| c.id() == node.id() || attaches(language, c, source))
            });
        if !alone {
            break;
        }
        node = parent;
    }
    let mut start = node.start_position().row;
    let mut above = node.prev_sibling();
    while let Some(sibling) = above {
        let row = sibling.start_position().row;
        if !attaches(language, sibling, source) || last_row(sibling) + 1 < start {
            break;
        }
        above = sibling.prev_sibling();
        if above.is_some_and(|a| last_row(a) == row && !attaches(language, a, source)) {
            break;
        }
        start = start.min(row);
    }
    start
}

/// Whether `node` documents or annotates the definition below it: a doc
/// comment or attribute in Rust (`///`, `/** */`, `#[...]`; not `//!`, which
/// documents the module), any comment in Go, a JSDoc block or decorator in
/// TypeScript and JavaScript, an XML doc comment in C# (`///`, `/** */`).
/// Python's decorators are part of the definition already, and its docstring
/// sits inside it.
fn attaches(language: Language, node: Node<'_>, source: &[u8]) -> bool {
    let text = node.utf8_text(source).unwrap_or_default();
    let doc_line = text.starts_with("///") && !text.starts_with("////");
    let doc_block = text.starts_with("/**") && !text.starts_with("/***") && text != "/**/";
    match (language, node.kind()) {
        (Language::Rust, "attribute_item") => true,
        (Language::Rust, "line_comment") => doc_line,
        (Language::Rust, "block_comment") => doc_block,
        (Language::Go, "comment") => true,
        (Language::TypeScript | Language::Tsx | Language::JavaScript, "decorator") => true,
        (Language::TypeScript | Language::Tsx | Language::JavaScript, "comment") => doc_block,
        (Language::CSharp, "comment") => doc_line || doc_block,
        _ => false,
    }
}

/// The kinds of value node that the language's query already captures as
/// their own definition, so a @variable or @alias holding one is skipped: a
/// function assigned to a name, or a Go struct or interface. A named class
/// expression is skipped too; an anonymous one is the variable's.
const DEFINED_ELSEWHERE: [&str; 4] = [
    "arrow_function",
    "function_expression",
    "struct_type",
    "interface_type",
];

/// The kind of a captured definition node.
fn symbol_kind(node: Node<'_>) -> SymbolKind {
    match node.kind() {
        "method_definition"
        | "method_declaration"
        | "abstract_method_signature"
        | "method_signature"
        | "method_elem"
        | "constructor_declaration" => SymbolKind::Method,
        "struct_item" | "union_item" | "struct_declaration" => SymbolKind::Struct,
        "enum_item" | "enum_declaration" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "interface_declaration" => SymbolKind::Interface,
        "class_definition"
        | "class_declaration"
        | "class"
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

/// The bare type name of a Rust `impl` target: `Foo` for `Foo<T>`, `a::Foo`,
/// `&'a Foo`, `*const Foo` or `dyn Foo`.
fn type_name(mut node: Node<'_>) -> Node<'_> {
    loop {
        match node.kind() {
            "reference_type" | "pointer_type" => match node.child_by_field_name("type") {
                Some(t) => node = t,
                None => return node,
            },
            "dynamic_type" | "abstract_type" => match node.child_by_field_name("trait") {
                Some(t) => node = t,
                None => return node,
            },
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
    let start = (node.start_position().row as u32 + 1).min(last);
    (start, (last_row(node) as u32 + 1).clamp(start, last))
}

/// The row a node's last character is on, from 0: a node that ends at the
/// start of a row, like a Rust line comment with its newline, ends on the
/// row before.
fn last_row(node: Node<'_>) -> usize {
    let start = node.start_position().row;
    let end = node.end_position();
    if end.column == 0 && end.row > start {
        end.row - 1
    } else {
        end.row
    }
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
