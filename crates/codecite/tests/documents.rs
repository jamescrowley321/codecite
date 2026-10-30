//! Every document indexes and cites the same way: a README and an ADR in a
//! repository and a fetched page that is not a spec each come back from
//! `search` as a section with a title and an anchor, and `read` and `cite`
//! resolve it by its lines or by its anchor to the same text. Linux only,
//! since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

const README: &str = "# App\n\nIntro.\n\n## Install\n\nRun cargo install app.\nThen run app init.\n\n## Usage\n\nRun app.\n";
const ADR: &str = "# 1. Use SQLite\n\n## Context\n\nWe need one file per corpus.\n\n## Decision\n\nStore each corpus in SQLite.\n";
const PAGE: &[u8] = include_bytes!("../../codecite-index/tests/fixtures/document/plain.html");

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Serves `body` on 127.0.0.1 for exactly one request, then stops.
fn serve(body: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/notes.html", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut stream = listener.incoming().next().unwrap().unwrap();
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
            line.clear();
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    });
    url
}

/// Runs the binary in `work` with its data and cache under `home`.
fn codecite(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .current_dir(home.join("work"))
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .args(args)
        .output()
        .unwrap()
}

/// A corpus of one git repository and one fetched page, indexed.
struct Demo {
    tmp: tempfile::TempDir,
    commit: String,
    page_hash: String,
}

impl Demo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let repo = home.join("app");
        fs::create_dir_all(repo.join("docs/adr")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("README.md"), README).unwrap();
        fs::write(repo.join("docs/adr/0001-use-sqlite.md"), ADR).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let commit = git(&repo, &["rev-parse", "HEAD"]);

        let corpora = home.join("work/corpora");
        fs::create_dir_all(&corpora).unwrap();
        let url = serve(PAGE);
        let manifest = |sha256: &str| {
            fs::write(
                corpora.join("demo.toml"),
                format!(
                    "name = \"demo\"\n\n\
                     [[source]]\nkind = \"local\"\npath = {:?}\nuse = \"adapt\"\n\n\
                     [[source]]\nkind = \"document\"\nname = \"notes\"\nurl = {url:?}\n\
                     sha256 = {sha256:?}\nuse = \"quote\"\ncurrent = true\n",
                    repo.to_str().unwrap()
                ),
            )
            .unwrap();
        };

        // The one download pins the page; the index reads it from the cache.
        manifest(&"0".repeat(64));
        let hashes = stdout(&codecite(home, &["index", "demo", "--print-hashes"]));
        let page_hash = hashes.split_whitespace().next().unwrap().to_owned();
        manifest(&page_hash);
        stdout(&codecite(home, &["index", "demo"]));
        Demo {
            tmp,
            commit,
            page_hash,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        codecite(self.tmp.path(), args)
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        serde_json::from_str(&stdout(&self.run(&args))).unwrap()
    }

    /// The first search result for `query`.
    fn top(&self, query: &str) -> Value {
        self.json(&["search", "demo", query])["results"][0].clone()
    }
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Search finds the section at `lines`; `read` and `cite`, by those lines
/// and by its anchor, return its text under that same citation.
fn cites_the_same_way(demo: &Demo, query: &str, lines: &str, anchor: &str, title: &str) {
    let found = demo.top(query);
    assert_eq!(found["citation"], lines, "{found}");
    assert_eq!(found["kind"], "section", "{found}");
    assert_eq!(found["title"], title, "{found}");
    assert_eq!(found["anchor"], anchor, "{found}");

    let (target, _) = lines.split_once('#').unwrap();
    let by_anchor = format!("{target}#{anchor}");
    let text = demo.json(&["read", "demo", lines])["text"].clone();
    assert!(text.is_string(), "{text}");
    for command in ["read", "cite"] {
        for citation in [lines, &by_anchor] {
            let got = demo.json(&[command, "demo", citation]);
            assert_eq!(got["citation"], lines, "{command} {citation}: {got}");
            assert_eq!(got["text"], text, "{command} {citation}: {got}");
            assert_eq!(got["title"], title, "{command} {citation}: {got}");
            assert_eq!(got["anchor"], anchor, "{command} {citation}: {got}");
        }
    }
    let cited = demo.json(&["cite", "demo", &by_anchor]);
    assert_eq!(cited["resolved"], true, "{cited}");
    assert_eq!(cited["findings"], Value::Array(vec![]), "{cited}");
}

#[test]
fn a_readme_an_adr_and_a_fetched_page_cite_the_same_way() {
    let demo = Demo::new();
    let file = |path: &str| format!("app@{}:{path}", demo.commit);
    let readme = file("README.md#L5-L8");
    cites_the_same_way(&demo, "cargo install", &readme, "install", "Install");
    let adr = file("docs/adr/0001-use-sqlite.md#L7-L9");
    cites_the_same_way(&demo, "store each corpus", &adr, "decision", "Decision");
    let page = |fragment: &str| format!("notes@sha256:{}#{fragment}", &demo.page_hash[..12]);
    cites_the_same_way(&demo, "caching", &page("L5-L9"), "v2", "Version 2");

    // The page's text is its rendered text, not its HTML.
    let text = demo.json(&["read", "demo", &page("v2")])["text"].clone();
    assert_eq!(text, "Version 2\n\nAdds caching.\n\nDrops the old flag.");
}

#[test]
fn a_missing_anchor_is_an_error_that_names_it() {
    let demo = Demo::new();
    let citation = format!("app@{}:README.md#instal", demo.commit);
    for command in ["read", "cite"] {
        let output = demo.run(&[command, "demo", &citation]);
        assert!(!output.status.success(), "{command}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("instal"), "{command}: {stderr}");
    }
}
