//! `codecite assist`: what codecite knows about an agent's own grep or glob
//! call, for a hook to attach to that call's result.

use std::path::Path;

use anyhow::{Context as _, bail};
use clap::ValueEnum;
use codecite_core::{ContextQuery, ContextTarget, Error, SearchQuery, Store};
use serde_json::Value;

/// How many hits a block shows at most.
const MAX_HITS: usize = 5;

/// The most characters of a hit's first line a block shows.
const MAX_LINE_CHARS: usize = 120;

/// The agent tool whose call is being assisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Tool {
    /// A content search: Claude Code's `Grep`, pi's `grep`.
    Grep,
    /// A file-name search: Claude Code's `Glob`, pi's `find`.
    Glob,
}

/// The parts of the tool's input that assist reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInput {
    /// The regex (grep) or glob (glob) the agent searched for.
    pub pattern: String,
    /// The file or directory the agent searched in, if it named one.
    pub path: Option<String>,
}

impl ToolInput {
    /// Reads the tool's input JSON. `pattern` must be a string; `path` is
    /// kept when it is one.
    pub fn parse(json: &str) -> anyhow::Result<ToolInput> {
        let value: Value = serde_json::from_str(json).context("reading the tool input as JSON")?;
        let Some(pattern) = value.get("pattern").and_then(Value::as_str) else {
            bail!("the tool input has no string `pattern`");
        };
        let path = value.get("path").and_then(Value::as_str).map(str::to_owned);
        Ok(ToolInput {
            pattern: pattern.to_owned(),
            path,
        })
    }
}

/// What to look up for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// The pattern is one identifier: look up its definition.
    Symbol(String),
    /// Search for these words.
    Search(String),
    /// The pattern has nothing to look up.
    Nothing,
}

/// Decides what to look up for a call.
pub fn lookup(tool: Tool, input: &ToolInput) -> Lookup {
    match tool {
        Tool::Grep => match symbol(&input.pattern) {
            Some(name) => Lookup::Symbol(name.to_owned()),
            None => words(&input.pattern),
        },
        Tool::Glob => {
            // Keep the last dot-free part of each path segment's literal
            // text: `**/*logout*.py` → `logout`; the extension is dropped.
            let stems: Vec<&str> = input
                .pattern
                .split('/')
                .map(|segment| match segment.rsplit_once('.') {
                    Some((stem, _ext)) => stem,
                    None => segment,
                })
                .collect();
            words(&stems.join(" "))
        }
    }
}

/// The identifier a grep pattern names, if it is exactly one: `name`,
/// `\bname\b`, or a qualified path (`Type::name`, `a.b`) whose last segment
/// is taken.
fn symbol(pattern: &str) -> Option<&str> {
    let pattern = pattern.strip_prefix(r"\b").unwrap_or(pattern);
    let pattern = pattern.strip_suffix(r"\b").unwrap_or(pattern);
    let last = pattern.rsplit(['.', ':']).next()?;
    let segments_ok = pattern
        .split("::")
        .flat_map(|part| part.split('.'))
        .all(is_identifier);
    (segments_ok && is_identifier(last)).then_some(last)
}

fn is_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The words of a pattern as a search query: escapes and every character
/// that is not a letter, digit or `_` split words, and words under three
/// characters are dropped.
fn words(pattern: &str) -> Lookup {
    let mut cleaned = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // `\s`, `\b`, `\.`: the escaped character is never a word.
            chars.next();
            cleaned.push(' ');
        } else if c.is_alphanumeric() || c == '_' {
            cleaned.push(c);
        } else {
            cleaned.push(' ');
        }
    }
    let kept: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|word| word.chars().count() >= 3)
        .collect();
    if kept.is_empty() {
        Lookup::Nothing
    } else {
        Lookup::Search(kept.join(" "))
    }
}

/// One line of the block: a cited hit, and the indexed path it is in, when
/// it is in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The hit's citation.
    pub citation: String,
    /// The path inside the source, read from the citation; `None` for a
    /// fetched document or a spec.
    pub path: Option<String>,
    /// The rest of the line: `<kind>[ <symbol>] · <first line>`.
    pub line: String,
}

