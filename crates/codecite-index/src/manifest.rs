//! Corpus manifests: the committed TOML file that says what a corpus indexes,
//! at which commit or hash, and how agents may use each source.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use codecite_core::{RoleGlobs, RoleRules, RuleScope, SectionNumber};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// A parsed and validated corpus manifest (`corpora/<name>.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The corpus name.
    pub name: String,
    /// How many days an index stays fresh before queries refuse it.
    pub max_age_days: u32,
    /// The sources, in manifest order.
    pub sources: Vec<Source>,
    /// The sha256 of the manifest's text, in hex, which each index run
    /// records as `meta.manifest_hash`.
    pub sha256: String,
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
    /// A Markdown document of rules, read from a file or fetched and pinned
    /// by its hash (`kind = "guidance"`).
    Guidance(GuidanceSource),
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
    /// A SCIP index of the repository at `commit`, with a leading `~`
    /// expanded; [`Manifest::load`] resolves a relative one against the
    /// manifest's directory.
    pub scip: Option<PathBuf>,
    /// The sha256 the SCIP index must have: 64 lowercase hex digits.
    pub scip_sha256: Option<String>,
    /// Globs that replace the path conventions giving files the test,
    /// example and docs roles (`[source.roles]`); the default keeps them.
    #[serde(default)]
    pub roles: RoleGlobs,
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
    /// A SCIP index of the working tree, with a leading `~` expanded;
    /// [`Manifest::load`] resolves a relative one against the manifest's
    /// directory.
    pub scip: Option<PathBuf>,
    /// The sha256 the SCIP index must have: 64 lowercase hex digits.
    pub scip_sha256: Option<String>,
    /// Globs that replace the path conventions giving files the test,
    /// example and docs roles (`[source.roles]`); the default keeps them.
    #[serde(default)]
    pub roles: RoleGlobs,
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

/// A team's rules: a Markdown document whose headings are rules, each
/// governing the files its scope matches. It is read from `path`, a file on
/// this machine whose hash is taken at each run and checked against
/// `sha256` when that is given, or fetched from `url` and pinned by
/// `sha256` as a document is.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceSource {
    /// The name its rules' citations use.
    pub name: String,
    /// A Markdown file on this machine, with a leading `~` expanded.
    pub path: Option<PathBuf>,
    /// Where the document is fetched from, with `sha256`.
    pub url: Option<String>,
    /// The pinned content hash: 64 lowercase hex digits. Required with
    /// `url`; with `path`, an optional pin that a changed file fails.
    pub sha256: Option<String>,
    /// The license as written by a person (an SPDX expression or a name).
    pub license: Option<String>,
    /// How agents may use this source.
    #[serde(rename = "use")]
    pub license_class: LicenseClass,
    /// Other names citations use for this document.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// The scope of every rule that has none of its own.
    #[serde(default)]
    pub scope: RuleScope,
    /// Scopes by rule number, which win over a rule's own comment.
    #[serde(default)]
    pub rules: BTreeMap<String, RuleScope>,
}

