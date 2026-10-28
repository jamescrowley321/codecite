//! `codecite requirements` end to end, on the real pinned pages of
//! Back-Channel Logout 1.0 and RFC 6749: the binary indexes them from a
//! seeded download cache, so no network is needed, then lists the rules
//! governing a section. Linux only, since other platforms ignore
//! `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The pages as `corpora/identity.toml` pins them, unmodified.
const DOCUMENTS: [(&str, &str, &str); 2] = [
    (
        "openid-connect-backchannel-1_0",
        "3b5c933d5269047b39a65166d5c4d8ff6852b06451f54ed2531b8f97299fdc50",
        "Back-Channel Logout",
    ),
    (
        "rfc6749",
        "535362fa3b4ca668d4734c244c6ed8c811f2ccd7ea1612a4201e5d8922a74b4e",
        "OAuth 2.0",
    ),
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../codecite-index/tests/fixtures/spec")
}

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

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// A home with the corpus `specs` indexed: both pages, put in the
/// download cache under their hashes, which the indexer checks.
fn indexed() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let cache = home.path().join("cache/codecite/documents");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(home.path().join("work/corpora")).unwrap();
    let mut manifest = String::from("name = \"specs\"\n");
    for (name, sha256, alias) in DOCUMENTS {
        fs::copy(fixtures().join(format!("{name}.html")), cache.join(sha256)).unwrap();
        manifest.push_str(&format!(
            "\n[[source]]\nkind = \"document\"\nname = \"{name}\"\n\
             url = \"https://example.invalid/{name}.html\"\nsha256 = \"{sha256}\"\n\
             use = \"quote\"\ncurrent = true\naliases = [\"{alias}\"]\n"
        ));
    }
    fs::write(home.path().join("work/corpora/specs.toml"), manifest).unwrap();
    let out = stdout(&codecite(home.path(), &["index", "specs"]));
    assert!(out.contains("2 from the cache"), "{out}");
    home
}

/// Runs `requirements --json` and returns the result.
fn requirements(home: &Path, args: &[&str]) -> Value {
    let mut all = vec!["requirements", "specs"];
    all.extend_from_slice(args);
    all.push("--json");
    serde_json::from_str(&stdout(&codecite(home, &all))).unwrap()
}

/// The group for `#section-<number>`.
fn group<'a>(result: &'a Value, number: &str) -> &'a Value {
    let anchor = format!("#section-{number}");
    result["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["citation"].as_str().unwrap().ends_with(&anchor))
        .unwrap_or_else(|| panic!("no group for §{number}: {:?}", sections(result)))
}

/// The citations of the groups, for a failure message. The statements
/// themselves stay out of it: RFC 6749's hold example credentials.
fn sections(result: &Value) -> Vec<&str> {
    result["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["citation"].as_str().unwrap())
        .collect()
}

/// Whether a statement of the group holds `words`.
fn holds(group: &Value, words: &str) -> bool {
    group["statements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["text"].as_str().unwrap().contains(words))
}

#[test]
fn back_channel_logout_2_6_lays_out_its_rules() {
    let home = indexed();
    let result = requirements(home.path(), &["Back-Channel Logout §2.6"]);
    let pinned = "openid-connect-backchannel-1_0@sha256:3b5c933d5269";
    assert_eq!(result["citation"], format!("{pinned}#section-2.6"));
    assert_eq!(result["truncated"], false);

    let asked = group(&result, "2.6");
    assert_eq!(asked["relation"], "asked");
    let steps: Vec<&str> = asked["statements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == "step")
        .map(|s| s["citation"].as_str().unwrap())
        .collect();
    for k in 2..=8 {
        let step = format!("{pinned}#section-2.6/step-{k}");
        assert!(steps.contains(&step.as_str()), "{step}: {steps:?}");
    }
    assert!(holds(
        asked,
        "the RP MUST validate the Logout Token as follows"
    ));

    // §2.4, through its title in §2.6's: the claim list.
    let token = group(&result, "2.4");
    assert_eq!(token["relation"], "referenced");
    assert!(holds(token, "exp: REQUIRED. Expiration time"), "{token:#}");
    assert!(holds(
        token,
        "The corresponding member value MUST be a JSON object"
    ));
    assert!(holds(
        token,
        "nonce: PROHIBITED. A nonce Claim MUST NOT be present"
    ));
    assert!(!holds(token, "sub: OPTIONAL"), "{token:#}");

    // The human form names each section and its statements.
    let human = stdout(&codecite(
        home.path(),
        &["requirements", "specs", "Back-Channel Logout §2.6"],
    ));
    assert!(
        human.contains("#section-2.4 · Logout Token (referenced)"),
        "{human}"
    );
    assert!(human.contains("[REQUIRED] exp: REQUIRED."), "{human}");
}

#[test]
fn rfc_6749_2_3_lays_out_its_rules() {
    let home = indexed();
    let result = requirements(home.path(), &["RFC 6749 §2.3"]);
    assert!(
        holds(
            group(&result, "2.3"),
            "MUST NOT use more than one authentication method"
        ),
        "{:?}",
        sections(&result)
    );
    let password = group(&result, "2.3.1");
    assert_eq!(password["relation"], "subsection");
    assert!(holds(password, "MUST NOT be included in the request URI"));
    assert!(holds(password, "client_id: REQUIRED."));
    // §3.2.1 names §2.3, and has its title, which comes first; its
    // enclosing §3.2, the token endpoint, holds the rule on repeated
    // parameters.
    assert_eq!(group(&result, "3.2.1")["relation"], "referenced");
    let endpoint = group(&result, "3.2");
    assert_eq!(endpoint["relation"], "context");
    assert!(holds(endpoint, "MUST NOT be included more than once"));
    // Statements cite their own section, MUSTs first.
    let first = &group(&result, "2.3")["statements"][0];
    assert!(
        first["citation"]
            .as_str()
            .unwrap()
            .ends_with("#section-2.3")
    );
    assert!(
        first["keywords"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k.as_str().unwrap().starts_with("MUST"))
    );
}

#[test]
fn a_topic_and_a_document_pick_the_section() {
    let home = indexed();
    let result = requirements(
        home.path(),
        &[
            "--topic",
            "logout token validation",
            "--document",
            "Back-Channel Logout",
        ],
    );
    assert!(
        result["citation"]
            .as_str()
            .unwrap()
            .ends_with("#section-2.6"),
        "{result:#}"
    );
    // A document as citations write it, not only by name or alias.
    let result = requirements(
        home.path(),
        &["--topic", "client password", "--document", "RFC 6749"],
    );
    assert!(
        result["citation"].as_str().unwrap().starts_with("rfc6749@"),
        "{}",
        result["citation"]
    );
    // A citation and a topic together are refused, and so is neither.
    let both = codecite(
        home.path(),
        &[
            "requirements",
            "specs",
            "RFC 6749 §2.3",
            "--topic",
            "x",
            "--document",
            "rfc6749",
        ],
    );
    assert!(!both.status.success());
    let neither = codecite(home.path(), &["requirements", "specs"]);
    assert!(!neither.status.success());
}
