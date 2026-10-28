//! `codecite context` end to end: a corpus whose guidance is a Markdown
//! file of rules, indexed under a temporary `XDG_DATA_HOME`, asked for the
//! rules that govern files in another repository. Linux only, since other
//! platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const RULES: &str = "# House rules\n\n\
## 1 Settings\n\n<!-- codecite-scope languages = [\"python\"] -->\n\
Code MUST read settings through `get_settings`, never `os.environ`.\n\n\
## 2 Handlers\n\n<!-- codecite-scope\npaths = [\"app/**/handlers/*.py\"]\n-->\n\
Handlers SHOULD log through `audit_log`.\n\n\
## 3 Partner calls\n\n<!-- codecite-scope imports = [\"requests\", \"httpx\"] -->\n\
Code MUST NOT call partners with a raw HTTP client; use `partner_client`.\n\n\
## 4 Headers\n\nEvery file MUST carry the license header.\n";

/// A corpus `demo` whose guidance is [`RULES`], and an empty repository
/// `service` the rules are asked about.
struct Demo {
    tmp: tempfile::TempDir,
}

impl Demo {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir_all(work.join("corpora")).unwrap();
        fs::write(work.join("RULES.md"), RULES).unwrap();
        fs::write(
            work.join("corpora/demo.toml"),
            "name = \"demo\"\n\n[[source]]\nkind = \"guidance\"\nname = \"house-rules\"\n\
             path = \"RULES.md\"\nuse = \"quote\"\naliases = [\"House Rules\"]\n",
        )
        .unwrap();
        // A repository is any directory holding `.git`.
        fs::create_dir_all(tmp.path().join("service/.git")).unwrap();
        fs::create_dir_all(tmp.path().join("service/app/api/handlers")).unwrap();
        let demo = Demo { tmp };
        let indexed = demo.run(&["index", "demo"], None);
        assert!(indexed.status.success(), "{}", stderr(&indexed));
        demo
    }

    /// A path in the `service` repository.
    fn service(&self, path: &str) -> PathBuf {
        self.tmp.path().join("service").join(path)
    }

    /// Runs the binary with its data directory in the scratch directory,
    /// writing `stdin` to it when given.
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codecite"))
            .current_dir(self.tmp.path().join("work"))
            .env("HOME", self.tmp.path())
            .env("XDG_DATA_HOME", self.tmp.path().join("data"))
            .env("XDG_CACHE_HOME", self.tmp.path().join("cache"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut pipe = child.stdin.take().unwrap();
        if let Some(text) = stdin {
            pipe.write_all(text.as_bytes()).unwrap();
        }
        drop(pipe);
        child.wait_with_output().unwrap()
    }

    /// `codecite context demo <path> --json`, with `extra` arguments.
    fn context(&self, path: &Path, extra: &[&str], stdin: Option<&str>) -> Value {
        let mut args = vec!["context", "demo", path.to_str().unwrap(), "--json"];
        args.extend_from_slice(extra);
        let output = self.run(&args, stdin);
        assert!(output.status.success(), "{}", stderr(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Each rule's section number.
fn numbers(context: &Value) -> Vec<&str> {
    context["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["citation"].as_str().unwrap().rsplit('-').next().unwrap())
        .collect()
}

#[test]
fn a_handler_gets_its_rules_must_first() {
    let demo = Demo::new();
    let handler = demo.service("app/api/handlers/partner.py");
    fs::write(&handler, "import requests\n").unwrap();
    let context = demo.context(&handler, &[], None);
    assert_eq!(context["path"], "app/api/handlers/partner.py");
    assert_eq!(context["language"], "python");
    assert_eq!(context["imports"], serde_json::json!(["requests"]));
    assert_eq!(numbers(&context), ["1", "3", "4", "2"]);
    assert_eq!(context["total"], 4);
    let first = &context["rules"][0];
    assert_eq!(first["strength"], serde_json::json!(["MUST"]));
    assert_eq!(first["scope"], serde_json::json!({"languages": ["python"]}));
    assert_eq!(first["title"], "Settings");

    // Its citation reads back, as a spec section's does.
    let citation = first["citation"].as_str().unwrap();
    let read = demo.run(&["read", "demo", citation, "--json"], None);
    assert!(read.status.success(), "{}", stderr(&read));
    let read: Value = serde_json::from_slice(&read.stdout).unwrap();
    assert_eq!(read["text"], first["text"]);
    let alias = demo.run(&["read", "demo", "House Rules §3", "--json"], None);
    assert!(alias.status.success(), "{}", stderr(&alias));
}

#[test]
fn a_new_file_counts_the_imports_it_is_about_to_write() {
    let demo = Demo::new();
    let new = demo.service("lib/client.py");
    let before = demo.context(&new, &[], None);
    assert_eq!(numbers(&before), ["1", "4"]);
    let after = demo.context(&new, &["--stdin"], Some("import httpx\n"));
    assert_eq!(numbers(&after), ["1", "3", "4"]);
}

#[test]
fn a_root_moves_where_globs_start() {
    let demo = Demo::new();
    let handler = demo.service("app/api/handlers/x.py");
    let root = demo.service("app");
    let context = demo.context(&handler, &["--root", root.to_str().unwrap()], None);
    assert_eq!(context["path"], "api/handlers/x.py");
    assert_eq!(numbers(&context), ["1", "4"]);
}

#[test]
fn the_human_form_carries_each_rule_text() {
    let demo = Demo::new();
    let readme = demo.service("README.md");
    let output = demo.run(&["context", "demo", readme.to_str().unwrap()], None);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("1 rule for README.md · markdown"));
    assert_eq!(lines.next(), Some(""));
    let header = lines.next().unwrap();
    assert!(header.starts_with("1. house-rules@sha256:"), "{header}");
    assert!(header.ends_with("#section-4 · MUST · Headers"), "{header}");
    assert_eq!(lines.next(), Some("   ## 4 Headers"));
    assert!(
        text.contains("   Every file MUST carry the license header.\n"),
        "{text}"
    );
}

#[test]
fn a_corpus_without_guidance_has_no_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = Demo { tmp };
    let work = demo.tmp.path().join("work");
    fs::create_dir_all(work.join("corpora")).unwrap();
    fs::write(work.join("corpora/demo.toml"), "name = \"demo\"\n").unwrap();
    assert!(demo.run(&["index", "demo"], None).status.success());
    let output = demo.run(&["context", "demo", "src/a.rs"], None);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "no rules for src/a.rs\n"
    );
}