impl GuidanceSource {
    /// A fetched guidance document as the document source the fetcher
    /// takes, or `None` when it is read from `path`.
    pub fn as_document(&self) -> Option<DocumentSource> {
        Some(DocumentSource {
            name: self.name.clone(),
            url: self.url.clone()?,
            sha256: self.sha256.clone()?,
            license: self.license.clone(),
            license_class: self.license_class,
            current: true,
            aliases: self.aliases.clone(),
            obsoleted_by: None,
            updated_by: None,
        })
    }
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
            Source::Guidance(rules) => Some(&rules.name),
        }
    }

    /// The license as the manifest writes it, when it gives one.
    pub fn license(&self) -> Option<&str> {
        match self {
            Source::Git(git) => git.license.as_deref(),
            Source::Local(local) => local.license.as_deref(),
            Source::Document(doc) => doc.license.as_deref(),
            Source::Guidance(rules) => rules.license.as_deref(),
        }
    }

    /// How agents may use this source.
    pub fn license_class(&self) -> LicenseClass {
        match self {
            Source::Git(git) => git.license_class,
            Source::Local(local) => local.license_class,
            Source::Document(doc) => doc.license_class,
            Source::Guidance(rules) => rules.license_class,
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

    /// A guidance source names neither a file nor a pinned URL, or both.
    #[error(
        "source {source_name}: a guidance source needs `path` or `url`, not both, and `url` needs `sha256`"
    )]
    GuidanceOrigin {
        /// The source's label: its name, URL or path, else its position.
        source_name: String,
    },

    /// A scope glob does not compile, or a scope names a language codecite
    /// doesn't detect.
    #[error("source {source_name}: in `{field}`, {message}")]
    Scope {
        /// The source's label: its name, URL or path, else its position.
        source_name: String,
        /// `scope`, or `rules."<number>"`.
        field: String,
        /// What is wrong, naming the glob or language.
        message: String,
    },

    /// A `[source.roles]` glob does not compile.
    #[error("source {source_name}: in `roles.{role}`, {message}")]
    RoleGlob {
        /// The source's label: its name, URL or path, else its position.
        source_name: String,
        /// `test`, `example` or `docs`.
        role: &'static str,
        /// What is wrong, naming the glob.
        message: String,
    },

    /// A source says `use = "adapt"`, but its `license` can't be met
    /// without a copyleft or source-available license.
    #[error(
        "source {source_name}: `use` is \"adapt\", but `license` {license:?} needs {}, which keeps code from being adapted; use \"approach_only\", or leave `license` out for a source you hold the rights to",
        forbidding.join(", ")
    )]
    LicenseForbidsAdapt {
        /// The source's label: its name, URL or path, else its position.
        source_name: String,
        /// The license as written.
        license: String,
        /// The licenses in it that forbid adapting.
        forbidding: Vec<String>,
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

    /// One version of a document is pinned twice.
    #[error("document `{name}`: sha256 {sha256} is pinned twice; remove one")]
    DuplicateVersion {
        /// The document's name.
        name: String,
        /// The repeated pin.
        sha256: String,
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
    codecite_core::DEFAULT_MAX_AGE_DAYS
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
        check_versions(&sources)?;
        Ok(Manifest {
            name: raw.name,
            max_age_days: raw.max_age_days,
            sources,
            sha256: crate::fetch::hex(&Sha256::digest(text)),
        })
    }

    /// Reads and parses the manifest at `path`. A guidance source's
    /// relative `path`, and a relative `scip` index, are resolved against
    /// the manifest's directory, so what they name doesn't depend on where
    /// `codecite index` runs.
    pub fn load(path: &Path) -> crate::Result<Self> {
        let mut manifest = Self::parse(&std::fs::read_to_string(path)?)?;
        let dir = path.parent().unwrap_or(Path::new(""));
        for source in &mut manifest.sources {
            let file = match source {
                Source::Guidance(rules) => rules.path.as_mut(),
                Source::Git(GitSource { scip, .. }) | Source::Local(LocalSource { scip, .. }) => {
                    scip.as_mut()
                }
                Source::Document(_) => None,
            };
            if let Some(file) = file
                && file.is_relative()
            {
                *file = dir.join(&*file);
            }
        }
        Ok(manifest)
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
        Source::Guidance(rules) => {
            match (&rules.path, &rules.url, &rules.sha256) {
                (Some(_), None, _) | (None, Some(_), Some(_)) => {}
                _ => {
                    return Err(ManifestError::GuidanceOrigin {
                        source_name: source_name.clone(),
                    });
                }
            }
            if let Some(sha256) = &rules.sha256
                && !is_lower_hex(sha256, 64)
            {
                return Err(bad_field("sha256", "64 lowercase hex digits", sha256));
            }
            if let Some(path) = rules.path.as_ref().and_then(|p| p.to_str()) {
                rules.path =
                    Some(
                        expand_home(path, home).ok_or_else(|| ManifestError::NoHome {
                            source_name: source_name.clone(),
                        })?,
                    );
            }
            for number in rules.rules.keys() {
                if number
                    .parse::<SectionNumber>()
                    .map(|n| n.to_string())
                    .as_deref()
                    != Ok(number.as_str())
                {
                    return Err(bad_field(
                        "rules",
                        "keyed by rule numbers such as \"3.1\"",
                        number,
                    ));
                }
            }
            let scopes = std::iter::once(("scope".to_owned(), &rules.scope)).chain(
                rules
                    .rules
                    .iter()
                    .map(|(n, scope)| (format!("rules.\"{n}\""), scope)),
            );
            for (field, scope) in scopes {
                scope.check().map_err(|message| ManifestError::Scope {
                    source_name: source_name.clone(),
                    field,
                    message,
                })?;
            }
        }
        _ => {}
    }
    if let Source::Git(GitSource {
        scip, scip_sha256, ..
    })
    | Source::Local(LocalSource {
        scip, scip_sha256, ..
    }) = &source
    {
        match (scip, scip_sha256) {
            (None, Some(_)) => {
                return Err(bad_field(
                    "scip_sha256",
                    "a pin for the index `scip` names",
                    "no `scip`",
                ));
            }
            (_, Some(pin)) if !is_lower_hex(pin, 64) => {
                return Err(bad_field("scip_sha256", "64 lowercase hex digits", pin));
            }
            _ => {}
        }
    }
    if let Source::Git(GitSource { roles, .. }) | Source::Local(LocalSource { roles, .. }) = &source
    {
        check_roles(roles).map_err(|(role, message)| ManifestError::RoleGlob {
            source_name: source_name.clone(),
            role,
            message,
        })?;
    }
    if let Source::Git(GitSource { scip, .. }) | Source::Local(LocalSource { scip, .. }) =
        &mut source
        && let Some(path) = scip.as_ref().and_then(|p| p.to_str())
    {
        *scip = Some(
            expand_home(path, home).ok_or_else(|| ManifestError::NoHome {
                source_name: source_name.clone(),
            })?,
        );
    }
    if source.license_class() == LicenseClass::Adapt
        && let Some(license) = source.license()
        && let Some(forbidding) = crate::license::forbids_adapting(license)
    {
        return Err(ManifestError::LicenseForbidsAdapt {
            source_name,
            license: license.to_owned(),
            forbidding,
        });
    }
    Ok(source)
}

