//! `codecite index` end to end: a real git checkout, a manifest, and the
//! corpus file under a temporary `XDG_DATA_HOME`. Linux only, since other
//! platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::thread::JoinHandle;

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

/// Runs the binary in `cwd` with its data directory under `home`.
fn codecite(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn index_writes_the_corpus_file_and_reindexes_incrementally() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("app");
    fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("src/lib.rs"), "pub fn validate_token() {}\n").unwrap();
    fs::write(repo.join("README.md"), "# App\n").unwrap();
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

    let first = stdout(&codecite(&work, tmp.path(), &["index", "demo"]));
    assert!(
        first.contains("documents: 2 written, 0 unchanged, 0 removed"),
        "{first}"
    );
    assert!(tmp.path().join("data/codecite/demo.db").is_file());
    assert!(!work.join("corpora/demo.db").exists());

    fs::write(repo.join("README.md"), "# App, changed\n").unwrap();
    let second = stdout(&codecite(&work, tmp.path(), &["index", "demo"]));
    assert!(
        second.contains("documents: 1 written, 1 unchanged, 0 removed"),
        "{second}"
    );
}

#[test]
fn index_errors_name_the_problem() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = codecite(tmp.path(), tmp.path(), &["index", "nope"]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        stderr.contains("reading manifest corpora/nope.toml"),
        "{stderr}"
    );

    let escape = codecite(tmp.path(), tmp.path(), &["index", "../etc"]);
    assert!(!escape.status.success());
    assert!(String::from_utf8_lossy(&escape.stderr).contains("invalid corpus name"));

    fs::write(tmp.path().join("other.toml"), "name = \"other\"\n").unwrap();
    let mismatch = codecite(
        tmp.path(),
        tmp.path(),
        &["index", "demo", "--manifest", "other.toml"],
    );
    assert!(!mismatch.status.success());
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("is for corpus `other`"));
}

/// Serves `body` on 127.0.0.1 for exactly `requests` requests, then stops,
/// so any later download fails to connect. Returns the URL and the thread.
fn serve(body: &'static [u8], requests: usize) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/rfc9449.html", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        for stream in listener.incoming().take(requests) {
            let mut stream = stream.unwrap();
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
        }
    });
    (url, thread)
}

fn document_manifest(work: &Path, url: &str, sha256: &str) {
    fs::create_dir_all(work.join("corpora")).unwrap();
    fs::write(
        work.join("corpora/specs.toml"),
        format!(
            "name = \"specs\"\n\n[[source]]\nkind = \"document\"\nname = \"rfc9449\"\n\
             url = {url:?}\nsha256 = {sha256:?}\nuse = \"quote\"\ncurrent = true\n"
        ),
    )
    .unwrap();
}

#[test]
fn documents_are_pinned_by_hash_and_cached() {
    // The smallest xml2rfc v3 page: one numbered section.
    const SPEC: &[u8] = b"<section id=\"section-1\"><h2><a href=\"#section-1\" \
        class=\"section-number selfRef\">1. </a><a href=\"#name-introduction\" \
        class=\"section-name selfRef\">Introduction</a></h2>\n\
        <p>A client MUST send a DPoP proof.</p></section>\n";
    const SPEC_SHA256: &str = "14fc279128304abd07e3ae5da78db59f24f3916bd17e4579545e26eeeb29aef9";
    let old_pin = "0".repeat(64);
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    // Two downloads: --print-hashes, then the run that finds the mismatch.
    let (url, server) = serve(SPEC, 2);
    document_manifest(&work, &url, &old_pin);

    let hashes = stdout(&codecite(
        &work,
        tmp.path(),
        &["index", "specs", "--print-hashes"],
    ));
    assert_eq!(hashes, format!("{SPEC_SHA256}  rfc9449  {url}\n"));
    assert!(!tmp.path().join("data/codecite/specs.db").exists());

    let mismatch = codecite(&work, tmp.path(), &["index", "specs"]);
    assert!(!mismatch.status.success());
    let stderr = String::from_utf8_lossy(&mismatch.stderr);
    assert!(stderr.contains("document `rfc9449`"), "{stderr}");
    assert!(stderr.contains(&old_pin), "{stderr}");
    assert!(stderr.contains(SPEC_SHA256), "{stderr}");
    server.join().unwrap();

    // Re-pinned: the bytes --print-hashes fetched are in the cache, so the
    // run works with the server gone.
    document_manifest(&work, &url, SPEC_SHA256);
    let indexed = stdout(&codecite(&work, tmp.path(), &["index", "specs"]));
    assert!(
        indexed.contains("fetched: 0 downloaded, 1 from the cache"),
        "{indexed}"
    );
    assert!(
        indexed.contains("documents: 1 written, 0 unchanged, 0 removed"),
        "{indexed}"
    );
    let pinned = stdout(&codecite(&work, tmp.path(), &["index", "specs"]));
    assert!(pinned.contains("1 from the cache"), "{pinned}");
    assert!(
        pinned.contains("documents: 0 written, 1 unchanged, 0 removed"),
        "{pinned}"
    );
    assert!(
        tmp.path()
            .join("cache/codecite/documents")
            .join(SPEC_SHA256)
            .is_file()
    );
}
