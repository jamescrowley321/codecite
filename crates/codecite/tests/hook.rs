//! `codecite hook claude session-start` end to end: a corpus indexed from a
//! real git checkout under a temporary `XDG_DATA_HOME`. Linux only, since
//! other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

struct Env {
    tmp: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            tmp: tempfile::tempdir().unwrap(),
        }
    }

    /// Indexes a checkout `app` as the corpus `demo`; returns the checkout.
    fn index_demo(&self) -> PathBuf {
        let repo = self.tmp.path().join("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("src/lib.py"), "def hello():\n    return 1\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let work = self.tmp.path().join("work");
        fs::create_dir_all(work.join("corpora")).unwrap();
        fs::write(
            work.join("corpora/demo.toml"),
            format!(
                "name = \"demo\"\nmax_age_days = 7\n\n[[source]]\nkind = \"local\"\npath = {:?}\nuse = \"adapt\"\n",
                repo.to_str().unwrap()
            ),
        )
        .unwrap();
        let out = self.run(&["index", "demo"], "");
        assert!(out.status.success(), "{out:?}");
        repo
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let work = self.tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_codecite"))
            .current_dir(work)
            .env("HOME", self.tmp.path())
            .env("XDG_DATA_HOME", self.tmp.path().join("data"))
            .env("XDG_CACHE_HOME", self.tmp.path().join("cache"))
            .env_remove("CODECITE_CORPUS")
            .env_remove("CODECITE_HOOK_DEBUG")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The hook may exit before reading all of stdin.
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn hook(&self, stdin: &str) -> Output {
        self.run(&["hook", "claude", "session-start"], stdin)
    }
}

#[test]
fn hook_prints_the_note_for_the_checkout() {
    let env = Env::new();
    let repo = env.index_demo();
    let input = serde_json::json!({"cwd": repo.join("src"), "hook_event_name": "SessionStart"});
    let out = env.hook(&input.to_string());
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let out = &reply["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "SessionStart");
    let note = out["additionalContext"].as_str().unwrap();
    assert!(note.contains("`codecite search demo "), "{note}");
    assert!(note.contains("this repository"), "{note}");
}

#[test]
fn hook_survives_bad_stdin() {
    let env = Env::new();
    for stdin in ["", "{", "[]", r#"{"cwd":"/nonexistent"}"#] {
        let out = env.hook(stdin);
        assert!(out.status.success(), "{stdin:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{stdin:?}: {out:?}");
    }
}

#[test]
fn hook_without_codecite_corpora_is_silent() {
    let env = Env::new();
    fs::create_dir_all(env.tmp.path().join("data/codecite")).unwrap();
    let out = env.hook(r#"{"cwd":"."}"#);
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}
