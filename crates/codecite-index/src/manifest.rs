//! Corpus manifests: the committed TOML file that says what a corpus indexes,
//! at which commit or hash, and how agents may use each source.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A parsed and validated corpus manifest (`corpora/<name>.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The corpus name.
    pub name: String,
    /// How many days an index stays fresh before queries refuse it.
    pub max_age_days: u32,
    /// The sources, in manifest order.
    pub sources: Vec<Source>,
}

/// One `[[source]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A git repository fetched at a pinned commit (`kind = "git"`, the default).
    Git(GitSource),
    /// A working tree read in place (`kind = "local"`).
    Local(LocalSource),
    /// A document fetched by URL and pinned by its hash (`kind = "document"`).
    Document(DocumentSource),
}

/// A git repository pinned to one commit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSource {
    /// The name results cite; `None` until the index pipeline derives one.
    pub name: Option<String>,
    /// The repository URL.
    pub url: String,
    /// The pinned commit: 40 lowercase hex digits.
    pub commit: String,
    /// The license as written by a person (an SPDX expression or a name).
    pub license: Option<String>,
    /// How agents may use this source.
    #[serde(rename = "use")]
    pub license_class: LicenseClass,
    /// Globs of paths to index; empty means everything.
    #[serde(default)]
    pub include: Vec<String>,
    /// Globs of paths to skip.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// A working tree indexed in place, without fetching.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSource {
    /// The name results cite; `None` until the index pipeline derives one.
    pub name: Option<String>,
    /// The working tree's root, with a leading `~` expanded.
    pub path: PathBuf,
    /// The license as written by a person (an SPDX expression or a name).
    pub license: Option<String>,
    /// How agents may use this source.
    #[serde(rename = "use")]
    pub license_class: LicenseClass,
    /// Globs of paths to index; empty means everything.
    #[serde(default)]
    pub include: Vec<String>,
    /// Globs of paths to skip.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// A document (a spec, an RFC) fetched by URL and pinned by its hash.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSource {
    /// The document's name; versions of one document share it.
    pub name: String,
    /// Where the document is fetched from.
    pub url: String,
    /// The pinned content hash: 64 lowercase hex digits.
    pub sha256: String,
    /// The license as written by a person (an SPDX expression or a name).
    pub license: Option<String>,
    /// How agents may use this source.
    #[serde(rename = "use")]
    pub license_class: LicenseClass,
    /// Whether this is the version queries cite by default.
    #[serde(default)]
    pub current: bool,
    /// Other names citations use for this document.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// The document that replaces this one.
    pub obsoleted_by: Option<String>,
    /// A document that amends this one.
    pub updated_by: Option<String>,
}

/// How agents may use a source (`use` in the manifest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseClass {
    /// May adapt code, keeping attribution and the license notice.
    Adapt,
    /// Describe the approach in your own words; never copy code.
    ApproachOnly,
    /// Normative text: quote it exactly with its pinned citation.
    Quote,
    /// Not indexed.
    Excluded,
}

impl LicenseClass {
    /// The value written in the manifest and stored in `source.license_class`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adapt => "adapt",
            Self::ApproachOnly => "approach_only",
            Self::Quote => "quote",
            Self::Excluded => "excluded",
        }
    }
}

impl Source {
    /// The source's name, when the manifest gives one.
    pub fn name(&self) -> Option<&str> {
        match self {
            Source::Git(git) => git.name.as_deref(),
            Source::Local(local) => local.name.as_deref(),
            Source::Document(doc) => Some(&doc.name),
        }
    }

    /// How agents may use this source.
    pub fn license_class(&self) -> LicenseClass {
        match self {
            Source::Git(git) => git.license_class,
            Source::Local(local) => local.license_class,
            Source::Document(doc) => doc.license_class,
        }
    }
}

/// Why a manifest was rejected. Every per-source variant names the source.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// The file is not valid TOML, or its top level has a missing or unknown key.
    #[error("invalid manifest: {0}")]
    Toml(#[from] toml::de::Error),

    /// A source has an unknown kind, a missing or unknown key, or a value of
    /// the wrong type.
    #[error("source {source_name}: {message}")]
    Source {
        /// The source's label: its name, URL or path, else its position.
        source_name: String,
        /// What serde rejected.
        message: String,
    },

    /// A `path` starts with `~` but there is no home directory to expand it to.
    #[error("source {source_name}: `path` starts with `~` but the home directory is unknown")]
    NoHome {
        /// The source's label: its name, URL or path, else its position.
        source_name: String,
    },

    /// A field has a value of the right type but the wrong form.
    #[error("source {source_name}: `{field}` must be {expected}, got {value:?}")]
    Field {
        /// The source's label: its name, URL or path, else its position.
        source_name: String,
        /// The manifest key.
        field: &'static str,
        /// The form the value must take.
        expected: &'static str,
        /// The value as written.
        value: String,
    },

    /// Several versions of one document are marked current.
    #[error("document `{name}`: {count} versions have `current = true`; mark exactly one")]
    SeveralCurrent {
        /// The document's name.
        name: String,
        /// How many versions are marked current.
        count: usize,
    },

    /// No version of a document is marked current.
    #[error("document `{name}`: no version has `current = true`; mark exactly one")]
    NoCurrent {
        /// The document's name.
        name: String,
    },
}

