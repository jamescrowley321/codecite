//! Finding where a symbol is defined: the tree-sitter symbols an index run
//! stores, looked up by name, each cited.

use rusqlite::params;
use schemars::JsonSchema;
use serde::Serialize;

use super::Store;
use super::search::{MAX_SEARCH_LIMIT, code_citation, name_key, snippet};
use crate::{Error, Result, Role};

/// The providers a definition comes from. Only tree-sitter writes symbols
/// so far.
const PROVIDERS: [&str; 1] = ["treesitter"];

/// How sure a tree-sitter definition is: extracted (2) on the confidence
/// ladder, below an exact SCIP one (3).
const TREESITTER_CONFIDENCE: u8 = 2;

/// The most names a [`Error::NoSuchSymbol`] suggests.
const MAX_CANDIDATES: usize = 5;

/// Every definition of a symbol, best first: what `codecite def --json`
/// prints and the MCP `definition` tool returns, as the same JSON.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Definitions {
    /// The symbol as asked for.
    pub symbol: String,
    /// Whether more than one definition matched, so the caller must pick
    /// one rather than take the first.
    pub ambiguous: bool,
    /// How many definitions matched, which may be more than `results`
    /// holds.
    pub total: usize,
    /// The definitions, at most 50 (`MAX_SEARCH_LIMIT`): source first, then
    /// docs, examples and tests, each in citation order.
    pub results: Vec<Definition>,
}

/// One definition site, in the shape the CLI prints as JSON and the MCP
/// server returns.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Definition {
    /// The definition's lines: `repo@sha:path#L10-L42`. `read` it for the
    /// whole text.
    pub citation: String,
    /// The name as written in the code.
    pub symbol: String,
    /// What it defines: `function`, `method`, `class`, `struct`, `enum`,
    /// `trait`, `interface`, `variable` or `type_alias`.
    pub kind: String,
    /// What the definition's file is for: `source`, `test`, `example` or
    /// `docs`.
    pub role: &'static str,
    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
    pub license_class: String,
    /// When the definition's source was last indexed.
    pub indexed_at: String,
    /// Whether the file differed from the cited commit when it was indexed.
    pub dirty: bool,
    /// The definition's first 40 lines (`MAX_SNIPPET_LINES`), cut to 4 KiB
    /// (`MAX_SNIPPET_BYTES`).
    pub snippet: String,
    /// Whether the snippet stops before the end of the definition. The
    /// citation still names all of it.
    pub snippet_truncated: bool,
    /// The providers that found the definition: `treesitter`.
    pub providers: Vec<&'static str>,
    /// How sure they are: 3 exact, 2 extracted, 1 inferred, 0 ambiguous.
    pub confidence: u8,
}

/// A matching definition, with what ranking and the qualified lookup need.
struct Found {
    role: Role,
    /// The SCIP symbol string.
    id: String,
    definition: Definition,
}

impl Store {
    /// Every definition of `symbol` in the corpus.
    ///
    /// An exact, case-sensitive match of the name as written comes first.
    /// Only when nothing has that name, a dotted name such as
    /// `Client.callback` matches the definitions whose descriptors inside
    /// their file end with those names in that order
    /// (`Client#callback().`). Several matches are all returned, never one
    /// guess: by role (source, then docs, examples and tests), then by
    /// citation, at most [`MAX_SEARCH_LIMIT`] of them.
    ///
    /// A symbol that is empty or only whitespace is [`Error::EmptyQuery`],
    /// and one with no match is [`Error::NoSuchSymbol`], with the names
    /// that differ from it only in case, `_` or `-`. An index older than
    /// its maximum age is [`Error::StaleIndex`] unless `allow_stale` is set.
    pub fn definition(&self, symbol: &str, allow_stale: bool) -> Result<Definitions> {
        let name = symbol.trim();
        if name.is_empty() {
            return Err(Error::EmptyQuery);
        }
        self.check_fresh(allow_stale)?;
        let mut found = self.named(name)?;
        let parts = qualified(name);
        // The name the qualified lookup and the candidates go by.
        let last = parts
            .as_ref()
            .and_then(|p| p.last().copied())
            .unwrap_or(name);
        if found.is_empty()
            && let Some(parts) = &parts
        {
            found = self.named(last)?;
            found.retain(|f| {
                names_in_file(&f.id).is_some_and(|names| {
                    names.len() >= parts.len()
                        && names[names.len() - parts.len()..].iter().eq(parts.iter())
                })
            });
        }
        if found.is_empty() {
            return Err(Error::NoSuchSymbol {
                symbol: name.into(),
                candidates: self.similar_names(last)?,
            });
        }
        found.sort_by(|a, b| {
            (rank(a.role), &a.definition.citation).cmp(&(rank(b.role), &b.definition.citation))
        });
        let total = found.len();
        let limit = usize::try_from(MAX_SEARCH_LIMIT).unwrap_or(usize::MAX);
        let results: Vec<Definition> = found
            .into_iter()
            .take(limit)
            .map(|f| f.definition)
            .collect();
        Ok(Definitions {
            symbol: symbol.into(),
            ambiguous: results.len() > 1,
            total,
            results,
        })
    }

