//! `codecite read` end to end: a corpus indexed from a real git checkout
//! under a temporary `XDG_DATA_HOME`, searched, and its citations read
//! back. Linux only, since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

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

/// A corpus `demo` indexed from a one-file checkout.
struct Demo {
    tmp: tempfile::TempDir,
}

impl Demo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("src/lib.rs"), LIB).unwrap();
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
    let citation = results[0]["citation"].as_str().unwrap();
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
        .find(|l| l.starts_with("1. "))
        .unwrap_or_else(|| panic!("{text}"));
    let citation = line["1. ".len()..].split(" · ").next().unwrap();
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
    let citation = results[0]["citation"].as_str().unwrap();
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
    let citation = results[0]["citation"].as_str().unwrap();
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
