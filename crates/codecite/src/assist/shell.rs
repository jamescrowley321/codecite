//! Reading a search out of an agent's shell command, so `assist` can meet
//! the `grep`, `rg` and `find` an agent runs through its Bash tool as it
//! meets its Grep and Glob calls.
//!
//! This is a reader, not a shell: it splits words and quotes the way `sh`
//! does closely enough to find a search's pattern and path, and gives up
//! (no search) on anything else. Nothing here runs the command.

use super::{Tool, ToolInput};

/// The search in `command`, as the grep or glob call it amounts to: the
/// first content search (`grep`, `rg`, `git grep`, …) in it, or else the
/// first file-name search (`find … -name`, `fd`). `None` when it runs no
/// search assist can read, or only an inverted one (`grep -v`).
pub fn search(command: &str) -> Option<(Tool, ToolInput)> {
    let found: Vec<(Tool, ToolInput)> = segments(command)
        .iter()
        .filter_map(|words| one(words))
        .collect();
    let grep = found.iter().position(|(tool, _)| *tool == Tool::Grep);
    let pick = grep.unwrap_or(0);
    found.into_iter().nth(pick)
}

/// The simple commands in `command`, each as its words: split at `|`,
/// `||`, `&&`, `;`, `&`, newlines and parentheses outside quotes. Quotes
/// are removed and backslash escapes resolved; `$(…)` and backticks are
/// left as text, so a search inside one is not read.
fn segments(command: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    let end_word = |word: &mut String, in_word: &mut bool, words: &mut Vec<String>| {
        if *in_word {
            words.push(std::mem::take(word));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => match chars.peek() {
                            Some(&n @ ('"' | '\\' | '$' | '`')) => {
                                word.push(n);
                                chars.next();
                            }
                            _ => word.push('\\'),
                        },
                        _ => word.push(q),
                    }
                }
            }
            '\\' => match chars.next() {
                // A line continuation, or a trailing backslash: no word.
                Some('\n') | None => {}
                Some(n) => {
                    in_word = true;
                    word.push(n);
                }
            },
            '|' | '&' | ';' | '\n' | '(' | ')' => {
                end_word(&mut word, &mut in_word, &mut words);
                if !words.is_empty() {
                    segments.push(std::mem::take(&mut words));
                }
            }
            '#' if !in_word => {
                // A comment runs to the end of the line.
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
                end_word(&mut word, &mut in_word, &mut words);
                if !words.is_empty() {
                    segments.push(std::mem::take(&mut words));
                }
            }
            c if c.is_whitespace() => end_word(&mut word, &mut in_word, &mut words),
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut word, &mut in_word, &mut words);
    if !words.is_empty() {
        segments.push(words);
    }
    segments
}

/// The search one simple command runs, if any.
fn one(words: &[String]) -> Option<(Tool, ToolInput)> {
    let words = program(words);
    let (name, args) = words.split_first()?;
    let name = name.rsplit('/').next().unwrap_or(name);
    match name {
        "grep" | "egrep" | "fgrep" => grep(args, GREP_VALUES, GREP_LONG_VALUES),
        "rg" => grep(args, RG_VALUES, RG_LONG_VALUES),
        "ag" | "ack" => grep(args, AG_VALUES, AG_LONG_VALUES),
        "git" if args.first().is_some_and(|a| a == "grep") => {
            grep(&args[1..], GIT_GREP_VALUES, GREP_LONG_VALUES)
        }
        "find" => find(args),
        "fd" | "fdfind" => fd(args),
        _ => None,
    }
}

