//! The identity corpus (`corpora/identity.toml`): identity-model and the
//! OAuth and OpenID specs it cites. These tests read the real manifest, not
//! a fixture. The lookup check downloads every pinned document and indexes
//! the identity-model checkout, so it is ignored by default:
//!
//! ```text
//! cargo test -p codecite --test identity -- --ignored --nocapture
//! ```
//!
//! Linux only, since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use codecite_index::{LicenseClass, Manifest, Source};

/// The workspace root, where `corpora/` lives.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest() -> Manifest {
    Manifest::load(&workspace().join("corpora/identity.toml")).unwrap()
}

#[test]
fn the_manifest_lists_identity_model_and_the_specs() {
    let manifest = manifest();
    assert_eq!(manifest.name, "identity");

    let Source::Local(local) = &manifest.sources[0] else {
        panic!("the first source is identity-model's checkout");
    };
    assert!(local.path.ends_with("repos/auth/identity-model"));
    assert_eq!(local.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(local.license_class, LicenseClass::Adapt);

    let documents: Vec<_> = manifest
        .sources
        .iter()
        .filter_map(|source| match source {
            Source::Document(doc) => Some(doc),
            _ => None,
        })
        .collect();
    assert_eq!(documents.len() + 1, manifest.sources.len());
    for doc in &documents {
        assert_eq!(doc.license_class, LicenseClass::Quote, "{}", doc.name);
        assert!(doc.license.is_some(), "{}", doc.name);
    }

    // The older versions identity-model has cited, kept beside the current ones.
    let older: Vec<&str> = documents
        .iter()
        .filter(|doc| !doc.current)
        .map(|doc| doc.url.as_str())
        .collect();
    assert_eq!(
        older,
        [
            "https://openid.net/specs/openid-connect-core-1_0-final.html",
            "https://openid.net/specs/openid-connect-backchannel-1_0-final.html",
            "https://openid.net/specs/openid-connect-basic-1_0-32.html",
        ]
    );

    // Every RFC the design lists, current.
    for number in [
        2119, 6749, 6750, 7009, 7515, 7517, 7518, 7519, 7521, 7523, 7591, 7592, 7636, 7638, 7662,
        8174, 8414, 8628, 8693, 8705, 8707, 8725, 9101, 9126, 9207, 9396, 9449, 9700,
    ] {
        let name = format!("rfc{number}");
        assert!(
            documents.iter().any(|doc| doc.name == name && doc.current),
            "{name} is missing"
        );
    }
}

/// A citation, the citation `read` should resolve it to (its pinned hash
/// cut out), the section's title and words its text holds.
struct Lookup {
    citation: &'static str,
    resolves_to: (&'static str, &'static str),
    title: &'static str,
    holds: &'static str,
}

const LOOKUPS: &[Lookup] = &[
    Lookup {
        citation: "OIDC Core §3.1.3.7 step 13",
        resolves_to: (
            "openid-connect-core-1_0@sha256:",
            "#section-3.1.3.7/step-13",
        ),
        title: "ID Token Validation",
        holds: "the Client SHOULD check the auth_time Claim value",
    },
    Lookup {
        citation: "RFC 9449 §4.3",
        resolves_to: ("rfc9449@sha256:", "#section-4.3"),
        title: "Checking DPoP Proofs",
        holds: "To validate a DPoP proof, the receiving server MUST ensure",
    },
    Lookup {
        citation: "RFC 6749 §4.1.2.1",
        resolves_to: ("rfc6749@sha256:", "#section-4.1.2.1"),
        title: "Error Response",
        holds: "mismatching\n   redirection URI",
    },
    Lookup {
        citation: "RFC 7636 §4.6",
        resolves_to: ("rfc7636@sha256:", "#section-4.6"),
        title: "Server Verifies code_verifier before Returning the Tokens",
        holds: "calculating the code challenge from the received",
    },
    Lookup {
        citation: "Back-Channel Logout §2.6",
        resolves_to: ("openid-connect-backchannel-1_0@sha256:", "#section-2.6"),
        title: "Logout Token Validation",
        holds: "the RP MUST validate the Logout Token as follows",
    },
    Lookup {
        citation: "RFC 8414 §3",
        resolves_to: ("rfc8414@sha256:", "#section-3"),
        title: "Obtaining Authorization Server Metadata",
        holds: "Authorization servers supporting metadata MUST make a JSON document",
    },
    // A bare § resolves when exactly one current document has the step.
    Lookup {
        citation: "§3.1.3.7 step 12",
        resolves_to: (
            "openid-connect-core-1_0@sha256:",
            "#section-3.1.3.7/step-12",
        ),
        title: "ID Token Validation",
        holds: "If the acr Claim was requested",
    },
];

/// Runs the binary from the workspace root with its corpus file in `data`.
/// Fetched documents stay in the user's cache, where they are checked
/// against their pins, so a second run downloads nothing.
fn run(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .current_dir(workspace())
        .env("XDG_DATA_HOME", data)
        .args(args)
        .output()
        .unwrap()
}

#[test]
#[ignore = "downloads every pinned spec and indexes ~/repos/auth/identity-model; run with `cargo test -p codecite --test identity -- --ignored --nocapture`"]
fn the_identity_corpus_indexes_and_its_lookups_resolve() {
    let data = tempfile::tempdir().unwrap();
    let indexed = run(data.path(), &["index", "identity"]);
    let stdout = String::from_utf8_lossy(&indexed.stdout);
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    println!("$ codecite index identity");
    println!("{}", stdout.lines().skip(1).collect::<Vec<_>>().join("\n"));

    for lookup in LOOKUPS {
        let read = run(
            data.path(),
            &["read", "identity", lookup.citation, "--json"],
        );
        assert!(
            read.status.success(),
            "{}: {}",
            lookup.citation,
            String::from_utf8_lossy(&read.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
        let citation = result["citation"].as_str().unwrap();
        let text = result["text"].as_str().unwrap();
        println!("\n$ codecite read identity {:?}", lookup.citation);
        println!("{citation}  {}  {}", result["title"], result["strength"]);
        println!("{}", text.lines().next().unwrap_or_default());

        let (name, anchor) = lookup.resolves_to;
        assert!(
            citation.starts_with(name) && citation.ends_with(anchor),
            "{}: {citation}",
            lookup.citation
        );
        assert_eq!(result["title"], lookup.title, "{}", lookup.citation);
        assert!(text.contains(lookup.holds), "{}: {text}", lookup.citation);
    }

    // `cite` resolves the same step with no findings.
    let citation = "OIDC Core §3.1.3.7 step 13";
    let cited = run(data.path(), &["cite", "identity", citation, "--json"]);
    assert!(
        cited.status.success(),
        "{}",
        String::from_utf8_lossy(&cited.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&cited.stdout).unwrap();
    println!("\n$ codecite cite identity {citation:?} --json\n{result}");
    assert_eq!(result["resolved"], true, "{result}");
    assert_eq!(result["findings"], serde_json::json!([]), "{result}");
    let pinned = result["citation"].as_str().unwrap();
    assert!(
        pinned.starts_with("openid-connect-core-1_0@sha256:")
            && pinned.ends_with("#section-3.1.3.7/step-13"),
        "{pinned}"
    );

    // Three current documents have a §2.6, so a bare one is not guessed.
    let ambiguous = run(data.path(), &["read", "identity", "§2.6"]);
    let stderr = String::from_utf8_lossy(&ambiguous.stderr);
    println!("\n$ codecite read identity \"§2.6\"\n{}", stderr.trim_end());
    assert!(!ambiguous.status.success());
    for name in ["openid-connect-backchannel-1_0@", "rfc8725@", "rfc9700@"] {
        assert!(stderr.contains(name), "{stderr}");
    }
}
