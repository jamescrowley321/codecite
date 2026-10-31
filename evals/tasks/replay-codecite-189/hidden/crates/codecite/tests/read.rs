//! `codecite read` end to end: a corpus indexed from a real git checkout
//! under a temporary `XDG_DATA_HOME`, searched, and its citations read
//! back. Linux only, since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use codecite_core::{MAX_READ_BYTES, MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES};

/// The one indexed file: five lines, the third a lone `}` that no chunk
/// holds.
const LIB: &str =
    "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}\n\npub fn check() {}\n";

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=codecite",
            "-c",
            "user.email=codecite@example.com",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A corpus `demo` indexed from a checkout.
struct Demo {
    tmp: tempfile::TempDir,
}

impl Demo {
    /// A corpus of one file, `src/lib.rs`.
    fn new() -> Self {
        Self::with(&[("src/lib.rs", LIB)])
    }

    /// A corpus of `files`, each a path and its text.
    fn with(files: &[(&str, &str)]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        for (path, text) in files {
            let path = repo.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let work = tmp.path().join("work");
        fs::create_dir_all(work.join("corpora")).unwrap();
        fs::write(
            work.join("corpora/demo.toml"),
            format!(
                "name = \"demo\"\n\n[[source]]\nkind = \"local\"\npath = {:?}\nuse = \"adapt\"\n",
                repo.to_str().unwrap()
            ),
        )
        .unwrap();
        let demo = Demo { tmp };
        let indexed = demo.run(&["index", "demo"]);
        assert!(indexed.status.success(), "{}", stderr(&indexed));
        demo
    }

    /// Runs the binary with its data directory in the scratch directory.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_codecite"))
            .current_dir(self.tmp.path().join("work"))
            .env("HOME", self.tmp.path())
            .env("XDG_DATA_HOME", self.tmp.path().join("data"))
            .env("XDG_CACHE_HOME", self.tmp.path().join("cache"))
            .args(args)
            .output()
            .unwrap()
    }