/// The top level before each source is parsed on its own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    name: String,
    #[serde(default = "default_max_age_days")]
    max_age_days: u32,
    #[serde(default)]
    source: Vec<toml::Table>,
}

fn default_max_age_days() -> u32 {
    30
}

impl Manifest {
    /// Parses and validates a manifest's TOML text.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(text)?;
        let home = std::env::home_dir();
        let sources = raw
            .source
            .into_iter()
            .enumerate()
            .map(|(i, table)| parse_source(i, table, home.as_deref()))
            .collect::<Result<Vec<_>, _>>()?;
        check_current(&sources)?;
        Ok(Manifest {
            name: raw.name,
            max_age_days: raw.max_age_days,
            sources,
        })
    }

    /// Reads and parses the manifest at `path`.
    pub fn load(path: &Path) -> crate::Result<Self> {
        Ok(Self::parse(&std::fs::read_to_string(path)?)?)
    }
}

/// Parses the source at `index` (0-based), defaulting its `kind` to `git`,
/// and checks its pinned commit or hash.
fn parse_source(
    index: usize,
    mut table: toml::Table,
    home: Option<&Path>,
) -> Result<Source, ManifestError> {
    let source_name = label(index, &table);
    table.entry("kind").or_insert_with(|| "git".into());
    let mut source = Source::deserialize(table).map_err(|e| ManifestError::Source {
        source_name: source_name.clone(),
        // toml ends its message with a newline and puts the key on its own line.
        message: e.to_string().trim_end().replace('\n', " "),
    })?;
    let bad_field = |field, expected, value: &str| ManifestError::Field {
        source_name: source_name.clone(),
        field,
        expected,
        value: value.to_owned(),
    };
    match &mut source {
        Source::Git(git) if !is_lower_hex(&git.commit, 40) => {
            return Err(bad_field("commit", "40 lowercase hex digits", &git.commit));
        }
        Source::Document(doc) if !is_lower_hex(&doc.sha256, 64) => {
            return Err(bad_field("sha256", "64 lowercase hex digits", &doc.sha256));
        }
        Source::Local(local) => {
            if let Some(path) = local.path.to_str() {
                local.path = expand_home(path, home).ok_or_else(|| ManifestError::NoHome {
                    source_name: source_name.clone(),
                })?;
            }
        }
        _ => {}
    }
    Ok(source)
}

/// Whether `s` is exactly `len` digits of lowercase hex.
pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks that each document name has exactly one version with
/// `current = true`. Names are checked in order, so the first error is stable.
fn check_current(sources: &[Source]) -> Result<(), ManifestError> {
    let mut current: BTreeMap<&str, usize> = BTreeMap::new();
    for source in sources {
        if let Source::Document(doc) = source {
            *current.entry(&doc.name).or_default() += usize::from(doc.current);
        }
    }
    for (name, count) in current {
        match count {
            1 => {}
            0 => {
                return Err(ManifestError::NoCurrent {
                    name: name.to_owned(),
                });
            }
            count => {
                return Err(ManifestError::SeveralCurrent {
                    name: name.to_owned(),
                    count,
                });
            }
        }
    }
    Ok(())
}

/// Names a source in errors: `` `name` ``, `` `url` `` or `` `path` ``, the
/// first that holds a string, else `#n` (1-based).
fn label(index: usize, table: &toml::Table) -> String {
    ["name", "url", "path"]
        .into_iter()
        .find_map(|key| table.get(key)?.as_str())
        .map_or_else(|| format!("#{}", index + 1), |s| format!("`{s}`"))
}

