//! `codecite add`: appends git sources to a corpus manifest, each pinned at
//! its remote's current commit with its license detected, keeping every
//! comment and the layout of what the manifest already says.

use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::Path;

use crate::{
    Clones, DetectedLicense, Error, GitSource, LicenseClass, Manifest, Source, detect_license,
    remote_head,
};

/// One URL [`add_sources`] handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The repository URL as given.
    pub url: String,
    /// The pinned commit; for a skipped URL, the manifest's pin.
    pub commit: String,
    /// The proposed license; for a skipped URL, what the manifest says.
    pub license: DetectedLicense,
    /// Already in the manifest, so left as it was.
    pub skipped: bool,
}

/// Adds each URL in `urls` to the manifest at `path` as a git source, pinned
/// at the remote's current commit, with the license found in its checkout.
///
/// A missing manifest is created for `corpus`. A URL already in the manifest
/// is skipped. The checkouts go through `clones`, so the next index run finds
/// them. A failure on any URL writes nothing.
pub fn add_sources(
    path: &Path,
    corpus: &str,
    urls: &[String],
    clones: &Clones,
) -> crate::Result<Vec<Added>> {
    let mut text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => format!("name = {}\n", quote(corpus)),
        Err(err) => return Err(err.into()),
    };
    let manifest = Manifest::parse(&text)?;
    if manifest.name != corpus {
        return Err(Error::WrongCorpus {
            path: path.to_owned(),
            expected: corpus.to_owned(),
            found: manifest.name,
        });
    }

    // What the manifest already pins, by URL; each URL added joins it, so a
    // URL given twice is added once.
    let mut present: Vec<Added> = manifest
        .sources
        .iter()
        .filter_map(|source| match source {
            Source::Git(git) => Some(Added {
                url: git.url.clone(),
                commit: git.commit.clone(),
                license: DetectedLicense {
                    spdx: git.license.clone(),
                    class: git.license_class,
                },
                skipped: true,
            }),
            _ => None,
        })
        .collect();
    let mut added = Vec::with_capacity(urls.len());
    for url in urls {
        if let Some(pinned) = present.iter().find(|p| &p.url == url) {
            added.push(Added {
                skipped: true,
                ..pinned.clone()
            });
            continue;
        }
        let commit = remote_head(url)?;
        let license = {
            // The checkout holds the clone's lock until it drops at the end
            // of this block.
            let checkout = clones.checkout(&GitSource {
                name: None,
                url: url.clone(),
                commit: commit.clone(),
                license: None,
                license_class: LicenseClass::Adapt,
                include: Vec::new(),
                exclude: Vec::new(),
            })?;
            detect_license(checkout.path())?
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&source_table(url, &commit, &license));
        let entry = Added {
            url: url.clone(),
            commit,
            license,
            skipped: false,
        };
        present.push(entry.clone());
        added.push(entry);
    }
    if added.iter().all(|a| a.skipped) && path.exists() {
        return Ok(added);
    }

    // Never write a manifest `index` would refuse.
    Manifest::parse(&text)?;
    write_atomically(path, &text)?;
    Ok(added)
}

/// The `[[source]]` table for one git source, after a blank line.
fn source_table(url: &str, commit: &str, license: &DetectedLicense) -> String {
    let mut table = format!(
        "\n[[source]]\nkind = \"git\"\nurl = {}\ncommit = {}\n",
        quote(url),
        quote(commit)
    );
    if let Some(spdx) = &license.spdx {
        table.push_str(&format!("license = {}\n", quote(spdx)));
    }
    table.push_str(&format!("use = {}\n", quote(license.class.as_str())));
    table
}