    /// Runs the binary, expects success and parses its JSON output.
    fn json(&self, args: &[&str]) -> serde_json::Value {
        let output = self.run(args);
        assert!(output.status.success(), "{}", stderr(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn read_returns_what_a_search_result_cites() {
    let demo = Demo::new();
    let results = demo.json(&["search", "demo", "validate", "--json"]);
    let citation = results["results"][0]["citation"].as_str().unwrap();
    assert!(citation.ends_with(":src/lib.rs#L1-L3"), "{citation}");
    let read = demo.json(&["read", "demo", citation, "--json"]);
    assert_eq!(read["citation"], citation);
    assert_eq!(read["kind"], "code");
    assert_eq!(read["dirty"], false);
    assert_eq!(read["truncated"], false);
    assert_eq!(
        read["text"],
        "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}"
    );
}

#[test]
fn a_citation_from_the_human_search_form_reads_back() {
    let demo = Demo::new();
    let output = demo.run(&["search", "demo", "validate"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with("app@"))
        .unwrap_or_else(|| panic!("{text}"));
    let citation = line.split(" · ").next().unwrap();
    assert!(citation.ends_with(":src/lib.rs#L1-L3"), "{text}");
    let read = demo.json(&["read", "demo", citation, "--json"]);
    assert_eq!(read["kind"], "code");
    assert!(
        read["citation"]
            .as_str()
            .unwrap()
            .starts_with(&citation[..citation.find(':').unwrap()]),
        "{read}"
    );
    assert_eq!(
        read["text"],
        "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}"
    );
}

#[test]
fn the_human_form_and_the_line_cap() {
    let demo = Demo::new();
    let results = demo.json(&["search", "demo", "validate", "--json"]);
    let citation = results["results"][0]["citation"].as_str().unwrap();
    // Lines 1 to 5, as an abbreviated commit cites them.
    let (repo, rest) = citation.split_once('@').unwrap();
    let short = format!("{repo}@{}:src/lib.rs#L1-L5", &rest[..7]);
    let output = demo.run(&["read", "demo", &short, "--max-lines", "3"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("app@") && lines[0].ends_with("#L1-L5"),
        "{text}"
    );
    assert!(lines[1].starts_with("code · adapt · indexed "), "{text}");
    assert_eq!(
        &lines[2..],
        [
            "",
            "pub fn validate(token: &str) -> bool {",
            "    !token.is_empty()",
            "}",
            "… (truncated)"
        ]
    );
}

#[test]
fn errors_name_the_problem() {
    let demo = Demo::new();
    let results = demo.json(&["search", "demo", "validate", "--json"]);
    let citation = results["results"][0]["citation"].as_str().unwrap();
    let past_end = citation.replace("#L1-L3", "#L4-L9");
    let cases = [
        (
            vec!["read", "demo", past_end.as_str()],
            "runs past the end of the file, which has 5 lines".to_owned(),
        ),
        (
            vec!["read", "demo", "hello world"],
            "\"hello world\" is not a citation".to_owned(),
        ),
        (
            vec!["read", "demo", "RFC 9449 §4.3"],
            "the corpus has no source or document `rfc9449`; no candidates".to_owned(),
        ),
        (
            vec!["read", "demo", "RFC 9449 §04.3"],
            "section \"04.3\" is not dot-separated numbers".to_owned(),
        ),
        (
            vec!["read", "dmeo", "§4.3"],
            "unknown corpus `dmeo`; known corpora: demo".to_owned(),
        ),
    ];
    for (args, want) in cases {
        let output = demo.run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            stderr(&output).contains(&want),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

/// A file whose only line is 1 MiB, as a minified bundle's can be: the
/// largest file the index takes.
fn mebibyte_line() -> String {
    let mut line = "token ".repeat(1024 * 1024 / 6 + 1);
    line.truncate(1024 * 1024);
    line
}

/// A line of two-byte characters after five one-byte ones, so both byte
/// budgets, which are even, fall inside a character.
fn wide_line() -> String {
    format!("wide {}", "é".repeat(MAX_READ_BYTES))
}

#[test]
fn a_mebibyte_line_is_cut_to_the_byte_budgets() {
    let line = mebibyte_line();
    let demo = Demo::with(&[("dist/bundle.txt", &line)]);
    let limit = MAX_SEARCH_LIMIT.to_string();
    let results = demo.json(&["search", "demo", "token", "--json", "--limit", &limit]);
    let results = results["results"].as_array().unwrap();
    assert!(!results.is_empty());
    for r in results {
        let snippet = r["snippet"].as_str().unwrap();
        assert!(snippet.len() <= MAX_SNIPPET_BYTES, "{}", snippet.len());
        assert!(line.starts_with(snippet));
        assert_eq!(r["snippet_truncated"], true, "{}", r["citation"]);
    }
    let citation = results[0]["citation"].as_str().unwrap();
    assert!(citation.ends_with(":dist/bundle.txt#L1"), "{citation}");

    let read = demo.json(&["read", "demo", citation, "--max-lines", "1", "--json"]);
    let text = read["text"].as_str().unwrap();
    assert!(text.len() <= MAX_READ_BYTES, "{}", text.len());
    assert!(line.starts_with(text));
    assert_eq!(read["truncated"], true);
    // The citation still names the whole line, and reads back the same.
    assert_eq!(read["citation"], citation);
    assert_eq!(
        demo.json(&["read", "demo", citation, "--max-lines", "1", "--json"]),
        read
    );

    // A grep match's line is held to the snippet budget, and cites the
    // same line.
    let grep = demo.json(&["grep", "demo", "^token", "--json"]);
    let m = &grep["matches"][0];
    let matched = m["text"].as_str().unwrap();
    assert!(matched.len() <= MAX_SNIPPET_BYTES, "{}", matched.len());
    assert!(line.starts_with(matched));
    assert_eq!(m["truncated"], true);
    assert_eq!(m["citation"], citation);
}

#[test]
fn a_cut_falls_on_a_character_boundary() {
    let line = wide_line();
    let demo = Demo::with(&[("dist/wide.txt", &line)]);
    let results = demo.json(&["search", "demo", "wide", "--json"]);
    let top = &results["results"][0];
    let snippet = top["snippet"].as_str().unwrap();
    // `wide ` and the `é`s that fit whole: one byte short of the budget.
    assert_eq!(snippet.len(), MAX_SNIPPET_BYTES - 1);
    assert!(line.starts_with(snippet));
    assert_eq!(top["snippet_truncated"], true);

    let citation = top["citation"].as_str().unwrap();
    let read = demo.json(&["read", "demo", citation, "--json"]);
    let text = read["text"].as_str().unwrap();
    assert_eq!(text.len(), MAX_READ_BYTES - 1);
    assert!(line.starts_with(text));
    assert_eq!(read["truncated"], true);

    let grep = demo.json(&["grep", "demo", "wide", "--fixed", "--json"]);
    let matched = grep["matches"][0]["text"].as_str().unwrap();
    assert_eq!(matched.len(), MAX_SNIPPET_BYTES - 1);
    assert!(line.starts_with(matched));
}
