//! `codecite assist`: what codecite knows about an agent's own grep or glob
//! call, for a hook to attach to that call's result.

use anyhow::{Context as _, bail};
use clap::ValueEnum;
use serde_json::Value;

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

#[cfg(test)]
mod tests {
    use super::*;

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
