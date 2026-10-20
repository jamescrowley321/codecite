//! `codecite index` end to end: a real git checkout, a manifest, and the
//! corpus file under a temporary `XDG_DATA_HOME`. Linux only, since other
//! platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
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

/// Runs the binary in `cwd` with its data directory under `home`.
fn codecite(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
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
