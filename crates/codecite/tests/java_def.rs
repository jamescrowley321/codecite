//! Java definitions indexed from a git checkout and read back by citation.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

const JAVA: &str = include_str!("../../codecite-index/tests/fixtures/chunk/sample.java");

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
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

struct JavaCorpus {
    tmp: tempfile::TempDir,
}

impl JavaCorpus {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("src/sample.java"), JAVA).unwrap();
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
        let corpus = Self { tmp };
        let indexed = corpus.run(&["index", "demo"]);
        assert!(
            indexed.status.success(),
            "{}",
            String::from_utf8_lossy(&indexed.stderr)
        );
        corpus
    }

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

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn def(&self, symbol: &str) -> Value {
        self.json(&["def", "demo", symbol, "--json"])
    }
}

fn span(result: &Value) -> &str {
    let citation = result["citation"].as_str().unwrap();
    &citation[citation.find(':').unwrap() + 1..]
}

#[test]
fn java_definitions_have_cited_spans_and_overloads_remain_ambiguous() {
    let corpus = JavaCorpus::new();
    let class = corpus.def("Client");
    // The class and both constructors share this display name.
    assert_eq!(class["total"], 3);
    assert_eq!(class["ambiguous"], true);
    let results = class["results"].as_array().unwrap();
    assert_eq!(
        results.iter().map(span).collect::<Vec<_>>(),
        [
            "src/sample.java#L3-L33",
            "src/sample.java#L6-L7",
            "src/sample.java#L9"
        ]
    );
    assert_eq!(results[0]["kind"], "class");
    assert_eq!(results[1]["kind"], "method");
    assert_eq!(results[0]["providers"], serde_json::json!(["treesitter"]));
    assert_eq!(results[0]["confidence"], 2);

    let methods = corpus.def("Client.validate");
    assert_eq!(methods["total"], 2);
    assert_eq!(methods["ambiguous"], true);
    let methods = methods["results"].as_array().unwrap();
    assert_eq!(
        methods.iter().map(span).collect::<Vec<_>>(),
        ["src/sample.java#L11-L15", "src/sample.java#L17"]
    );
    assert_ne!(methods[0]["id"], methods[1]["id"]);
    assert_eq!(methods[0]["kind"], "method");
    let read = corpus.json(&[
        "read",
        "demo",
        methods[0]["citation"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        read["text"],
        "    /** Validates a token. */\n    @Deprecated\n    public boolean validate(String token) {\n        return token != null;\n    }"
    );

    let nested = corpus.def("Client.Verifier.verify");
    assert_eq!(nested["total"], 1);
    assert_eq!(span(&nested["results"][0]), "src/sample.java#L25-L26");
    assert_eq!(nested["results"][0]["kind"], "method");
    let interface = corpus.def("Verifier");
    assert_eq!(interface["results"][0]["kind"], "interface");
    assert_eq!(span(&interface["results"][0]), "src/sample.java#L23-L27");
    let inner = corpus.def("Client.Inner.run");
    assert_eq!(span(&inner["results"][0]), "src/sample.java#L31");
}
