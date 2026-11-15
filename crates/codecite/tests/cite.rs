//! `codecite cite` end to end: the real binary checking citations against a
//! fixture corpus written through `Store` under a temporary
//! `XDG_DATA_HOME`. Linux only, since other platforms ignore
//! `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Output};

use codecite_core::{
    ChunkRecord, CiteOptions, DocumentRecord, Keyword, SourceRecord, SpecAliases, Store,
};
use serde_json::{Value, json};

const RFC9449: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";
const CORE: &str = "c0de00000000000000000000000000000000000000000000000000000000c0de";
const RFC2616: &str = "2616000000000000000000000000000000000000000000000000000000002616";
const PINNED: &str = "rfc9449@sha256:1a2b3c4d5e6f";
const CORE_PINNED: &str = "openid-connect-core-1_0@sha256:c0de00000000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

fn document<'a>(name: &'a str, revision: &'a str) -> SourceRecord<'a> {
    SourceRecord {
        name,
        kind: "document",
        url: "https://example.com",
        revision,
        license_spdx: None,
        license_class: "quote",
        is_current: true,
        obsoleted_by: None,
        updated_by: None,
    }
}

fn section<'a>(number: &'a str, anchor: &'a str, text: &'a str) -> ChunkRecord<'a> {
    ChunkRecord {
        kind: "section",
        start_line: 1,
        end_line: 1,
        text,
        number: Some(number),
        anchor: Some(anchor),
        title: Some(text),
        parent: None,
        strength: &[],
        symbol: None,
    }
}

fn step<'a>(number: &'a str, parent: usize, text: &'a str) -> ChunkRecord<'a> {
    ChunkRecord {
        kind: "step",
        anchor: None,
        title: None,
        parent: Some(parent),
        strength: &[Keyword::Must],
        ..section(number, "", text)
    }
}

fn heading<'a>(anchor: &'a str, title: &'a str, lines: (u32, u32)) -> ChunkRecord<'a> {
    ChunkRecord {
        start_line: lines.0,
        end_line: lines.1,
        number: None,
        ..section("", anchor, title)
    }
}

fn spec<'a>(path: &'a str, chunks: Vec<ChunkRecord<'a>>) -> DocumentRecord<'a> {
    DocumentRecord {
        path,
        language: None,
        content_hash: path.as_bytes(),
        size_bytes: 1,
        dirty: false,
        text: None,
        chunks,
    }
}

/// A data home holding the corpus `demo`: a repository `app` with a
/// README, RFC 9449 at two versions, OIDC Core, and RFC 2616, obsoleted by
/// RFC 9110, which is not in it. RFC 9449 and Core both have a §4.3.
fn data_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("codecite");
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::open_read_write(&dir.join("demo.db")).unwrap();
    let app = store
        .upsert_source(&SourceRecord {
            kind: "local",
            license_class: "adapt",
            ..document("app", HEAD)
        })
        .unwrap();
    let readme = DocumentRecord {
        language: Some("markdown"),
        text: Some("# App\nIntro.\n## Install\nRun it.\n"),
        ..spec(
            "README.md",
            vec![
                heading("app", "App", (1, 2)),
                heading("install", "Install", (3, 4)),
            ],
        )
    };
    store.write_document(app, &readme).unwrap();
    let rfc = store.upsert_source(&document("rfc9449", RFC9449)).unwrap();
    let chunks = vec![
        section("4.3", "section-4.3", "Checking DPoP Proofs"),
        step("1", 0, "1. There is not more than one DPoP header."),
        step("2", 0, "2. The DPoP header is a single well-formed JWT."),
    ];
    store.write_document(rfc, &spec("rfc9449", chunks)).unwrap();
    let old = "a".repeat(64);
    let older = store
        .upsert_source(&SourceRecord {
            is_current: false,
            ..document("rfc9449", &old)
        })
        .unwrap();
    let chunks = vec![section("4.3", "section-4.3", "Checking DPoP Proofs")];
    store
        .write_document(older, &spec("rfc9449", chunks))
        .unwrap();
    let core = store
        .upsert_source(&document("openid-connect-core-1_0", CORE))
        .unwrap();
    let chunks = vec![
        section("3.1.3.6", "CodeIDToken", "ID Token"),
        section("3.1.3.7", "IDTokenValidation", "ID Token Validation"),
        step("13", 1, "13. If the auth_time Claim was requested, ..."),
        section("4.3", "section-4.3", "Unrelated"),
    ];
    store
        .write_document(core, &spec("openid-connect-core-1_0", chunks))
        .unwrap();
    let obsolete = store
        .upsert_source(&SourceRecord {
            obsoleted_by: Some("rfc9110"),
            ..document("rfc2616", RFC2616)
        })
        .unwrap();
    let chunks = vec![section("14.9", "section-14.9", "Cache-Control")];
    store
        .write_document(obsolete, &spec("rfc2616", chunks))
        .unwrap();
    let aliases: SpecAliases = [("OIDC Core", "openid-connect-core-1_0")]
        .into_iter()
        .collect();
    store.set_aliases(&aliases).unwrap();
    store.set_indexed_at(30).unwrap();
    home
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .env("HOME", home)
        .env("XDG_DATA_HOME", home)
        .args(args)
        .output()
        .unwrap()
}