impl Hit {
    fn new(citation: &str, label: &str, text: &str) -> Hit {
        let first = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("");
        let mut first = crate::escape_controls(first);
        if let Some((cut, _)) = first.char_indices().nth(MAX_LINE_CHARS) {
            first.truncate(cut);
            first.push('…');
        }
        Hit {
            citation: crate::escape_controls(citation),
            path: cited_path(citation).map(str::to_owned),
            line: format!("{} · {first}", crate::escape_controls(label)),
        }
    }
}

/// The path a citation names inside its source: `repo@sha:src/a.rs#L1-L9`
/// → `src/a.rs`. A document citation (`doc@sha256:…`) has none.
fn cited_path(citation: &str) -> Option<&str> {
    let (_, pinned) = citation.split_once('@')?;
    if pinned.starts_with("sha256:") {
        return None;
    }
    let (_, rest) = pinned.split_once(':')?;
    let path = rest.split_once('#').map_or(rest, |(path, _)| path);
    (!path.is_empty()).then_some(path)
}

/// Whether the tool's output already names `path`: the path appears in it
/// as a whole path or as the end of a longer one (`/abs/src/a.rs` and
/// `src/a.rs:12:…` both match `src/a.rs`, `src/ab.rs` does not).
fn already_matched(output: &str, path: &str) -> bool {
    let starts = |c: char| c == '/' || c == '\\' || c == '"' || c == '\'' || c.is_whitespace();
    let ends = |c: char| c == '"' || c == '\'' || c == ':' || c == ',' || c.is_whitespace();
    output.match_indices(path).any(|(at, _)| {
        let before = output[..at].chars().next_back();
        let after = output[at + path.len()..].chars().next();
        before.is_none_or(starts) && after.is_none_or(ends)
    })
}

/// Looks the call up in `store` and renders the block; an empty string
/// when there is nothing the tool's `output` does not already show. A
/// stale or unreadable corpus is an error, which the caller drops.
pub fn run(
    store: &Store,
    corpus: &str,
    tool: Tool,
    input: &ToolInput,
    output: &str,
    max_bytes: usize,
) -> anyhow::Result<String> {
    let mut hits = Vec::new();
    let mut more = None;
    match lookup(tool, input) {
        Lookup::Symbol(symbol) => match store.definition(&symbol, false) {
            Ok(found) => {
                for def in &found.results {
                    let label = format!("{} {}", def.kind, def.symbol);
                    hits.push(Hit::new(&def.citation, &label, &def.snippet));
                }
                more = Some(format!("codecite def {corpus} {symbol}"));
            }
            Err(Error::NoSuchSymbol { .. }) => {
                more = search(store, corpus, &symbol, &mut hits)?;
            }
            Err(e) => return Err(e.into()),
        },
        Lookup::Search(words) => more = search(store, corpus, &words, &mut hits)?,
        Lookup::Nothing => {}
    }
    hits.retain(|hit| {
        hit.path
            .as_deref()
            .is_none_or(|path| !already_matched(output, path))
    });
    hits.truncate(MAX_HITS);
    if let Some(path) = input.path.as_deref().filter(|p| Path::new(p).is_file()) {
        hits.extend(rules(store, path));
    }
    Ok(render(corpus, &hits, more.as_deref(), max_bytes))
}

