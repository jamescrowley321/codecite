//! `codecite search` end to end: a corpus indexed from a real git checkout
//! under a temporary `XDG_DATA_HOME`, then searched. Linux only, since other
//! platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

/// A corpus `demo` indexed from a two-file checkout, one file dirty.
struct Demo {
    tmp: tempfile::TempDir,
}

impl Demo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("web")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(
            repo.join("src/lib.rs"),
            "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}\n",
        )
        .unwrap();
        fs::write(
            repo.join("web/check.ts"),
            "export function checkToken(token) {\n  return validate(token);\n}\n",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        // An uncommitted edit: results from this file carry `dirty`.
        fs::write(
            repo.join("src/lib.rs"),
            "pub fn validate(token: &str) -> bool {\n    token.len() > 8\n}\n",
        )
        .unwrap();

        let work = tmp.path().join("work");
        fs::create_dir_all(work.join("corpora")).unwrap();
        fs::write(
            work.join("corpora/demo.toml"),
            format!(
                "name = \"demo\"\nmax_age_days = 7\n\n[[source]]\nkind = \"local\"\npath = {:?}\nuse = \"adapt\"\n",
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

    /// Runs `codecite search demo <args> --json` and returns the results it
    /// prints, `{"results": [...]}`.
    fn json(&self, args: &[&str]) -> Vec<serde_json::Value> {
        let mut all = vec!["search", "demo"];
        all.extend(args);
        all.push("--json");
        let output = self.run(&all);
        assert!(output.status.success(), "{}", stderr(&output));
        let mut printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        serde_json::from_value(printed["results"].take()).unwrap()
    }

    fn db(&self) -> PathBuf {
        self.tmp.path().join("data/codecite/demo.db")
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn citations(results: &[serde_json::Value]) -> Vec<&str> {
    results
        .iter()
        .map(|r| r["citation"].as_str().unwrap())
        .collect()
}

#[test]
fn search_prints_cited_results_as_json() {
    let demo = Demo::new();
    let results = demo.json(&["validate"]);
    assert_eq!(results.len(), 2, "{results:#?}");
    let top = &results[0];
    assert_eq!(top["symbol"], "validate");
    assert_eq!(top["kind"], "function");
    assert_eq!(top["license_class"], "adapt");
    assert_eq!(top["dirty"], true);
    assert!(top["indexed_at"].as_str().unwrap().ends_with('Z'));
    assert!(top["snippet"].as_str().unwrap().contains("token.len() > 8"));
    let citation = top["citation"].as_str().unwrap();
    assert!(citation.starts_with("app@"), "{citation}");
    assert!(citation.ends_with(":src/lib.rs#L1-L3"), "{citation}");
    assert_eq!(results[1]["dirty"], false);
}

#[test]
fn filters_narrow_the_results() {
    let demo = Demo::new();
    let ts = demo.json(&["validate", "--language", "typescript"]);
    assert_eq!(citations(&ts).len(), 1);
    assert!(citations(&ts)[0].contains(":web/check.ts#"));
    let src = demo.json(&["validate", "--path", "src/*"]);
    assert_eq!(citations(&src).len(), 1);
    assert!(citations(&src)[0].contains(":src/lib.rs#"));
    assert_eq!(demo.json(&["validate", "--limit", "1"]).len(), 1);
    assert!(demo.json(&["nothing-matches-this"]).is_empty());
}

#[test]
fn the_human_form_is_short() {
    let demo = Demo::new();
    let output = demo.run(&["search", "demo", "check token"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // One header, since every result shares its license and index time.
    assert!(lines[0].contains(" · adapt · indexed "), "{text}");
    assert!(lines[0].contains(" result"), "{text}");
    // The commit is abbreviated to 12 characters.
    let (citation, rest) = lines[1].split_once(" · ").unwrap();
    assert!(citation.starts_with("1. app@"), "{text}");
    assert!(citation.ends_with(":web/check.ts#L1-L3"), "{text}");
    let commit = &citation["1. app@".len()..citation.find(':').unwrap()];
    assert_eq!(commit.len(), 12, "{text}");
    assert!(rest.starts_with("function checkToken · "), "{text}");
    assert!(!rest.contains("indexed"), "{text}");
    assert_eq!(lines[2], "   export function checkToken(token) {");
    assert!(lines.iter().all(|l| l.chars().count() <= 200), "{text}");
}

#[test]
fn a_stale_index_is_refused_unless_allowed() {
    let demo = Demo::new();
    rusqlite::Connection::open(demo.db())
        .unwrap()
        .execute(
            "UPDATE meta SET value = '2026-01-01T00:00:00Z' WHERE key = 'indexed_at'",
            [],
        )
        .unwrap();
    let output = demo.run(&["search", "demo", "validate"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        err.contains("corpus `demo` was indexed at 2026-01-01T00:00:00Z, more than 7 days ago"),
        "{err}"
    );
    assert_eq!(demo.json(&["validate", "--allow-stale"]).len(), 2);
}

#[test]
fn errors_name_the_problem() {
    let demo = Demo::new();
    let cases = [
        (
            vec!["search", "dmeo", "validate"],
            "unknown corpus `dmeo`; known corpora: demo",
        ),
        (vec!["search", "../demo", "x"], "invalid corpus name"),
        (
            vec!["search", "demo", "  !! "],
            "the query has no words to search for",
        ),
    ];
    for (args, want) in cases {
        let output = demo.run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            stderr(&output).contains(want),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}