/// Runs `codecite cite demo <citation> --json`, expects success, and checks
/// that it prints what `Store::cite` returns.
fn cite(home: &Path, citation: &str) -> Value {
    let output = run(home, &["cite", "demo", citation, "--json"]);
    assert!(output.status.success(), "{citation}: {}", stderr(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let store = Store::open_corpus(&home.join("codecite"), "demo").unwrap();
    let direct = store.cite(citation, &CiteOptions::default()).unwrap();
    let direct = serde_json::to_value(direct).unwrap();
    // Compact, with sorted keys, as `read --json` prints.
    assert_eq!(stdout, format!("{direct}\n"), "{citation}");
    direct
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_clean_citation_resolves_with_no_findings() {
    let home = data_home();
    let r = cite(home.path(), "OIDC Core §3.1.3.7 step 13");
    assert_eq!(r["resolved"], true);
    assert_eq!(
        r["citation"],
        format!("{CORE_PINNED}#section-3.1.3.7/step-13")
    );
    assert_eq!(r["kind"], "step");
    assert_eq!(r["strength"], json!(["MUST"]));
    assert_eq!(r["text"], "13. If the auth_time Claim was requested, ...");
    assert_eq!(r["findings"], json!([]));
    assert!(r.get("candidates").is_none() && r.get("superseded_by").is_none());
}

#[test]
fn a_repository_file_resolves_by_lines_or_anchor() {
    let home = data_home();
    let r = cite(home.path(), "app@0123456:README.md#L2");
    assert_eq!(r["resolved"], true);
    assert_eq!(r["citation"], format!("app@{HEAD}:README.md#L2"));
    assert_eq!(r["kind"], "code");
    assert_eq!(r["text"], "Intro.");
    assert!(r.get("anchor").is_none() && r.get("title").is_none());
    let r = cite(home.path(), "app@0123456:README.md#install");
    assert_eq!(r["citation"], format!("app@{HEAD}:README.md#L3-L4"));
    assert_eq!(r["kind"], "section");
    assert_eq!(
        (&r["title"], &r["anchor"]),
        (&json!("Install"), &json!("install"))
    );
    assert_eq!(r["text"], "## Install\nRun it.");
    assert_eq!(r["findings"], json!([]));
}

#[test]
fn findings_are_a_successful_check() {
    let home = data_home();
    let url = "https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation";
    let mismatch = format!("[OIDC Core §3.1.3.6]({url})");
    for (citation, kind) in [
        ("RFC 9449 §4.9", "no_such_section"),
        ("RFC 9449 §4.3 step 9", "no_such_step"),
        ("§4.3", "ambiguous_document"),
        (mismatch.as_str(), "anchor_label_mismatch"),
    ] {
        let r = cite(home.path(), citation);
        assert_eq!(r["resolved"], false, "{citation}");
        assert!(r.get("citation").is_none(), "{citation}");
        assert_eq!(r["findings"][0]["type"], kind, "{citation}: {r}");
        assert!(!r["candidates"].as_array().unwrap().is_empty(), "{r}");
    }
    let r = cite(home.path(), &mismatch);
    assert_eq!(
        r["findings"][0],
        json!({
            "type": "anchor_label_mismatch",
            "label": "OIDC Core §3.1.3.6",
            "label_citation": format!("{CORE_PINNED}#section-3.1.3.6"),
            "url": url,
            "url_citation": format!("{CORE_PINNED}#section-3.1.3.7"),
        })
    );
    // A link whose text and anchor agree resolves.
    let r = cite(home.path(), &format!("[§3.1.3.7 step 13]({url})"));
    assert_eq!(r["resolved"], true);
    assert_eq!(r["findings"], json!([]));
}

#[test]
fn a_bare_section_in_one_document_resolves() {
    let home = data_home();
    let r = cite(home.path(), "§3.1.3.7 step 13");
    assert_eq!(
        r["citation"],
        format!("{CORE_PINNED}#section-3.1.3.7/step-13")
    );
}

#[test]
fn superseded_versions_and_documents_say_what_replaces_them() {
    let home = data_home();
    let r = cite(home.path(), "rfc9449@sha256:aaaaaaaaaaaa#section-4.3");
    assert_eq!(r["resolved"], true);
    assert_eq!(r["superseded_by"], PINNED);
    let r = cite(home.path(), "RFC 2616 §14.9");
    assert_eq!(r["superseded_by"], "rfc9110");
}

#[test]
fn the_human_form() {
    let home = data_home();
    let output = run(home.path(), &["cite", "demo", "RFC 9449 §4.3 step 9"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "unresolved
no_such_step: {PINNED}#section-4.3 has no step 9
candidates:
  {PINNED}#section-4.3/step-2
  {PINNED}#section-4.3/step-1
"
        )
    );
    let output = run(home.path(), &["cite", "demo", "RFC 2616 §14.9"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "rfc2616@sha256:261600000000#section-14.9");
    assert!(lines[1].starts_with("section · Cache-Control · quote · indexed "));
    assert_eq!(&lines[2..], ["superseded by rfc9110", "", "Cache-Control"]);
}

#[test]
fn errors_exit_non_zero() {
    let home = data_home();
    for (args, want) in [
        (
            vec!["cite", "demo", "app@0123456:README.md#instal"],
            "README.md has no heading #instal; nearest:",
        ),
        (vec!["cite", "demo", "hello world"], "is not a citation"),
        (
            vec!["cite", "demo", "RFC 9999 §1"],
            "no source or document `rfc9999`",
        ),
        (
            vec!["cite", "dmeo", "§4.3"],
            "unknown corpus `dmeo`; known corpora: demo",
        ),
    ] {
        let output = run(home.path(), &args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            stderr(&output).contains(want),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_stale_index_is_refused_unless_allowed() {
    let home = data_home();
    rusqlite::Connection::open(home.path().join("codecite/demo.db"))
        .unwrap()
        .execute(
            "UPDATE meta SET value = '2000-01-01T00:00:00Z' WHERE key = 'indexed_at'",
            [],
        )
        .unwrap();
    let output = run(home.path(), &["cite", "demo", "RFC 9449 §4.3"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("re-run codecite index"),
        "{}",
        stderr(&output)
    );
    let output = run(
        home.path(),
        &["cite", "demo", "RFC 9449 §4.3", "--allow-stale", "--json"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
}