/// `s` as a TOML string, escaped so any URL stays valid TOML.
fn quote(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// Replaces `path` with `text` through a temporary file in the same
/// directory and a rename, so a reader sees the old file or the new one.
fn write_atomically(path: &Path, text: &str) -> crate::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|err| err.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_repo::{Remote, Repo};

    const MIT: &str = include_str!("../licenses/MIT.txt");

    /// A remote whose default branch holds `files`; returns it and its head.
    fn remote(files: &[(&str, &str)]) -> (Remote, String) {
        let repo = Repo::new();
        let mut paths = Vec::new();
        for (name, contents) in files {
            repo.write(name, contents);
            paths.push(*name);
        }
        repo.commit_all(&paths);
        let remote = Remote::new();
        let commit = repo.push(&remote, "main");
        remote.set_head("main");
        (remote, commit)
    }

    struct Setup {
        dir: tempfile::TempDir,
        clones: Clones,
    }

    impl Setup {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let clones = Clones::new(dir.path().join("repos"));
            Setup { dir, clones }
        }

        fn manifest(&self) -> std::path::PathBuf {
            self.dir.path().join("corpora/demo.toml")
        }

        fn add(&self, urls: &[String]) -> crate::Result<Vec<Added>> {
            add_sources(&self.manifest(), "demo", urls, &self.clones)
        }
    }

    fn git_sources(manifest: &Manifest) -> Vec<&GitSource> {
        manifest
            .sources
            .iter()
            .filter_map(|s| match s {
                Source::Git(git) => Some(git),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn add_creates_a_missing_manifest_with_pinned_licensed_sources() {
        let (mit, mit_commit) = remote(&[("LICENSE", MIT), ("a.rs", "fn a() {}\n")]);
        let (bare, bare_commit) = remote(&[("a.rs", "fn a() {}\n")]);
        let setup = Setup::new();

        let added = setup.add(&[mit.url(), bare.url()]).unwrap();

        assert_eq!(
            added,
            [
                Added {
                    url: mit.url(),
                    commit: mit_commit.clone(),
                    license: DetectedLicense {
                        spdx: Some("MIT".into()),
                        class: LicenseClass::Adapt
                    },
                    skipped: false,
                },
                Added {
                    url: bare.url(),
                    commit: bare_commit.clone(),
                    license: DetectedLicense {
                        spdx: None,
                        class: LicenseClass::ApproachOnly
                    },
                    skipped: false,
                },
            ]
        );
        let manifest = Manifest::load(&setup.manifest()).unwrap();
        assert_eq!(manifest.name, "demo");
        let git = git_sources(&manifest);
        assert_eq!(git.len(), 2);
        assert_eq!(
            (git[0].url.as_str(), git[0].commit.as_str()),
            (mit.url().as_str(), mit_commit.as_str())
        );
        assert_eq!(git[0].license.as_deref(), Some("MIT"));
        assert_eq!(git[0].license_class, LicenseClass::Adapt);
        assert_eq!(git[1].commit, bare_commit);
        assert_eq!(git[1].license, None);
        assert_eq!(git[1].license_class, LicenseClass::ApproachOnly);
        // The clone is where the next index run looks for it.
        assert!(setup.clones.path(&mit.url()).join("LICENSE").is_file());
    }

    #[test]
    fn add_keeps_an_existing_manifest_byte_for_byte() {
        let (mit, _) = remote(&[("LICENSE", MIT)]);
        let setup = Setup::new();
        let before = "# Hand-written notes.\nname = \"demo\"\n\n[[source]]\n\
                      kind = \"document\"   # a spec\nname = \"rfc\"\n\
                      url = \"https://example.com/rfc.txt\"\n\
                      sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
                      current = true\nuse = \"quote\"\n";
        fs::create_dir_all(setup.manifest().parent().unwrap()).unwrap();
        fs::write(setup.manifest(), before).unwrap();

        setup.add(&[mit.url()]).unwrap();

        let after = fs::read_to_string(setup.manifest()).unwrap();
        assert!(after.starts_with(before), "{after}");
        assert!(after[before.len()..].starts_with("\n[[source]]\nkind = \"git\"\n"));
        assert_eq!(Manifest::parse(&after).unwrap().sources.len(), 2);
    }

    #[test]
    fn add_skips_a_url_already_in_the_manifest() {
        let (mit, commit) = remote(&[("LICENSE", MIT)]);
        let setup = Setup::new();
        setup.add(&[mit.url()]).unwrap();
        let before = fs::read_to_string(setup.manifest()).unwrap();

        let added = setup.add(&[mit.url()]).unwrap();

        assert_eq!(
            added,
            [Added {
                url: mit.url(),
                commit,
                license: DetectedLicense {
                    spdx: Some("MIT".into()),
                    class: LicenseClass::Adapt
                },
                skipped: true,
            }]
        );
        assert_eq!(fs::read_to_string(setup.manifest()).unwrap(), before);
    }

    #[test]
    fn add_writes_nothing_when_one_url_fails() {
        let (mit, _) = remote(&[("LICENSE", MIT)]);
        let setup = Setup::new();
        let missing = format!("file://{}/nowhere", setup.dir.path().display());

        assert!(setup.add(&[mit.url(), missing.clone()]).is_err());
        assert!(!setup.manifest().exists());

        fs::create_dir_all(setup.manifest().parent().unwrap()).unwrap();
        fs::write(setup.manifest(), "name = \"demo\"\n").unwrap();
        assert!(setup.add(&[mit.url(), missing]).is_err());
        assert_eq!(
            fs::read_to_string(setup.manifest()).unwrap(),
            "name = \"demo\"\n"
        );
    }

    #[test]
    fn add_refuses_a_manifest_for_another_corpus() {
        let (mit, _) = remote(&[("LICENSE", MIT)]);
        let setup = Setup::new();
        fs::create_dir_all(setup.manifest().parent().unwrap()).unwrap();
        fs::write(setup.manifest(), "name = \"other\"\n").unwrap();

        let err = setup.add(&[mit.url()]).unwrap_err();

        assert!(
            matches!(&err, Error::WrongCorpus { expected, found, .. }
                if expected == "demo" && found == "other"),
            "{err:?}"
        );
    }

    #[test]
    fn add_takes_a_repeated_url_once() {
        let (mit, _) = remote(&[("LICENSE", MIT)]);
        let setup = Setup::new();

        let added = setup.add(&[mit.url(), mit.url()]).unwrap();

        assert_eq!(added.len(), 2);
        assert!(!added[0].skipped && added[1].skipped);
        let manifest = Manifest::load(&setup.manifest()).unwrap();
        assert_eq!(git_sources(&manifest).len(), 1);
    }
}