/// The words from the program on: leading `VAR=value` assignments and
/// wrappers that run the next word (`sudo`, `xargs`, `command`, `time`,
/// `nice`, `timeout <n>`, `env`) and their options are skipped.
fn program(mut words: &[String]) -> &[String] {
    loop {
        let Some((first, rest)) = words.split_first() else {
            return words;
        };
        let assignment = first
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && name.chars().all(is_name_char));
        if assignment {
            words = rest;
            continue;
        }
        let wrapper = matches!(
            first.rsplit('/').next().unwrap_or(first),
            "sudo" | "xargs" | "command" | "time" | "nice" | "env" | "timeout" | "exec"
        );
        if !wrapper {
            return words;
        }
        let takes_number = first.ends_with("timeout");
        words = rest;
        while words.first().is_some_and(|w| w.starts_with('-')) {
            words = &words[1..];
        }
        if takes_number && !words.is_empty() {
            words = &words[1..];
        }
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Short options that take a value, per program; `e` is the pattern.
const GREP_VALUES: &str = "efmABCdD";
const GIT_GREP_VALUES: &str = "efmABCO";
const RG_VALUES: &str = "efgtTmABCjMEr";
const AG_VALUES: &str = "GgABCm";

/// Long options that take a value as the next word (`--glob '*.py'`); the
/// `--opt=value` form is one word and needs no entry.
const GREP_LONG_VALUES: &[&str] = &[
    "--regexp",
    "--file",
    "--max-count",
    "--after-context",
    "--before-context",
    "--context",
    "--include",
    "--exclude",
    "--exclude-dir",
    "--devices",
    "--directories",
    "--label",
];
const RG_LONG_VALUES: &[&str] = &[
    "--regexp",
    "--file",
    "--glob",
    "--iglob",
    "--type",
    "--type-not",
    "--max-count",
    "--after-context",
    "--before-context",
    "--context",
    "--max-depth",
    "--threads",
    "--max-columns",
    "--encoding",
    "--replace",
    "--sort",
    "--sortr",
    "--type-add",
];
const AG_LONG_VALUES: &[&str] = &[
    "--file-search-regex",
    "--ignore",
    "--ignore-dir",
    "--after",
    "--before",
    "--context",
    "--max-count",
    "--depth",
];

/// A content search: the pattern is the first `-e` value or else the first
/// operand, and the path the first operand after the pattern. An inverted
/// search (`-v`, `--invert-match`) looks for nothing.
fn grep(args: &[String], values: &str, long_values: &[&str]) -> Option<(Tool, ToolInput)> {
    let mut pattern = None;
    let mut operands = Vec::new();
    let mut i = 0;
    let mut options_done = false;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if options_done || !arg.starts_with('-') || arg == "-" {
            operands.push(arg.as_str());
            continue;
        }
        if arg == "--" {
            options_done = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (long, None),
            };
            if name == "invert-match" {
                return None;
            }
            let takes = long_values.contains(&format!("--{name}").as_str());
            let value = match inline {
                Some(value) => Some(value.to_owned()),
                None if takes => {
                    i += 1;
                    args.get(i - 1).cloned()
                }
                None => None,
            };
            if name == "regexp" && pattern.is_none() {
                pattern = value;
            }
            continue;
        }
        // A cluster of short options: `-rn`, `-A3`, `-ie pattern`.
        let cluster: Vec<char> = arg[1..].chars().collect();
        for (at, &c) in cluster.iter().enumerate() {
            if c == 'v' {
                return None;
            }
            if values.contains(c) {
                let rest: String = cluster[at + 1..].iter().collect();
                let value = if rest.is_empty() {
                    i += 1;
                    args.get(i - 1).cloned()
                } else {
                    Some(rest)
                };
                if c == 'e' && pattern.is_none() {
                    pattern = value;
                }
                break;
            }
        }
    }
    let mut operands = operands.into_iter();
    let pattern = match pattern {
        Some(pattern) => pattern,
        None => operands.next()?.to_owned(),
    };
    let path = operands.next().map(str::to_owned);
    Some((Tool::Grep, ToolInput { pattern, path }))
}

/// `find [path…] [expression]`: the first `-name`, `-iname`, `-path`,
/// `-ipath`, `-wholename` or `-regex` value is the glob, and the first
/// path, when it is not `.`, the path. With none of those it lists files
/// and looks for nothing.
fn find(args: &[String]) -> Option<(Tool, ToolInput)> {
    let mut path = None;
    let mut iter = args.iter();
    let mut pattern = None;
    let mut in_paths = true;
    while let Some(arg) = iter.next() {
        if in_paths && !arg.starts_with('-') && arg != "(" && arg != "!" {
            if path.is_none() && arg != "." {
                path = Some(arg.clone());
            }
            continue;
        }
        in_paths = false;
        if matches!(
            arg.as_str(),
            "-name"
                | "-iname"
                | "-path"
                | "-ipath"
                | "-wholename"
                | "-iwholename"
                | "-regex"
                | "-iregex"
        ) {
            pattern = iter.next().cloned();
            break;
        }
    }
    let pattern = pattern?;
    Some((Tool::Glob, ToolInput { pattern, path }))
}

/// Short options of `fd` that take a value.
const FD_VALUES: &str = "edtEcjSxXo";