/// Expands a leading `~` or `~/` to `home`. Other paths, including `~user/…`,
/// are returned unchanged. `None` when the path needs a home and there is none.
fn expand_home(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    if path == "~" {
        return home.map(Path::to_path_buf);
    }
    match path.strip_prefix("~/") {
        Some(rest) => home.map(|h| h.join(rest)),
        None => Some(PathBuf::from(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest(sources: &str) -> String {
        format!("name = \"test\"\n\n{sources}")
    }

    #[test]
    fn parses_all_three_kinds() {
        let text = format!(
            r#"name = "identity"
max_age_days = 7

[[source]]
url = "https://github.com/panva/openid-client"
commit = "{COMMIT}"
license = "MIT"
use = "adapt"
include = ["lib/**", "docs/**"]
exclude = ["lib/vendor/**"]

[[source]]
kind = "local"
name = "identity-model"
path = "/srv/identity-model"
use = "approach_only"

[[source]]
kind = "document"
name = "openid-connect-core-1_0"
url = "https://openid.net/specs/openid-connect-core-1_0.html"
sha256 = "{SHA256}"
license = "OpenID Foundation"
use = "quote"
current = true
aliases = ["OIDC Core", "Core"]
updated_by = "openid-connect-core-1_0-errata2"
"#
        );
        let m = Manifest::parse(&text).unwrap();
        assert_eq!(m.name, "identity");
        assert_eq!(m.max_age_days, 7);
        assert_eq!(m.sources.len(), 3);

        let Source::Git(git) = &m.sources[0] else {
            panic!("{:?}", m.sources[0])
        };
        assert_eq!(git.url, "https://github.com/panva/openid-client");
        assert_eq!(git.commit, COMMIT);
        assert_eq!(git.name, None);
        assert_eq!(git.license.as_deref(), Some("MIT"));
        assert_eq!(git.license_class, LicenseClass::Adapt);
        assert_eq!(git.include, ["lib/**", "docs/**"]);
        assert_eq!(git.exclude, ["lib/vendor/**"]);

        let Source::Local(local) = &m.sources[1] else {
            panic!("{:?}", m.sources[1])
        };
        assert_eq!(local.path, Path::new("/srv/identity-model"));
        assert_eq!(local.license, None);
        assert_eq!(m.sources[1].name(), Some("identity-model"));
        assert_eq!(m.sources[1].license_class(), LicenseClass::ApproachOnly);

        let Source::Document(doc) = &m.sources[2] else {
            panic!("{:?}", m.sources[2])
        };
        assert_eq!(doc.name, "openid-connect-core-1_0");
        assert_eq!(doc.sha256, SHA256);
        assert!(doc.current);
        assert_eq!(doc.aliases, ["OIDC Core", "Core"]);
        assert_eq!(doc.obsoleted_by, None);
        assert_eq!(
            doc.updated_by.as_deref(),
            Some("openid-connect-core-1_0-errata2")
        );
        assert_eq!(doc.license_class, LicenseClass::Quote);
    }

    #[test]
    fn defaults() {
        let m = Manifest::parse("name = \"empty\"").unwrap();
        assert_eq!(m.max_age_days, 30);
        assert!(m.sources.is_empty());

        let m = Manifest::parse(&manifest(&format!(
            "[[source]]\nurl = \"u\"\ncommit = \"{COMMIT}\"\nuse = \"excluded\"\n"
        )))
        .unwrap();
        let Source::Git(git) = &m.sources[0] else {
            panic!()
        };
        assert!(git.include.is_empty() && git.exclude.is_empty());
        assert_eq!(git.license_class, LicenseClass::Excluded);
    }

    #[test]
    fn unknown_keys_are_errors() {
        let err = Manifest::parse("name = \"x\"\nmax_age = 3").unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)), "{err:?}");
        assert!(err.to_string().contains("unknown field `max_age`"), "{err}");

        // `commit` belongs to git sources, so a document rejects it.
        let err = Manifest::parse(&manifest(&format!(
            "[[source]]\nkind = \"document\"\nname = \"rfc9449\"\nurl = \"u\"\n\
             sha256 = \"{SHA256}\"\ncommit = \"{COMMIT}\"\nuse = \"quote\"\ncurrent = true\n"
        )))
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "source `rfc9449`: unknown field `commit`, expected one of `name`, `url`, \
             `sha256`, `license`, `use`, `current`, `aliases`, `obsoleted_by`, `updated_by`"
        );
    }

    #[test]
    fn source_errors_name_the_source() {
        let err = Manifest::parse(&manifest(
            "[[source]]\nkind = \"svn\"\nurl = \"https://example.com/r\"\nuse = \"adapt\"\n",
        ))
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "source `https://example.com/r`: unknown variant `svn`, expected one of \
             `git`, `local`, `document` in `kind`"
        );

        let err = Manifest::parse(&manifest(
            "[[source]]\nkind = \"local\"\npath = \"/a\"\nuse = \"adapt\"\n\
             [[source]]\ncommit = \"x\"\nuse = \"adapt\"\n",
        ))
        .unwrap_err();
        assert_eq!(err.to_string(), "source #2: missing field `url`");

        let err = Manifest::parse(&manifest(
            "[[source]]\nkind = \"local\"\nname = \"me\"\npath = \"/a\"\nuse = \"copy\"\n",
        ))
        .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("source `me`: unknown variant `copy`"),
            "{err}"
        );
    }

    #[test]
    fn expand_home_only_expands_a_leading_tilde() {
        let home = Path::new("/home/u");
        let expand = |p| expand_home(p, Some(home)).unwrap();
        assert_eq!(expand("~"), PathBuf::from("/home/u"));
        assert_eq!(expand("~/repos/x"), PathBuf::from("/home/u/repos/x"));
        assert_eq!(expand("~user/x"), PathBuf::from("~user/x"));
        assert_eq!(expand("a/~/b"), PathBuf::from("a/~/b"));
        assert_eq!(expand("/abs"), PathBuf::from("/abs"));
        assert_eq!(expand_home("~/x", None), None);
        assert_eq!(expand_home("/abs", None), Some(PathBuf::from("/abs")));
    }

    #[test]
    fn manifest_error_converts_into_the_crate_error() {
        let err: crate::Error = Manifest::parse("nope").unwrap_err().into();
        assert!(matches!(
            err,
            crate::Error::Manifest(ManifestError::Toml(_))
        ));
    }

    #[test]
    fn load_reads_a_file() {
        let path =
            std::env::temp_dir().join(format!("codecite-manifest-{}.toml", std::process::id()));
        std::fs::write(&path, "name = \"from-file\"\n").unwrap();
        let m = Manifest::load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(m.name, "from-file");

        let err = Manifest::load(Path::new("/nonexistent/codecite.toml")).unwrap_err();
        assert!(matches!(err, crate::Error::Io(_)), "{err:?}");
    }

    fn git(commit: &str) -> String {
        manifest(&format!(
            "[[source]]\nname = \"client\"\nurl = \"u\"\ncommit = \"{commit}\"\nuse = \"adapt\"\n"
        ))
    }

    fn document(name: &str, sha256: &str, current: Option<bool>) -> String {
        let current = current
            .map(|c| format!("current = {c}\n"))
            .unwrap_or_default();
        format!(
            "[[source]]\nkind = \"document\"\nname = \"{name}\"\nurl = \"u\"\n\
             sha256 = \"{sha256}\"\nuse = \"quote\"\n{current}"
        )
    }

    #[test]
    fn bad_commits_are_errors() {
        for commit in [
            &COMMIT[..39],
            &format!("{COMMIT}0"),
            &COMMIT.to_uppercase(),
            &format!("{}g", &COMMIT[..39]),
            "main",
        ] {
            let err = Manifest::parse(&git(commit)).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "source `client`: `commit` must be 40 lowercase hex digits, got {commit:?}"
                )
            );
        }
        assert!(Manifest::parse(&git(COMMIT)).is_ok());
    }

    #[test]
    fn bad_sha256_is_an_error() {
        for sha in [&SHA256[..63], &SHA256.to_uppercase(), "sha256:abc"] {
            let err =
                Manifest::parse(&manifest(&document("rfc9449", sha, Some(true)))).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("source `rfc9449`: `sha256` must be 64 lowercase hex digits, got {sha:?}")
            );
        }
    }

    #[test]
    fn several_current_versions_are_an_error() {
        let other = SHA256.replace('0', "f");
        let text = manifest(&format!(
            "{}{}",
            document("rfc7234", SHA256, Some(true)),
            document("rfc7234", &other, Some(true)),
        ));
        let err = Manifest::parse(&text).unwrap_err();
        assert_eq!(
            err.to_string(),
            "document `rfc7234`: 2 versions have `current = true`; mark exactly one"
        );
    }

    #[test]
    fn no_current_version_is_an_error() {
        let other = SHA256.replace('0', "f");
        for text in [
            manifest(&document("core", SHA256, None)),
            manifest(&format!(
                "{}{}",
                document("core", SHA256, Some(false)),
                document("core", &other, None)
            )),
        ] {
            let err = Manifest::parse(&text).unwrap_err();
            assert_eq!(
                err.to_string(),
                "document `core`: no version has `current = true`; mark exactly one"
            );
        }
    }

    #[test]
    fn one_current_version_among_several_is_valid() {
        let other = SHA256.replace('0', "f");
        let m = Manifest::parse(&manifest(&format!(
            "{}{}",
            document("core", SHA256, Some(false)),
            document("core", &other, Some(true)),
        )))
        .unwrap();
        assert_eq!(m.sources.len(), 2);
    }
}
