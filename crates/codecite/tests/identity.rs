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