/// `fd [options] [pattern] [path…]`: the pattern is a file-name search.
fn fd(args: &[String]) -> Option<(Tool, ToolInput)> {
    let mut operands = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if arg == "--" {
            operands.extend(args[i..].iter().cloned());
            break;
        }
        if arg.starts_with("--") {
            // fd's long options with values all accept `--opt=value`; the
            // spaced form is read as a flag, at worst taking a value for
            // the pattern, which then only looks up its words.
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            let last = arg.chars().last().unwrap_or('-');
            if arg.len() == 2 && FD_VALUES.contains(last) {
                i += 1;
            }
            continue;
        }
        operands.push(arg.clone());
    }
    let mut operands = operands.into_iter();
    let pattern = operands.next()?;
    let path = operands.next();
    Some((Tool::Glob, ToolInput { pattern, path }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(command: &str) -> Option<(Tool, String, Option<String>)> {
        search(command).map(|(tool, input)| (tool, input.pattern, input.path))
    }

    fn grep_of(pattern: &str, path: Option<&str>) -> Option<(Tool, String, Option<String>)> {
        Some((Tool::Grep, pattern.to_owned(), path.map(str::to_owned)))
    }

    fn glob_of(pattern: &str, path: Option<&str>) -> Option<(Tool, String, Option<String>)> {
        Some((Tool::Glob, pattern.to_owned(), path.map(str::to_owned)))
    }

    #[test]
    fn a_plain_grep_gives_its_pattern_and_path() {
        assert_eq!(
            read("grep -rn validate_logout_token app/"),
            grep_of("validate_logout_token", Some("app/"))
        );
        assert_eq!(
            read("grep -rn 'logout token'"),
            grep_of("logout token", None)
        );
    }

    #[test]
    fn quotes_and_escapes_are_resolved() {
        assert_eq!(
            read(r#"rg "fn\s+validate.*token" src"#),
            grep_of(r"fn\s+validate.*token", Some("src"))
        );
        assert_eq!(
            read(r"grep logout\ token ."),
            grep_of("logout token", Some("."))
        );
        assert_eq!(
            read(r#"grep "say \"hi\"" a.py"#),
            grep_of(r#"say "hi""#, Some("a.py"))
        );
    }

    #[test]
    fn options_that_take_values_are_skipped() {
        assert_eq!(
            read("grep -A 3 -m 5 --include '*.py' sid app"),
            grep_of("sid", Some("app"))
        );
        assert_eq!(
            read("rg -g '*.rs' -t rust --max-depth 2 open_corpus crates"),
            grep_of("open_corpus", Some("crates"))
        );
        assert_eq!(read("rg -C3 open_corpus"), grep_of("open_corpus", None));
        assert_eq!(read("grep --include=*.py sid"), grep_of("sid", None));
    }

    #[test]
    fn the_e_option_is_the_pattern() {
        assert_eq!(
            read("grep -e logout_token src"),
            grep_of("logout_token", Some("src"))
        );
        assert_eq!(
            read("grep -rie logout_token src"),
            grep_of("logout_token", Some("src"))
        );
        assert_eq!(read("rg --regexp=sid src"), grep_of("sid", Some("src")));
        assert_eq!(read("grep -- -v src"), grep_of("-v", Some("src")));
    }

    #[test]
    fn an_inverted_grep_is_no_search() {
        assert_eq!(read("grep -v '/.git/'"), None);
        assert_eq!(read("grep --invert-match x"), None);
        assert_eq!(read("grep -rnv x"), None);
    }

    #[test]
    fn the_search_is_found_inside_a_compound_command() {
        assert_eq!(
            read("cd /work/project && grep -rn sid app | head -20"),
            grep_of("sid", Some("app"))
        );
        assert_eq!(
            read("cd /work/project; ls; LC_ALL=C grep -n sid app/logout.py"),
            grep_of("sid", Some("app/logout.py"))
        );
        assert_eq!(read("echo hi\nrg sid"), grep_of("sid", None));
        assert_eq!(read("/usr/bin/grep sid"), grep_of("sid", None));
        assert_eq!(
            read("git grep -n logout_token"),
            grep_of("logout_token", None)
        );
    }

    #[test]
    fn a_content_search_wins_over_a_file_listing() {
        assert_eq!(
            read("find . -name '*.py' | xargs grep -l logout_token"),
            grep_of("logout_token", None)
        );
        assert_eq!(
            read("find . -name '*.py' -not -path './.venv/*' | head -50"),
            glob_of("*.py", None)
        );
        assert_eq!(read("find . -type f | grep -v '/.git/' | head -100"), None);
    }

    #[test]
    fn find_gives_its_name_pattern_and_path() {
        assert_eq!(
            read("find app -iname '*logout*'"),
            glob_of("*logout*", Some("app"))
        );
        assert_eq!(
            read("find . -type f -name logout.py"),
            glob_of("logout.py", None)
        );
        assert_eq!(
            read("find /opt/identity-model -path '*/core/*'"),
            glob_of("*/core/*", Some("/opt/identity-model"))
        );
        assert_eq!(read("find . -type f"), None);
    }

    #[test]
    fn fd_gives_its_pattern() {
        assert_eq!(read("fd -e py logout src"), glob_of("logout", Some("src")));
        assert_eq!(read("fd --type=f logout"), glob_of("logout", None));
        assert_eq!(read("fd"), None);
    }

    #[test]
    fn other_commands_are_no_search() {
        assert_eq!(read("cat app/logout.py"), None);
        assert_eq!(read("ls -la"), None);
        assert_eq!(read("uv run pytest -q"), None);
        assert_eq!(read(""), None);
        assert_eq!(read("grep"), None);
        assert_eq!(read("echo 'grep sid'"), None);
        assert_eq!(read("cat a.py # grep sid"), None);
    }

    #[test]
    fn wrappers_run_the_next_word() {
        assert_eq!(read("timeout 5 rg sid"), grep_of("sid", None));
        assert_eq!(
            read("sudo -n grep -r sid /etc"),
            grep_of("sid", Some("/etc"))
        );
        assert_eq!(read("xargs -0 grep -l sid"), grep_of("sid", None));
    }

    #[test]
    fn an_unclosed_quote_never_panics() {
        assert_eq!(read("grep 'sid"), grep_of("sid", None));
        assert_eq!(read("grep \"sid\\"), grep_of("sid\\", None));
        assert_eq!(read("grep sid \\"), grep_of("sid", None));
    }
}
