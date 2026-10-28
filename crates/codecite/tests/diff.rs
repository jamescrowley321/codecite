//! `codecite diff` end to end: the real binary diffing two versions of a
//! fixture document written through `Store` under a temporary
//! `XDG_DATA_HOME`. Linux only, since other platforms ignore
//! `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Output};

use codecite_core::{ChunkRecord, DiffOptions, DocumentRecord, SourceRecord, SpecAliases, Store};
use serde_json::Value;

const NEW: &str = "3b5c933d5269047b39a65166d5c4d8ff6852b06451f54ed2531b8f97299fdc50";
const OLD: &str = "02343424f9e52edf8c78173540e25da913ad2784ea292dbf7980222ab1b18a22";
const DOC: &str = "openid-connect-backchannel-1_0";

fn version(store: &mut Store, revision: &str, current: bool, text: &str) {
    let id = store
        .upsert_source(&SourceRecord {
            name: DOC,
            kind: "document",
            url: "https://example.com",
            revision,
            license_spdx: None,
            license_class: "quote",
            is_current: current,
            obsoleted_by: None,
            updated_by: None,
        })
        .unwrap();
    let section = ChunkRecord {
        kind: "section",
        start_line: 1,
        end_line: u32::try_from(text.lines().count()).unwrap(),
        text,
        number: Some("2.4"),
        anchor: Some("LogoutToken"),
        title: Some("Logout Token"),
        parent: None,
        strength: &[],
        symbol: None,
    };
    let doc = DocumentRecord {
        path: DOC,
        language: None,
        content_hash: revision.as_bytes(),
        size_bytes: 1,
        dirty: false,
        text: None,
        chunks: vec![section],
    };
    store.write_document(id, &doc).unwrap();
}

/// A data home holding the corpus `demo`: Back-Channel Logout §2.4 before
/// and after `exp` became REQUIRED.
fn data_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("codecite");
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::open_read_write(&dir.join("demo.db")).unwrap();
    version(
        &mut store,
        OLD,
        false,
        "2.4. Logout Token\n\niat\nREQUIRED. Issued at time, November 2014.",
    );
    version(
        &mut store,
        NEW,
        true,
        "2.4. Logout Token\n\niat\nREQUIRED. Issued at time, December 2023.\nexp\nREQUIRED. Expiration time.",
    );
    let aliases: SpecAliases = [("Back-Channel Logout", DOC)].into_iter().collect();
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

#[test]
fn diff_prints_what_store_diff_returns() {
    let home = data_home();
    let out = run(
        home.path(),
        &["diff", "demo", "Back-Channel Logout", "--json"],
    );
    assert!(out.status.success(), "{out:?}");
    let printed: Value = serde_json::from_slice(&out.stdout).unwrap();
    let store = Store::open_read_only(&home.path().join("codecite/demo.db")).unwrap();
    let expected = store.diff(DOC, &DiffOptions::default()).unwrap();
    assert_eq!(printed, serde_json::to_value(&expected).unwrap());
    assert_eq!(printed["changes"][0]["change"], "added");
    assert_eq!(printed["changes"][0]["to_strength"][0], "REQUIRED");
    assert_eq!(
        printed["changes"][0]["from"],
        format!("{DOC}@sha256:02343424f9e5#section-2.4")
    );
}

#[test]
fn diff_prints_a_human_form() {
    let home = data_home();
    let out = run(home.path(), &["diff", "demo", DOC, "--section", "2.4"]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.starts_with(&format!(
            "{DOC}@sha256:02343424f9e5 → {DOC}@sha256:3b5c933d5269 · section 2.4\n1 normative changes"
        )),
        "{text}"
    );
    assert!(
        text.contains("1. added · section 2.4 · REQUIRED\n"),
        "{text}"
    );
    assert!(
        text.contains("   + exp\n   + REQUIRED. Expiration time.\n"),
        "{text}"
    );
    assert!(
        text.contains("1 reworded, with no normative change:"),
        "{text}"
    );
}

#[test]
fn read_and_search_say_an_older_version_is_pinned() {
    let home = data_home();
    let out = run(home.path(), &["read", "demo", "Back-Channel Logout §2.4"]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains(&format!("older version pinned: {DOC}@sha256:02343424f9e5")),
        "{text}"
    );
    let out = run(home.path(), &["search", "demo", "expiration"]);
    let text = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(text.contains(" · older version pinned\n"), "{text}{out:?}");
}

#[test]
fn an_unknown_document_is_an_error() {
    let home = data_home();
    let out = run(home.path(), &["diff", "demo", "rfc9999"]);
    assert!(!out.status.success());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("rfc9999"), "{err}");
}
