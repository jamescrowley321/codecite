//! The fhir corpus (`corpora/fhir.toml`): open-source FHIR servers and
//! libraries, and pages of the FHIR R4 specification. This reads the real
//! manifest, not a fixture, and indexes nothing: the corpus is eight
//! repositories and a 3 GB corpus file.

use std::path::Path;

use codecite_index::{LicenseClass, Manifest, Source};

#[test]
fn the_manifest_lists_the_servers_and_the_r4_pages() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpora/fhir.toml");
    let manifest = Manifest::load(&path).unwrap();
    assert_eq!(manifest.name, "fhir");

    let mut repositories = Vec::new();
    let mut pages = Vec::new();
    for source in &manifest.sources {
        match source {
            Source::Git(git) => {
                assert_eq!(git.license_class, LicenseClass::Adapt, "{}", git.url);
                assert!(git.license.is_some(), "{}", git.url);
                repositories.push(git.url.as_str());
            }
            Source::Document(doc) => {
                assert_eq!(doc.license_class, LicenseClass::Quote, "{}", doc.name);
                assert_eq!(doc.license.as_deref(), Some("CC0-1.0"), "{}", doc.name);
                assert!(doc.current, "{}", doc.name);
                assert!(
                    doc.url.starts_with("https://hl7.org/fhir/R4/"),
                    "{}",
                    doc.name
                );
                pages.push(doc.name.as_str());
            }
            other => panic!("unexpected source: {other:?}"),
        }
    }
    assert_eq!(repositories.len(), 8);
    assert!(repositories.contains(&"https://github.com/hapifhir/hapi-fhir"));
    assert_eq!(pages.len(), 19);
    for name in ["fhir-r4-http", "fhir-r4-search", "fhir-r4-json"] {
        assert!(pages.contains(&name), "{name}");
    }
}