/// Searches for `words`, adding the hits; returns the command that shows
/// more of them.
fn search(
    store: &Store,
    corpus: &str,
    words: &str,
    hits: &mut Vec<Hit>,
) -> anyhow::Result<Option<String>> {
    let results = store.search(&SearchQuery {
        text: words,
        language: None,
        path: None,
        only: None,
        // More than are shown, so dropping what grep found leaves some.
        limit: 4 * MAX_HITS as u32,
        allow_stale: false,
    });
    let results = match results {
        Ok(results) => results,
        Err(Error::EmptyQuery) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    for result in &results {
        let label = match &result.symbol {
            Some(symbol) => format!("{} {symbol}", result.kind),
            None => result.kind.clone(),
        };
        hits.push(Hit::new(&result.citation, &label, &result.snippet));
    }
    Ok(Some(format!("codecite search {corpus} \"{words}\"")))
}

/// The guidance rules that govern the file at `path`, one line each; none
/// when the file can't be described or the corpus has none.
fn rules(store: &Store, path: &str) -> Vec<Hit> {
    let Ok(target) = ContextTarget::inspect(Path::new(path), None, None) else {
        return Vec::new();
    };
    let query = ContextQuery {
        target: &target,
        allow_stale: false,
    };
    let Ok(found) = store.context(&query) else {
        return Vec::new();
    };
    found
        .rules
        .iter()
        .map(|rule| {
            let label = format!("rule {}", rule.strength.join("/"));
            let text = rule.title.as_deref().unwrap_or(&rule.text);
            Hit::new(&rule.citation, &label, text)
        })
        .collect()
}

/// The block: a header naming codecite and the corpus, then one line per
/// hit, `<citation> · <kind>[ <symbol>] · <first line>`, as many whole
/// lines as fit in `max_bytes`, then `… N more: <more>` when some were left
/// out. No hits → an empty string.
pub fn render(corpus: &str, hits: &[Hit], more: Option<&str>, max_bytes: usize) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = format!("codecite ({corpus}) also found, not in this result:\n");
    for (shown, hit) in hits.iter().enumerate() {
        let line = format!("{} · {}\n", hit.citation, hit.line);
        let left = hits.len() - shown;
        // Room for this line, and for the `more` line if any are left after.
        let tail = if left > 1 {
            more_line(left - 1, more).len()
        } else {
            0
        };
        if out.len() + line.len() + tail > max_bytes {
            if shown == 0 {
                return String::new();
            }
            out.push_str(&more_line(left, more));
            return out;
        }
        out.push_str(&line);
    }
    out
}

