//! `codecite assist` end to end: a corpus indexed from a real git checkout
//! under a temporary `XDG_DATA_HOME`, asked about grep and glob calls the
//! way a hook asks. Linux only, since other platforms ignore
//! `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const LOGOUT: &str = "def validate_logout_token(token):\n    \"\"\"Check a back-channel logout token's claims.\"\"\"\n    return token.claims.get(\"events\") is not None\n";
const SESSION: &str = "def end_session(session):\n    \"\"\"Expire the session after a logout token arrives.\"\"\"\n    session.expired = True\n";

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

/// A corpus `demo` indexed from a checkout with a logout token check and a
/// session function that mentions logout tokens.
struct Demo {
    tmp: tempfile::TempDir,
}

impl Demo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("src/logout.py"), LOGOUT).unwrap();
        fs::write(repo.join("src/session.py"), SESSION).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);

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
        let indexed = demo.run(&["index", "demo"], "");
        assert!(indexed.status.success(), "{}", stderr(&indexed));
        demo
    }

    /// Runs the binary with its data directory in the scratch directory and
    /// `stdin` as its standard input.
    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codecite"))
            .current_dir(self.tmp.path().join("work"))
            .env("HOME", self.tmp.path())
            .env("XDG_DATA_HOME", self.tmp.path().join("data"))
            .env("XDG_CACHE_HOME", self.tmp.path().join("cache"))
            .env_remove("CODECITE_ASSIST_DEBUG")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// Runs `codecite assist demo --tool <tool> --input <input>` with the
    /// tool's output on stdin, checks it succeeded quietly on stderr, and
    /// returns what it printed.
    fn assist(&self, tool: &str, input: &str, output: &str) -> String {
        let out = self.run(
            &["assist", "demo", "--tool", tool, "--input", input],
            output,
        );
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(stderr(&out), "");
        String::from_utf8(out.stdout).unwrap()
    }

    fn db(&self) -> PathBuf {
        self.tmp.path().join("data/codecite/demo.db")
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_symbol_pattern_prints_its_definition() {
    let demo = Demo::new();
    let block = demo.assist("grep", r#"{"pattern": "validate_logout_token"}"#, "");
    assert!(
        block.starts_with("codecite (demo) also found, not in this result:\n"),
        "{block}"
    );
    let first = block.lines().nth(1).unwrap();
    assert!(first.contains(":src/logout.py#L1-L3 · "), "{block}");
    assert!(
        first.contains("validate_logout_token") && first.contains("def validate_logout_token"),
        "{block}"
    );
}

#[test]
fn a_free_text_pattern_prints_search_hits() {
    let demo = Demo::new();
    let block = demo.assist("grep", r#"{"pattern": "logout.*token"}"#, "");
    assert!(block.contains(":src/logout.py#"), "{block}");
    assert!(block.contains(":src/session.py#"), "{block}");
}

#[test]
fn a_glob_prints_hits() {
    let demo = Demo::new();
    let block = demo.assist("glob", r#"{"pattern": "**/*session*.py"}"#, "");
    assert!(block.contains(":src/session.py#"), "{block}");
}

#[test]
fn what_grep_already_found_is_left_out() {
    let demo = Demo::new();
    let found = "/home/u/app/src/logout.py\n";
    let block = demo.assist("grep", r#"{"pattern": "logout.*token"}"#, found);
    assert!(!block.contains("src/logout.py"), "{block}");
    assert!(block.contains(":src/session.py#"), "{block}");

    let both = r#"{"filenames": ["src/logout.py", "src/session.py"], "numFiles": 2}"#;
    let block = demo.assist("grep", r#"{"pattern": "logout.*token"}"#, both);
    assert_eq!(block, "");
}

#[test]
fn a_pattern_with_nothing_behind_it_prints_nothing() {
    let demo = Demo::new();
    assert_eq!(
        demo.assist("grep", r#"{"pattern": "zebra_crossing"}"#, ""),
        ""
    );
    assert_eq!(demo.assist("grep", r#"{"pattern": "^\\s*$"}"#, ""), "");
    assert_eq!(demo.assist("glob", r#"{"pattern": "**/*.rs"}"#, ""), "");
}

#[test]
fn a_missing_corpus_prints_nothing() {
    let demo = Demo::new();
    let out = demo.run(
        &[
            "assist",
            "nope",
            "--tool",
            "grep",
            "--input",
            r#"{"pattern": "validate_logout_token"}"#,
        ],
        "",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"");
    assert_eq!(stderr(&out), "");
}

#[test]
fn a_stale_corpus_prints_nothing() {
    let demo = Demo::new();
    rusqlite::Connection::open(demo.db())
        .unwrap()
        .execute(
            "UPDATE meta SET value = '2026-01-01T00:00:00Z' WHERE key = 'indexed_at'",
            [],
        )
        .unwrap();
    assert_eq!(
        demo.assist("grep", r#"{"pattern": "validate_logout_token"}"#, ""),
        ""
    );
    assert_eq!(
        demo.assist("grep", r#"{"pattern": "logout token"}"#, ""),
        ""
    );
}

#[test]
fn malformed_input_prints_nothing() {
    let demo = Demo::new();
    for input in ["[]", "not json", r#"{"pattern": 3}"#, ""] {
        assert_eq!(demo.assist("grep", input, ""), "", "{input}");
    }
}

#[test]
fn the_debug_variable_explains_an_empty_block() {
    let demo = Demo::new();
    let out = Command::new(env!("CARGO_BIN_EXE_codecite"))
        .env("HOME", demo.tmp.path())
        .env("XDG_DATA_HOME", demo.tmp.path().join("data"))
        .env("CODECITE_ASSIST_DEBUG", "1")
        .args(["assist", "nope", "--tool", "grep", "--input", "[]"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"");
    assert!(
        stderr(&out).contains("codecite assist:"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn the_budget_bounds_the_block() {
    let demo = Demo::new();
    let out = demo.run(
        &[
            "assist",
            "demo",
            "--tool",
            "grep",
            "--input",
            r#"{"pattern": "logout.*token"}"#,
            "--max-bytes",
            "150",
        ],
        "",
    );
    assert!(out.status.success());
    let block = String::from_utf8(out.stdout).unwrap();
    assert!(block.len() <= 150, "{block}");
}

/// Runs assist on a PostToolUse payload Claude Code sent a hook, split the
/// way the documented hook script splits it: `.tool_input` as `--input`,
/// `.tool_response` as stdin.
fn assist_on_payload(demo: &Demo, fixture: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/assist")
        .join(fixture);
    let payload: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let tool = match payload["tool_name"].as_str().unwrap() {
        "Grep" => "grep",
        "Glob" => "glob",
        other => panic!("unexpected tool {other}"),
    };
    demo.assist(
        tool,
        &payload["tool_input"].to_string(),
        &payload["tool_response"].to_string(),
    )
}

#[test]
fn a_claude_code_grep_payload_leaves_out_its_filenames() {
    let demo = Demo::new();
    let block = assist_on_payload(&demo, "claude-grep.json");
    assert!(!block.contains("src/logout.py"), "{block}");
    assert!(block.contains(":src/session.py#"), "{block}");
}

#[test]
fn a_claude_code_glob_payload_prints_hits() {
    let demo = Demo::new();
    let block = assist_on_payload(&demo, "claude-glob.json");
    assert!(block.contains(":src/session.py#"), "{block}");
}

#[test]
fn a_bash_grep_prints_what_its_output_does_not_show() {
    let demo = Demo::new();
    let input = r#"{"command": "cd /work/app && grep -rn 'logout.*token' src | head -20"}"#;
    // Claude Code's Bash result as a hook passes it on: JSON, the lines
    // joined by `\n` escapes, the matched file on the second line.
    let found = r#"{"stdout":"src/other.py:1:x\nsrc/logout.py:2:    \"\"\"Check a back-channel logout token's claims.\"\"\"\n","stderr":"","interrupted":false}"#;
    let block = demo.assist("bash", input, found);
    assert!(!block.contains("src/logout.py"), "{block}");
    assert!(block.contains(":src/session.py#"), "{block}");
}

#[test]
fn a_bash_find_prints_hits() {
    let demo = Demo::new();
    let block = demo.assist("bash", r#"{"command": "find . -name '*session*'"}"#, "");
    assert!(block.contains(":src/session.py#"), "{block}");
}

#[test]
fn a_bash_command_that_searches_nothing_prints_nothing() {
    let demo = Demo::new();
    for command in [
        "cat src/logout.py",
        "find . -type f | grep -v .git",
        "uv run pytest",
    ] {
        let input = serde_json::json!({ "command": command }).to_string();
        assert_eq!(demo.assist("bash", &input, ""), "", "{command}");
    }
}