    /// The definitions whose name as written is `name`, found through the
    /// `symbol_name` index.
    fn named(&self, name: &str) -> Result<Vec<Found>> {
        let mut stmt = self.conn.prepare_cached(DEFINITIONS)?;
        let rows = stmt.query_map([name], |row| {
            let path: String = row.get(4)?;
            let source_kind: String = row.get(7)?;
            let source: String = row.get(6)?;
            let revision: String = row.get(8)?;
            let text: String = row.get(11)?;
            let role = Role::of(&path, &source_kind);
            let (snippet, snippet_truncated) = snippet(&text);
            let definition = Definition {
                citation: code_citation(&source, &revision, &path, row.get(2)?, row.get(3)?),
                symbol: row.get(12)?,
                kind: row.get(1)?,
                role: role.as_str(),
                license_class: row.get(9)?,
                indexed_at: row.get(10)?,
                dirty: row.get(5)?,
                snippet,
                snippet_truncated,
                providers: PROVIDERS.to_vec(),
                confidence: TREESITTER_CONFIDENCE,
            };
            Ok(Found {
                role,
                id: row.get(0)?,
                definition,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The defined names that differ from `name` only in case, `_` or `-`,
    /// sorted. Only an error asks, so a scan of the symbols is fine here.
    fn similar_names(&self, name: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT display_name FROM symbol
             WHERE codecite_name_key(display_name) = ?1 ORDER BY display_name LIMIT ?2",
        )?;
        let limit = i64::try_from(MAX_CANDIDATES).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![name_key(name), limit], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// The definitions with one name as written, `?1`. A symbol's lines are its
/// chunk's, which the index run wrote with it.
const DEFINITIONS: &str = "SELECT sym.id, coalesce(sym.kind, c.kind), c.start_line, c.end_line,
       d.path, d.dirty, s.name, s.kind, s.revision, s.license_class, s.indexed_at, c.text,
       sym.display_name
     FROM symbol sym
     JOIN chunk c ON c.symbol_id = sym.id
     JOIN document d ON d.id = sym.document_id
     JOIN source s ON s.id = d.source_id
     WHERE sym.display_name = ?1 AND s.is_current";

/// Where a role's definitions come in the results: the code an agent is
/// asked to use first, then what describes it, then what exercises it.
fn rank(role: Role) -> u8 {
    match role {
        Role::Source => 0,
        Role::Docs => 1,
        Role::Example => 2,
        Role::Test => 3,
        Role::Spec => 4,
    }
}

/// The names of a dotted symbol such as `Client.callback`, outermost
/// first, or `None` when it has no dot or an empty name between dots.
fn qualified(symbol: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = symbol.split('.').collect();
    (parts.len() > 1 && parts.iter().all(|p| !p.is_empty())).then_some(parts)
}

/// The names a symbol ID's descriptors give inside its file, outermost
/// first: `["Client", "callback"]` for
/// ``codecite . app . src/`client.rs`/Client#callback().``. The path's
/// segments are left out. `None` when the ID is not in the form an index
/// run writes.
fn names_in_file(id: &str) -> Option<Vec<String>> {
    let rest = id.strip_prefix("codecite . ")?;
    // The source's name doubles its spaces, so a single space ends it.
    let mut chars = rest.char_indices().peekable();
    let end = loop {
        let (i, c) = chars.next()?;
        if c == ' ' {
            if chars.peek().is_some_and(|&(_, c)| c == ' ') {
                chars.next();
            } else {
                break i;
            }
        }
    };
    let mut descriptors = rest[end..].strip_prefix(" . ")?;
    let mut names = Vec::new();
    while !descriptors.is_empty() {
        let (name, after) = descriptor_name(descriptors)?;
        // `/` ends a path segment, `#` a type, `.` a term and `().` or
        // `(+1).` a function.
        let (suffix, in_file) = match after.chars().next()? {
            '/' => (1, false),
            '#' | '.' => (1, true),
            '(' => (after.find(").")? + 2, true),
            _ => return None,
        };
        if in_file {
            names.push(name);
        }
        descriptors = &after[suffix..];
    }
    Some(names)
}

/// The name at the start of `text`, bare or in backticks with any backtick
/// doubled, and the text after it.
fn descriptor_name(text: &str) -> Option<(String, &str)> {
    if let Some(mut rest) = text.strip_prefix('`') {
        let mut name = String::new();
        loop {
            let tick = rest.find('`')?;
            name.push_str(&rest[..tick]);
            rest = &rest[tick + 1..];
            match rest.strip_prefix('`') {
                Some(more) => {
                    name.push('`');
                    rest = more;
                }
                None => return Some((name, rest)),
            }
        }
    }
    let bare = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '$');
    let end = text.find(|c: char| !bare(c)).unwrap_or(text.len());
    (end > 0).then(|| (text[..end].to_owned(), &text[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, DocumentRecord, ReadOptions, SourceRecord, SymbolRecord};

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    /// A client with a method, and a test and an example with a function of
    /// the same name.
    const CLIENT: &str = "pub struct Client;\n\nimpl Client {\n    pub fn callback(&self) {\n        run();\n    }\n}\n";
    const SERVER: &str = "impl Server {\n    pub fn callback(&self) {}\n}\n";
    const FREE: &str = "fn callback() {\n    assert!(true);\n}\n";

    /// One definition in a file: `(descriptor, name, kind, lines)`.
    type Def<'a> = (&'a str, &'a str, &'a str, (u32, u32));

    fn file<'a>(path: &'a str, text: &'a str, defs: &[Def<'a>]) -> DocumentRecord<'a> {
        let chunks = defs
            .iter()
            .map(|&(descriptor, name, kind, (start, end))| ChunkRecord {
                kind: if kind == "struct" {
                    "class"
                } else {
                    "function"
                },
                start_line: start,
                end_line: end,
                text: span(text, start, end),
                number: None,
                anchor: None,
                title: None,
                parent: None,
                strength: &[],
                symbol: Some(SymbolRecord {
                    id: format!("codecite . app . {}/{descriptor}", path_descriptors(path)),
                    display_name: name,
                    kind,
                }),
            })
            .collect();
        DocumentRecord {
            path,
            language: Some("rust"),
            content_hash: path.as_bytes(),
            size_bytes: 1,
            dirty: path.starts_with("tests/"),
            text: Some(text),
            chunks,
        }
    }

    /// Lines `start` to `end` of `text`, from 1, without the last newline.
    fn span(text: &str, start: u32, end: u32) -> &str {
        let offset = |line: u32| -> usize {
            text.split_inclusive('\n')
                .take(line as usize)
                .map(str::len)
                .sum()
        };
        text[offset(start - 1)..offset(end)].trim_end_matches('\n')
    }

    /// A path's segments as descriptors: `src/`client.rs``.
    fn path_descriptors(path: &str) -> String {
        let segments: Vec<String> = path
            .split('/')
            .map(|s| {
                if s.contains('.') {
                    format!("`{s}`")
                } else {
                    s.to_owned()
                }
            })
            .collect();
        segments.join("/")
    }

    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store
            .upsert_source(&SourceRecord {
                name: "app",
                kind: "local",
                url: "/src/app",
                revision: HEAD,
                license_spdx: Some("MIT"),
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        let docs = [
            file(
                "tests/client.rs",
                FREE,
                &[("callback().", "callback", "function", (1, 3))],
            ),
            file(
                "src/client.rs",
                CLIENT,
                &[
                    ("Client#", "Client", "struct", (1, 1)),
                    ("Client#callback().", "callback", "method", (4, 6)),
                ],
            ),
            file(
                "examples/demo.rs",
                FREE,
                &[("callback().", "callback", "function", (1, 3))],
            ),
            file(
                "src/server.rs",
                SERVER,
                &[("Server#callback().", "callback", "method", (2, 2))],
            ),
        ];
        for doc in &docs {
            store.write_document(app, doc).unwrap();
        }
        store.set_indexed_at(30).unwrap();
        (dir, store)
    }

    fn citations(found: &Definitions) -> Vec<&str> {
        found.results.iter().map(|r| r.citation.as_str()).collect()
    }

    #[test]
    fn a_unique_symbol_has_one_cited_definition() {
        let (_dir, store) = fixture();
        let found = store.definition("Client", false).unwrap();
        assert_eq!(found.symbol, "Client");
        assert!(!found.ambiguous);
        assert_eq!(found.total, 1);
        let def = &found.results[0];
        assert_eq!(def.citation, format!("app@{HEAD}:src/client.rs#L1"));
        assert_eq!(def.symbol, "Client");
        assert_eq!(def.kind, "struct");
        assert_eq!(def.role, "source");
        assert_eq!(def.license_class, "adapt");
        assert!(!def.dirty);
        assert_eq!(def.snippet, "pub struct Client;");
        assert_eq!(def.providers, ["treesitter"]);
        assert_eq!(def.confidence, 2);
        let json = serde_json::to_value(&found).unwrap();
        let keys: Vec<&String> = json["results"][0].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "citation",
                "confidence",
                "dirty",
                "indexed_at",
                "kind",
                "license_class",
                "providers",
                "role",
                "snippet",
                "snippet_truncated",
                "symbol"
            ]
        );
    }

    #[test]
    fn an_ambiguous_symbol_returns_every_definition_in_a_fixed_order() {
        let (_dir, store) = fixture();
        let found = store.definition("callback", false).unwrap();
        assert!(found.ambiguous);
        assert_eq!(found.total, 4);
        assert_eq!(
            citations(&found),
            [
                format!("app@{HEAD}:src/client.rs#L4-L6"),
                format!("app@{HEAD}:src/server.rs#L2"),
                format!("app@{HEAD}:examples/demo.rs#L1-L3"),
                format!("app@{HEAD}:tests/client.rs#L1-L3"),
            ]
        );
        let roles: Vec<&str> = found.results.iter().map(|r| r.role).collect();
        assert_eq!(roles, ["source", "source", "example", "test"]);
        assert!(found.results[3].dirty);
        // The same corpus always gives the same order.
        assert_eq!(store.definition("callback", false).unwrap(), found);
    }

    #[test]
    fn each_citation_reads_back_the_definition() {
        let (_dir, store) = fixture();
        for def in store.definition("callback", false).unwrap().results {
            let read = store.read(&def.citation, &ReadOptions::default()).unwrap();
            assert_eq!(read.text, def.snippet, "{}", def.citation);
        }
    }

    #[test]
    fn a_dotted_name_matches_the_descriptors_it_ends_with() {
        let (_dir, store) = fixture();
        let found = store.definition("Client.callback", false).unwrap();
        assert_eq!(found.symbol, "Client.callback");
        assert!(!found.ambiguous);
        assert_eq!(
            citations(&found),
            [format!("app@{HEAD}:src/client.rs#L4-L6")]
        );
        assert_eq!(found.results[0].symbol, "callback");
        let err = store.definition("Other.callback", false).unwrap_err();
        assert!(
            matches!(&err, Error::NoSuchSymbol { candidates, .. } if candidates == &["callback"]),
            "{err:?}"
        );
        // A path segment is not a name in the file.
        assert!(store.definition("client.callback", false).is_err());
    }

    #[test]
    fn a_missing_symbol_is_an_error_naming_close_names() {
        let (_dir, store) = fixture();
        let err = store.definition("call_back", false).unwrap_err();
        assert_eq!(err.kind(), "no_such_symbol");
        assert_eq!(err.candidates(), ["callback"]);
        // The exact match is case-sensitive.
        let err = store.definition("client", false).unwrap_err();
        assert_eq!(err.candidates(), ["Client"]);
        let err = store.definition("nothing", false).unwrap_err();
        assert!(err.candidates().is_empty(), "{err:?}");
        for empty in ["", "   "] {
            let err = store.definition(empty, false).unwrap_err();
            assert!(matches!(err, Error::EmptyQuery), "{empty:?}: {err:?}");
        }
    }

    #[test]
    fn a_stale_index_is_refused_unless_allowed() {
        let (dir, store) = fixture();
        rusqlite::Connection::open(dir.path().join("demo.db"))
            .unwrap()
            .execute(
                "UPDATE meta SET value = '2026-01-01T00:00:00Z' WHERE key = 'indexed_at'",
                [],
            )
            .unwrap();
        let err = store.definition("Client", false).unwrap_err();
        assert!(matches!(err, Error::StaleIndex { .. }), "{err:?}");
        assert_eq!(store.definition("Client", true).unwrap().total, 1);
    }

    #[test]
    fn at_most_50_are_returned_with_the_full_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        let app = store
            .upsert_source(&SourceRecord {
                name: "app",
                kind: "local",
                url: "/src/app",
                revision: HEAD,
                license_spdx: None,
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        let paths: Vec<String> = (0..60).map(|i| format!("src/m{i:02}.rs")).collect();
        for path in &paths {
            let doc = file(
                path,
                FREE,
                &[("callback().", "callback", "function", (1, 3))],
            );
            store.write_document(app, &doc).unwrap();
        }
        store.set_indexed_at(30).unwrap();
        let found = store.definition("callback", false).unwrap();
        assert_eq!(found.total, 60);
        assert_eq!(found.results.len(), 50);
        assert!(found.results[0].citation.contains(":src/m00.rs#"));
        assert!(found.results[49].citation.contains(":src/m49.rs#"));
    }

    #[test]
    fn both_lookups_go_through_the_name_index() {
        let (_dir, store) = fixture();
        let mut stmt = store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {DEFINITIONS}"))
            .unwrap();
        let plan: Vec<String> = stmt
            .query_map(["callback"], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        // The symbol table is searched first, by name, and nothing is
        // scanned.
        assert!(
            plan[0].contains("sym USING INDEX symbol_name (display_name=?)"),
            "{plan:?}"
        );
        assert!(
            plan.iter().all(|step| !step.starts_with("SCAN")),
            "{plan:?}"
        );
    }

    #[test]
    fn descriptors_give_the_names_inside_the_file() {
        let names = |id: &str| names_in_file(id);
        assert_eq!(
            names("codecite . app . src/`client.rs`/Client#callback()."),
            Some(vec!["Client".into(), "callback".into()])
        );
        assert_eq!(
            names("codecite . my  repo . `a``b`/c/Client#`new thing`(+1)."),
            Some(vec!["Client".into(), "new thing".into()])
        );
        assert_eq!(
            names("codecite . app . `main.py`/outer().inner().LIMIT."),
            Some(vec!["outer".into(), "inner".into(), "LIMIT".into()])
        );
        for bad in [
            "scip . app . a/b#",
            "codecite . app",
            "codecite . app . a/b!",
        ] {
            assert_eq!(names(bad), None, "{bad}");
        }
        assert_eq!(
            qualified("Client.callback"),
            Some(vec!["Client", "callback"])
        );
        for not in ["callback", ".callback", "Client.", "a..b"] {
            assert_eq!(qualified(not), None, "{not}");
        }
    }
}