fn more_line(n: usize, more: Option<&str>) -> String {
    match more {
        Some(more) => format!("… {n} more: {more}\n"),
        None => format!("… {n} more\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str) -> Hit {
        Hit {
            citation: format!("repo@abc1234:{path}#L1-L9"),
            path: Some(path.to_owned()),
            line: "function f · fn f() {".to_owned(),
        }
    }

    #[test]
    fn a_citation_names_its_path() {
        assert_eq!(cited_path("repo@abc1234:src/a.rs#L1-L9"), Some("src/a.rs"));
        assert_eq!(cited_path("repo@abc1234:README.md"), Some("README.md"));
        assert_eq!(cited_path("rfc9110@sha256:0123456789ab#section-4"), None);
        assert_eq!(cited_path("no citation"), None);
    }

    #[test]
    fn what_grep_found_is_matched_whole() {
        assert!(already_matched("/home/u/repo/src/a.rs\n", "src/a.rs"));
        assert!(already_matched("src/a.rs:12:fn f() {}", "src/a.rs"));
        assert!(already_matched(r#"{"filenames":["src/a.rs"]}"#, "src/a.rs"));
        assert!(!already_matched("src/ab.rs\n", "src/a.rs"));
        assert!(!already_matched("lib/xsrc/a.rs\n", "src/a.rs"));
        assert!(!already_matched("", "src/a.rs"));
    }

    #[test]
    fn no_hits_render_nothing() {
        assert_eq!(render("c", &[], Some("codecite search c \"x\""), 2048), "");
    }

    #[test]
    fn the_block_names_codecite_and_the_corpus() {
        let block = render("identity", &[hit("src/a.rs")], None, 2048);
        assert_eq!(
            block,
            "codecite (identity) also found, not in this result:\n\
             repo@abc1234:src/a.rs#L1-L9 · function f · fn f() {\n"
        );
    }

    #[test]
    fn the_budget_keeps_whole_lines_and_says_how_many_more() {
        let hits: Vec<Hit> = (0..5).map(|i| hit(&format!("src/{i}.rs"))).collect();
        let more = "codecite search c \"x\"";
        let full = render("c", &hits, Some(more), usize::MAX);
        assert_eq!(full.lines().count(), 6);
        let cut = render("c", &hits, Some(more), 200);
        assert!(cut.len() <= 200, "{} bytes: {cut}", cut.len());
        let last = cut.lines().last().unwrap();
        assert!(last.starts_with("… ") && last.ends_with(more), "{last}");
        let shown = cut.lines().count() - 2;
        assert!(shown >= 1);
        assert_eq!(last, format!("… {} more: {more}", 5 - shown));
        for line in cut.lines().skip(1).take(shown) {
            assert!(full.contains(line));
        }
    }

    #[test]
    fn a_budget_too_small_for_one_line_renders_nothing() {
        assert_eq!(render("c", &[hit("src/a.rs")], None, 40), "");
    }

    #[test]
    fn a_long_first_line_is_cut() {
        let long = "x".repeat(500);
        let h = Hit::new(
            "repo@abc1234:a.rs#L1-L2",
            "window",
            &format!("\n  {long}\n"),
        );
        assert_eq!(h.line, format!("window · {}…", "x".repeat(MAX_LINE_CHARS)));
        assert_eq!(h.path.as_deref(), Some("a.rs"));
    }

    fn grep(pattern: &str) -> Lookup {
        lookup(
            Tool::Grep,
            &ToolInput {
                pattern: pattern.to_owned(),
                path: None,
            },
        )
    }

    fn glob(pattern: &str) -> Lookup {
        lookup(
            Tool::Glob,
            &ToolInput {
                pattern: pattern.to_owned(),
                path: None,
            },
        )
    }

    #[test]
    fn an_identifier_is_a_symbol() {
        let expected = Lookup::Symbol("validate_logout_token".to_owned());
        assert_eq!(grep("validate_logout_token"), expected);
        assert_eq!(grep(r"\bvalidate_logout_token\b"), expected);
    }

    #[test]
    fn a_qualified_path_takes_its_last_segment() {
        assert_eq!(
            grep("Store::open_corpus"),
            Lookup::Symbol("open_corpus".to_owned())
        );
        assert_eq!(grep("store.search"), Lookup::Symbol("search".to_owned()));
    }

    #[test]
    fn a_regex_becomes_its_words() {
        assert_eq!(
            grep(r"fn\s+validate.*token"),
            Lookup::Search("validate token".to_owned())
        );
        assert_eq!(
            grep("logout token"),
            Lookup::Search("logout token".to_owned())
        );
    }

    #[test]
    fn a_pattern_without_words_is_nothing() {
        assert_eq!(grep(r"^\s*$"), Lookup::Nothing);
        assert_eq!(grep("a.b"), Lookup::Symbol("b".to_owned()));
        assert_eq!(grep("1+2"), Lookup::Nothing);
    }

    #[test]
    fn a_glob_becomes_its_literal_words() {
        assert_eq!(glob("**/*logout*.py"), Lookup::Search("logout".to_owned()));
        assert_eq!(
            glob("src/auth/**/*.ts"),
            Lookup::Search("src auth".to_owned())
        );
        assert_eq!(glob("**/*.rs"), Lookup::Nothing);
        assert_eq!(glob("*"), Lookup::Nothing);
    }

    #[test]
    fn the_input_keeps_pattern_and_path() {
        let input =
            ToolInput::parse(r#"{"pattern": "x", "path": "src/a.py", "glob": "*.py"}"#).unwrap();
        assert_eq!(input.pattern, "x");
        assert_eq!(input.path.as_deref(), Some("src/a.py"));
        let input = ToolInput::parse(r#"{"pattern": "x", "path": 3}"#).unwrap();
        assert_eq!(input.path, None);
    }

    #[test]
    fn a_missing_or_non_string_pattern_is_an_error() {
        assert!(ToolInput::parse(r#"{"path": "src"}"#).is_err());
        assert!(ToolInput::parse(r#"{"pattern": 3}"#).is_err());
        assert!(ToolInput::parse("[]").is_err());
        assert!(ToolInput::parse("not json").is_err());
    }
}
