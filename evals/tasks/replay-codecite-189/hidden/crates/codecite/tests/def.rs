//! `codecite def` end to end: a corpus indexed from a real git checkout
//! under a temporary `XDG_DATA_HOME`, its symbols looked up and their
//! citations read back. Linux only, since other platforms ignore
//! `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

const CLIENT: &str = "pub struct Client;\n\nimpl Client {\n    pub fn callback(&self) -> bool {\n        true\n    }\n}\n";
const SERVER: &str = "pub struct Server;\n\nimpl Server {\n    pub fn callback(&self) {}\n}\n";
const TEST: &str = "fn callback() {\n    assert!(true);\n}\n";

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

/// A corpus `demo` indexed from a checkout with three `callback`s: two
/// methods in source files and a function in a test.
struct Demo {
    tmp: tempfile::TempDir,
}

impl Demo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("tests")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("src/client.rs"), CLIENT).unwrap();
        fs::write(repo.join("src/server.rs"), SERVER).unwrap();
        fs::write(repo.join("tests/callback.rs"), TEST).unwrap();
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

    /// Runs `codecite def demo <symbol> --json` and returns what it prints.
    fn def(&self, symbol: &str) -> Value {
        let output = self.run(&["def", "demo", symbol, "--json"]);
        assert!(output.status.success(), "{}", stderr(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// The text `codecite read` gives for `citation`.
    fn read(&self, citation: &str) -> String {
        let output = self.run(&["read", "demo", citation, "--json"]);
        assert!(output.status.success(), "{}", stderr(&output));
        let read: Value = serde_json::from_slice(&output.stdout).unwrap();
        read["text"].as_str().unwrap().to_owned()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Each result's path and line range, the part of its citation after the
/// commit.
fn spans(found: &Value) -> Vec<&str> {
    found["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let citation = r["citation"].as_str().unwrap();
            &citation[citation.find(':').unwrap() + 1..]
        })
        .collect()
}

#[test]
fn a_unique_symbol_has_one_definition() {
    let demo = Demo::new();
    let found = demo.def("Client");
    assert_eq!(found["symbol"], "Client");
    assert_eq!(found["ambiguous"], false);
    assert_eq!(found["total"], 1);
    assert_eq!(spans(&found), ["src/client.rs#L1"]);
    let def = &found["results"][0];
    assert_eq!(def["kind"], "struct");
    assert_eq!(def["role"], "source");
    assert_eq!(def["providers"], serde_json::json!(["treesitter"]));
    assert_eq!(def["confidence"], 2);
    assert_eq!(
        demo.read(def["citation"].as_str().unwrap()),
        "pub struct Client;"
    );

    let found = demo.def("Client.callback");
    assert_eq!(found["ambiguous"], false);
    assert_eq!(spans(&found), ["src/client.rs#L4-L6"]);
    let citation = found["results"][0]["citation"].as_str().unwrap();
    assert_eq!(
        demo.read(citation),
        "    pub fn callback(&self) -> bool {\n        true\n    }"
    );
}

#[test]
fn an_ambiguous_symbol_lists_every_definition_source_first() {
    let demo = Demo::new();
    let found = demo.def("callback");
    assert_eq!(found["ambiguous"], true);
    assert_eq!(found["total"], 3);
    assert_eq!(
        spans(&found),
        [
            "src/client.rs#L4-L6",
            "src/server.rs#L4",
            "tests/callback.rs#L1-L3"
        ]
    );
    for def in found["results"].as_array().unwrap() {
        let citation = def["citation"].as_str().unwrap();
        assert_eq!(demo.read(citation), def["snippet"].as_str().unwrap());
    }

    let output = demo.run(&["def", "demo", "callback"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("3 definitions of callback · ambiguous · adapt · indexed "),
        "{text}"
    );
    assert!(
        lines[1].contains(
            ":src/client.rs#L4-L6 · method callback · source · pub fn callback(&self) -> bool {"
        ),
        "{text}"
    );
    assert!(
        lines[3].contains(" · function callback · test · "),
        "{text}"
    );
    assert_eq!(lines.len(), 4, "{text}");
}

#[test]
fn a_missing_symbol_is_an_error() {
    let demo = Demo::new();
    let cases = [
        (
            "callBack",
            "the corpus defines no symbol `callBack`; did you mean: callback",
        ),
        ("Other.callback", "did you mean: callback"),
        (
            "nothing",
            "the corpus defines no symbol `nothing`; no candidates",
        ),
        ("  ", "the query has no words to search for"),
    ];
    for (symbol, want) in cases {
        let output = demo.run(&["def", "demo", symbol]);
        assert!(!output.status.success(), "{symbol}");
        assert!(output.stdout.is_empty(), "{symbol}");
        assert!(
            stderr(&output).contains(want),
            "{symbol}: {}",
            stderr(&output)
        );
    }
    let output = demo.run(&["def", "dmeo", "callback"]);
    assert!(stderr(&output).contains("unknown corpus `dmeo`; known corpora: demo"));
}
