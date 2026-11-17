//! `codecite refs` and `codecite graph` end to end: a corpus indexed from a
//! git checkout of the SCIP fixture crate, with its rust-analyzer index
//! named in the manifest, under a temporary `XDG_DATA_HOME`. Linux only,
//! since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

/// The fixture crate and its index, from `codecite-index`'s tests.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../codecite-index/tests/fixtures/scip/app"
);

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

/// A corpus `demo` indexed from the fixture crate with its SCIP index.
struct Demo {
    tmp: tempfile::TempDir,
}

impl Demo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        for file in ["src/lib.rs", "src/client.rs"] {
            fs::copy(Path::new(FIXTURE).join(file), repo.join(file)).unwrap();
        }
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        // The index sits beside the manifest's directory, named relative
        // to it.
        let work = tmp.path().join("work");
        fs::create_dir_all(work.join("corpora")).unwrap();
        fs::copy(Path::new(FIXTURE).join("index.scip"), work.join("app.scip")).unwrap();
        fs::write(
            work.join("corpora/demo.toml"),
            format!(
                "name = \"demo\"\n\n[[source]]\nkind = \"local\"\nname = \"app\"\npath = {:?}\n\
                 use = \"adapt\"\nscip = \"../app.scip\"\n",
                repo.to_str().unwrap()
            ),
        )
        .unwrap();
        let demo = Demo { tmp };
        let indexed = demo.run(&["index", "demo"]);
        assert!(indexed.status.success(), "{}", stderr(&indexed));
        assert!(
            String::from_utf8_lossy(&indexed.stdout).contains("scip app: 2 documents"),
            "{}",
            String::from_utf8_lossy(&indexed.stdout)
        );
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

    /// What a command prints with `--json`, parsed.
    fn json(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let output = self.run(&args);
        assert!(output.status.success(), "{}", stderr(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// What a command prints in its human form.
    fn text(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(output.status.success(), "{}", stderr(&output));
        String::from_utf8(output.stdout).unwrap()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A citation without its source and commit: `src/lib.rs#L10`.
fn span(citation: &Value) -> &str {
    let citation = citation.as_str().unwrap();
    &citation[citation.find(':').unwrap() + 1..]
}

#[test]
fn def_reports_the_scip_id_and_both_providers() {
    let demo = Demo::new();
    let found = demo.json(&["def", "demo", "Client.validate"]);
    let def = &found["results"][0];
    assert_eq!(def["providers"], serde_json::json!(["scip", "treesitter"]));
    assert_eq!(def["confidence"], 3);
    assert_eq!(
        def["id"],
        "rust-analyzer cargo app 0.1.0 client/impl#[Client]validate()."
    );
    assert_eq!(span(&def["citation"]), "src/client.rs#L14-L17");
}

#[test]
fn refs_cites_each_line_that_references_a_symbol() {
    let demo = Demo::new();
    let found = demo.json(&["refs", "demo", "Client.validate"]);
    assert_eq!(
        found["id"],
        "rust-analyzer cargo app 0.1.0 client/impl#[Client]validate()."
    );
    assert_eq!(found["total"], 1);
    let r = &found["results"][0];
    assert_eq!(span(&r["citation"]), "src/lib.rs#L10");
    assert_eq!(r["enclosing"], "rust-analyzer cargo app 0.1.0 login().");
    assert_eq!(r["text"], "    client.validate(token)");
    assert_eq!(r["providers"], serde_json::json!(["scip"]));
    // `read` resolves the citation to the same line.
    let read = demo.json(&["read", "demo", r["citation"].as_str().unwrap()]);
    assert_eq!(read["text"], "    client.validate(token)");

    let text = demo.text(&["refs", "demo", "issuer"]);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with(
            "2 references to rust-analyzer cargo app 0.1.0 client/Client#issuer. · adapt · indexed "
        ),
        "{text}"
    );
    assert!(
        lines[1].ends_with(":src/client.rs#L10 · in new · issuer: String::new(),"),
        "{text}"
    );
    assert!(
        lines[2].ends_with(
            ":src/client.rs#L16 · in validate · check(token) && !self.issuer.is_empty()"
        ),
        "{text}"
    );
    assert_eq!(lines.len(), 3, "{text}");

    // Nothing references `relogin`: an empty answer, not an error.
    let text = demo.text(&["refs", "demo", "relogin"]);
    assert_eq!(
        text,
        "0 references to rust-analyzer cargo app 0.1.0 relogin().\n"
    );
}

#[test]
fn graph_walks_callers_and_callees() {
    let demo = Demo::new();
    let text = demo.text(&["graph", "callers", "demo", "check", "--depth", "3"]);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "3 callers of rust-analyzer cargo app 0.1.0 client/check(). within 3 calls"
    );
    assert!(
        lines[1].starts_with("1 · app@")
            && lines[1].contains(":src/client.rs#L14-L17 · method validate · calls check at app@")
            && lines[1].ends_with(":src/client.rs#L16 · adapt"),
        "{text}"
    );
    assert!(
        lines[2].contains(" · function login · calls validate at "),
        "{text}"
    );
    assert!(
        lines[3].contains(" · function relogin · calls login at "),
        "{text}"
    );
    assert_eq!(lines.len(), 4, "{text}");

    let found = demo.json(&["graph", "callees", "demo", "login"]);
    assert_eq!(found["direction"], "callees");
    assert_eq!(found["depth"], 1);
    let names: Vec<&str> = found["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["new", "validate"]);
    assert_eq!(span(&found["results"][0]["call_site"]), "src/lib.rs#L9");
}

#[test]
fn graph_errors_are_typed() {
    let demo = Demo::new();
    let output = demo.run(&["refs", "demo", "nothing"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("the corpus defines no symbol `nothing`"),
        "{}",
        stderr(&output)
    );
    let output = demo.run(&["graph", "callers", "demo", "check", "--depth", "11"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--depth"), "{}", stderr(&output));
    let output = demo.run(&["refs", "demo", "login", "--min-confidence", "4"]);
    assert!(!output.status.success());
}