/// Whether `s` is exactly `len` digits of lowercase hex.
/// Compiles each role's globs on its own, so an error names the role whose
/// glob is bad.
fn check_roles(roles: &RoleGlobs) -> Result<(), (&'static str, String)> {
    let one = [
        (
            "test",
            RoleGlobs {
                test: roles.test.clone(),
                ..RoleGlobs::default()
            },
        ),
        (
            "example",
            RoleGlobs {
                example: roles.example.clone(),
                ..RoleGlobs::default()
            },
        ),
        (
            "docs",
            RoleGlobs {
                docs: roles.docs.clone(),
                ..RoleGlobs::default()
            },
        ),
    ];
    for (role, globs) in one {
        RoleRules::new(&globs).map_err(|e| (role, e.to_string()))?;
    }
    Ok(())
}

pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks that no document version is pinned twice and that each document
/// name has exactly one version with `current = true`. Names are checked in
/// order, so the first error is stable.
fn check_versions(sources: &[Source]) -> Result<(), ManifestError> {
    let mut current: BTreeMap<&str, usize> = BTreeMap::new();
    let mut pins = HashSet::new();
    for source in sources {
        if let Source::Document(doc) = source {
            if !pins.insert((&doc.name, &doc.sha256)) {
                return Err(ManifestError::DuplicateVersion {
                    name: doc.name.clone(),
                    sha256: doc.sha256.clone(),
                });
            }
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
    fn the_manifest_hash_is_the_sha256_of_its_text() {
        let text = manifest("");
        let parsed = Manifest::parse(&text).unwrap();
        assert_eq!(
            parsed.sha256,
            crate::fetch::hex(&Sha256::digest(text.as_bytes()))
        );
        assert_ne!(
            parsed.sha256,
            Manifest::parse(&format!("{text}\n")).unwrap().sha256
        );
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
             `git`, `local`, `document`, `guidance` in `kind`"
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
    fn a_scip_index_resolves_against_the_manifests_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpora/s.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                "name = \"s\"\n\
                 [[source]]\nkind = \"local\"\npath = \"/src/app\"\nuse = \"adapt\"\nscip = \"../app.scip\"\n\
                 [[source]]\nurl = \"https://example.com/r.git\"\ncommit = \"{}\"\nuse = \"adapt\"\nscip = \"/abs/r.scip\"\n\
                 [[source]]\nkind = \"local\"\npath = \"/src/other\"\nuse = \"adapt\"\n",
                "a".repeat(40)
            ),
        )
        .unwrap();
        let m = Manifest::load(&path).unwrap();
        let scips: Vec<Option<PathBuf>> = m
            .sources
            .iter()
            .map(|s| match s {
                Source::Git(g) => g.scip.clone(),
                Source::Local(l) => l.scip.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            scips,
            [
                Some(dir.path().join("corpora/../app.scip")),
                Some(PathBuf::from("/abs/r.scip")),
                None
            ]
        );
        // A pin needs an index, and is 64 lowercase hex digits.
        let local = |extra: &str| {
            Manifest::parse(&format!(
                "name = \"p\"\n[[source]]\nkind = \"local\"\npath = \"/a\"\nuse = \"adapt\"\n{extra}"
            ))
        };
        let err = local(&format!("scip_sha256 = \"{}\"\n", "a".repeat(64))).unwrap_err();
        assert!(err.to_string().contains("scip_sha256"), "{err}");
        let err = local("scip = \"x.scip\"\nscip_sha256 = \"ABC\"\n").unwrap_err();
        assert!(err.to_string().contains("64 lowercase hex digits"), "{err}");
        assert!(
            local(&format!(
                "scip = \"x.scip\"\nscip_sha256 = \"{}\"\n",
                "a".repeat(64)
            ))
            .is_ok()
        );
        // A document has no SCIP index.
        let err = Manifest::parse(
            "name = \"d\"\n[[source]]\nkind = \"document\"\nname = \"x\"\nurl = \"https://x\"\n\
             sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
             use = \"quote\"\ncurrent = true\nscip = \"x.scip\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("scip"), "{err}");
    }

    #[test]
    fn load_reads_a_file() {
        let path =
            std::env::temp_dir().join(format!("codecite-manifest-{}.toml", std::process::id()));
        std::fs::write(&path, "name = \"from-file\"\n").unwrap();
        let m = Manifest::load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(m.name, "from-file");

        // A relative guidance path starts at the manifest's directory.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpora/g.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "name = \"g\"\n[[source]]\nkind = \"guidance\"\nname = \"r\"\npath = \"../RULES.md\"\nuse = \"quote\"\n\
             [[source]]\nkind = \"guidance\"\nname = \"a\"\npath = \"/abs/RULES.md\"\nuse = \"quote\"\n",
        )
        .unwrap();
        let m = Manifest::load(&path).unwrap();
        let paths: Vec<PathBuf> = m
            .sources
            .iter()
            .map(|s| match s {
                Source::Guidance(g) => g.path.clone().unwrap(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            paths,
            [
                dir.path().join("corpora/../RULES.md"),
                PathBuf::from("/abs/RULES.md")
            ]
        );

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

    #[test]
    fn a_version_pinned_twice_is_an_error() {
        let text = manifest(&format!(
            "{}{}",
            document("core", SHA256, Some(true)),
            document("core", SHA256, Some(false)),
        ));
        let err = Manifest::parse(&text).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("document `core`: sha256 {SHA256} is pinned twice; remove one")
        );
    }

    #[test]
    fn parses_a_guidance_source() {
        let m = Manifest::parse(&manifest(
            "[[source]]\nkind = \"guidance\"\nname = \"house-rules\"\npath = \"/srv/app/RULES.md\"\n\
             use = \"quote\"\naliases = [\"House Rules\"]\nscope = { languages = [\"python\"] }\n\
             [source.rules.\"3.1\"]\npaths = [\"app/**/handlers/*.py\"]\nimports = [\"requests\"]\n",
        ))
        .unwrap();
        let Source::Guidance(rules) = &m.sources[0] else {
            panic!("{:?}", m.sources[0])
        };
        assert_eq!(rules.path.as_deref(), Some(Path::new("/srv/app/RULES.md")));
        assert_eq!(rules.scope.languages, ["python"]);
        let scope = &rules.rules["3.1"];
        assert_eq!(scope.paths, ["app/**/handlers/*.py"]);
        assert_eq!(scope.imports, ["requests"]);
        assert!(rules.as_document().is_none());
        assert_eq!(m.sources[0].name(), Some("house-rules"));

        let m = Manifest::parse(&manifest(&format!(
            "[[source]]\nkind = \"guidance\"\nname = \"g\"\nurl = \"https://x/R.md\"\nsha256 = \"{SHA256}\"\nuse = \"adapt\"\n"
        )))
        .unwrap();
        let Source::Guidance(rules) = &m.sources[0] else {
            panic!()
        };
        let doc = rules.as_document().unwrap();
        assert_eq!(
            (doc.url.as_str(), doc.sha256.as_str(), doc.current),
            ("https://x/R.md", SHA256, true)
        );
    }

    #[test]
    fn a_guidance_source_needs_one_origin() {
        let want = "source `g`: a guidance source needs `path` or `url`, not both, and `url` needs `sha256`";
        for origin in [
            String::new(),
            "url = \"u\"\n".to_owned(),
            format!("path = \"/a\"\nurl = \"u\"\nsha256 = \"{SHA256}\"\n"),
        ] {
            let err = Manifest::parse(&manifest(&format!(
                "[[source]]\nkind = \"guidance\"\nname = \"g\"\n{origin}use = \"quote\"\n"
            )))
            .unwrap_err();
            assert_eq!(err.to_string(), want, "{origin:?}");
        }
        // A file may be pinned too.
        let pinned = Manifest::parse(&manifest(&format!(
            "[[source]]\nkind = \"guidance\"\nname = \"g\"\npath = \"/a\"\nsha256 = \"{SHA256}\"\nuse = \"quote\"\n"
        )))
        .unwrap();
        let Source::Guidance(rules) = &pinned.sources[0] else {
            panic!()
        };
        assert!(rules.as_document().is_none());
        let err = Manifest::parse(&manifest(
            "[[source]]\nkind = \"guidance\"\nname = \"g\"\nurl = \"u\"\nsha256 = \"abc\"\nuse = \"quote\"\n",
        ))
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("`sha256` must be 64 lowercase hex digits"),
            "{err}"
        );
    }

    #[test]
    fn guidance_scopes_are_checked() {
        let base =
            "[[source]]\nkind = \"guidance\"\nname = \"g\"\npath = \"/a\"\nuse = \"quote\"\n";
        let err = Manifest::parse(&manifest(&format!(
            "{base}scope = {{ paths = [\"a/[\"] }}\n"
        )))
        .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("source `g`: in `scope`, invalid glob \"a/[\""),
            "{err}"
        );
        let err = Manifest::parse(&manifest(&format!(
            "{base}[source.rules.\"2\"]\nlanguages = [\"c#\"]\n"
        )))
        .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("source `g`: in `rules.\"2\"`, unknown language \"c#\""),
            "{err}"
        );
        let err =
            Manifest::parse(&manifest(&format!("{base}[source.rules.\"03\"]\n"))).unwrap_err();
        assert!(
            err.to_string()
                .contains("`rules` must be keyed by rule numbers"),
            "{err}"
        );
        let err = Manifest::parse(&manifest(&format!(
            "{base}[source.rules.\"1\"]\nglobs = []\n"
        )))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field `globs`"), "{err}");
    }

    #[test]
    fn git_and_local_sources_take_role_globs() {
        let m = Manifest::parse(&manifest(&format!(
            "[[source]]\nurl = \"u\"\ncommit = \"{COMMIT}\"\nuse = \"adapt\"\n\
             [source.roles]\ntest = [\"spec/**\", \"**/*_spec.rb\"]\nexample = []\n\
             docs = [\"notes/**\"]\n\
             [[source]]\nkind = \"local\"\npath = \"/a\"\nuse = \"adapt\"\n\
             [source.roles]\nexample = [\"playground/**\"]\n\
             [[source]]\nkind = \"local\"\npath = \"/b\"\nuse = \"adapt\"\n"
        )))
        .unwrap();
        let Source::Git(git) = &m.sources[0] else {
            panic!("{:?}", m.sources[0])
        };
        assert_eq!(
            git.roles,
            RoleGlobs {
                test: Some(vec!["spec/**".into(), "**/*_spec.rb".into()]),
                example: Some(vec![]),
                docs: Some(vec!["notes/**".into()]),
            }
        );
        let Source::Local(local) = &m.sources[1] else {
            panic!("{:?}", m.sources[1])
        };
        assert_eq!(
            local.roles,
            RoleGlobs {
                example: Some(vec!["playground/**".into()]),
                ..RoleGlobs::default()
            }
        );
        let Source::Local(plain) = &m.sources[2] else {
            panic!("{:?}", m.sources[2])
        };
        assert!(plain.roles.is_default());
    }

    #[test]
    fn role_globs_are_checked() {
        let base = "[[source]]\nkind = \"local\"\nname = \"me\"\npath = \"/a\"\nuse = \"adapt\"\n";
        let err = Manifest::parse(&manifest(&format!(
            "{base}[source.roles]\ntests = [\"t/**\"]\n"
        )))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field `tests`"), "{err}");

        let err = Manifest::parse(&manifest(&format!(
            "{base}[source.roles]\nexample = [\"ok/**\"]\ntest = [\"a/[b\"]\n"
        )))
        .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("source `me`: in `roles.test`, invalid role glob \"a/[b\""),
            "{err}"
        );

        let err = Manifest::parse(&manifest(&format!(
            "[[source]]\nkind = \"document\"\nname = \"rfc\"\nurl = \"u\"\n\
             sha256 = \"{SHA256}\"\nuse = \"quote\"\ncurrent = true\n\
             [source.roles]\ntest = []\n"
        )))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field `roles`"), "{err}");

        let err = Manifest::parse(&manifest(
            "[[source]]\nkind = \"guidance\"\nname = \"g\"\npath = \"/a\"\nuse = \"quote\"\n\
             [source.roles]\ntest = []\n",
        ))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field `roles`"), "{err}");
    }

    #[test]
    fn adapt_with_a_copyleft_license_is_refused() {
        let err = Manifest::parse(&manifest(&format!(
            "[[source]]\nname = \"lib\"\nurl = \"u\"\ncommit = \"{COMMIT}\"\n\
             license = \"MIT AND GPL-3.0-only\"\nuse = \"adapt\"\n"
        )))
        .unwrap_err();
        assert!(
            matches!(&err, ManifestError::LicenseForbidsAdapt { source_name, forbidding, .. }
                if source_name == "`lib`" && *forbidding == ["GPL-3.0-only"]),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "source `lib`: `use` is \"adapt\", but `license` \"MIT AND GPL-3.0-only\" needs \
             GPL-3.0-only, which keeps code from being adapted; use \"approach_only\", or \
             leave `license` out for a source you hold the rights to"
        );

        // So is a license that names a copyleft id beside other text.
        let err = Manifest::parse(&manifest(&format!(
            "[[source]]\nname = \"lib\"\nurl = \"u\"\ncommit = \"{COMMIT}\"\n\
             license = \"GPL-3.0-only AND IETF Trust\"\nuse = \"adapt\"\n"
        )))
        .unwrap_err();
        assert!(
            matches!(&err, ManifestError::LicenseForbidsAdapt { forbidding, .. }
                if *forbidding == ["GPL-3.0-only"]),
            "{err:?}"
        );

        // A local source is held to it too.
        let err = Manifest::parse(&manifest(
            "[[source]]\nkind = \"local\"\npath = \"/a\"\nlicense = \"AGPL-3.0-only\"\nuse = \"adapt\"\n",
        ))
        .unwrap_err();
        assert!(
            matches!(err, ManifestError::LicenseForbidsAdapt { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn adapt_stands_where_the_license_allows_it_or_is_not_spdx() {
        for license in [
            Some("MIT OR GPL-3.0-only"),
            Some("Apache-2.0"),
            Some("IETF Trust"),
            None,
        ] {
            let line = license.map_or(String::new(), |l| format!("license = \"{l}\"\n"));
            let m = Manifest::parse(&manifest(&format!(
                "[[source]]\nurl = \"u\"\ncommit = \"{COMMIT}\"\n{line}use = \"adapt\"\n"
            )));
            assert!(m.is_ok(), "{license:?}: {m:?}");
        }
        // A copyleft license with any other use is the person's call.
        for class in ["approach_only", "quote", "excluded"] {
            let m = Manifest::parse(&manifest(&format!(
                "[[source]]\nurl = \"u\"\ncommit = \"{COMMIT}\"\n\
                 license = \"GPL-3.0-only\"\nuse = \"{class}\"\n"
            )));
            assert!(m.is_ok(), "{class}: {m:?}");
        }
    }

    #[test]
    fn the_repositorys_corpora_parse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpora");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "toml") {
                let text = std::fs::read_to_string(&path).unwrap();
                if let Err(err) = Manifest::parse(&text) {
                    panic!("{}: {err}", path.display());
                }
            }
        }
    }
}
