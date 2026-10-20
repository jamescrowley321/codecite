# Corpus Manifests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #17: parse corpus manifests (`corpora/<name>.toml`) with `local`, `git` and `document` sources in `codecite-index`, and reject invalid ones with errors that name the source and the field.

**Architecture:** A new module `crates/codecite-index/src/manifest.rs`. Parsing happens in two serde passes. The first deserializes the top level (`name`, `max_age_days`, and each `[[source]]` as a raw `toml::Table`). The second fills in `kind = "git"` when it is missing and deserializes each table into a `#[serde(tag = "kind")]` enum, so every error from that pass can be prefixed with the source's label. A plain-Rust validation pass then checks commits, hashes and current versions. Manifest errors are their own `ManifestError` enum, wrapped by the crate's `Error`.

**Tech Stack:** Rust 1.99.1 (edition 2024), `serde` 1 (derive), `toml` 1, `thiserror` 2.

**Spec:** Issue #17 and `docs/design.md#corpus-manifests-and-licensing`. The schema's `source` table (`docs/design.md#data-model`) shows where each field lands later: `license` → `license_spdx` (nullable), `use` → `license_class` (not null), `current` → `is_current`.

## Global Constraints

- **Kinds.** `kind` is `git` (the default when the key is absent), `local` or `document`. Any other value is an error naming the source.
- **Fields per kind** (every struct has `#[serde(deny_unknown_fields)]`):
  - `git`: `url` (required), `commit` (required), `name` (optional), `license` (optional), `use` (required), `include`, `exclude` (optional, default empty).
  - `local`: `path` (required, `~` expanded), `name` (optional), `license` (optional), `use` (required), `include`, `exclude`.
  - `document`: `name` (required), `url` (required), `sha256` (required), `license` (optional), `use` (required), `current` (default `false`), `aliases` (default empty), `obsoleted_by`, `updated_by` (optional).
- `use` is required because the schema's `license_class` is `NOT NULL` and the design says a person writes it. `license` is optional because the schema's `license_spdx` is nullable and SPDX detection and `add` stay in M5.
- `include`/`exclude` stay `Vec<String>`. Compiling and checking the globs belongs to the file walker that uses them; this issue adds no glob crate.
- **Top level:** `name` (required), `max_age_days` (`u32`, default 30), `source` (default empty). `deny_unknown_fields` there too.
- **Validation** (after serde):
  - git `commit`: exactly 40 lowercase hex digits. Lowercase matches the canonical citation grammar (#15), which spells commits in lowercase.
  - document `sha256`: exactly 64 lowercase hex digits.
  - documents grouped by `name`: exactly one has `current = true`. Two or more → `SeveralCurrent`; none → `NoCurrent`. Groups are checked in name order (`BTreeMap`) so the first error is deterministic.
  - No other cross-source checks (duplicate git names, `obsoleted_by` pointing at a missing document): the issue doesn't ask for them.
- **Source labels in errors:** `` `<name>` `` when the source has a `name`, else `` `<url>` `` or `` `<path>` ``, else `#<n>` (1-based position). Errors read like ``source `rfc9449`: `sha256` must be 64 lowercase hex digits, got "abc"``.
- **`~` expansion:** a `path` of exactly `~` or starting with `~/` is joined onto the home directory; anything else (including `~user/`) is left unchanged. No home directory when one is needed → `ManifestError::NoHome` naming the source. Expansion is a pure function taking `Option<&Path>` so tests don't depend on `$HOME`.
- `thiserror` treats a field named `source` as the error's cause, so the label field is `source_name`, never `source`.
- The workspace forbids `unsafe_code` and warns on `missing_docs`. Every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- A git source without `kind` must parse as git, and an unknown key inside a source must fail and name that source. Tests: `parses_all_three_kinds`, `unknown_keys_are_errors` (Task 1).
- A short, long or uppercase `commit` and a bad `sha256` must fail and name the source and the field. Tests: `bad_commits_are_errors`, `bad_sha256_is_an_error` (Task 2).
- Two current versions of one document, or none, must fail and name the document. Tests: `several_current_versions_are_an_error`, `no_current_version_is_an_error` (Task 2).
- `~user/x` and `a/~/b` must not be expanded. Test: `expand_home_only_expands_a_leading_tilde` (Task 1).

---

### Task 1: Manifest types and deserialization

**Files:**
- Modify: `Cargo.toml` (workspace dependencies `serde`, `toml`)
- Modify: `crates/codecite-index/Cargo.toml`
- Create: `crates/codecite-index/src/manifest.rs`
- Modify: `crates/codecite-index/src/lib.rs` (module, re-exports, crate doc)
- Modify: `crates/codecite-index/src/error.rs` (`Manifest` variant)

**Interfaces:**
- Produces, re-exported from `codecite_index`:
  - `pub struct Manifest { pub name: String, pub max_age_days: u32, pub sources: Vec<Source> }` with `fn parse(text: &str) -> Result<Manifest, ManifestError>` and `fn load(path: &Path) -> crate::Result<Manifest>`.
  - `pub enum Source { Git(GitSource), Local(LocalSource), Document(DocumentSource) }`, with `fn name(&self) -> Option<&str>` and `fn license_class(&self) -> LicenseClass`.
  - `pub struct GitSource`, `LocalSource`, `DocumentSource` with the public fields listed in Global Constraints (`use` is the field `license_class`).
  - `pub enum LicenseClass { Adapt, ApproachOnly, Quote, Excluded }`, `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`.
  - `pub enum ManifestError`, `#[non_exhaustive]`.
  - `Error::Manifest(#[from] ManifestError)`, `#[error(transparent)]`.

- [ ] **Step 1: Add the dependencies**

`Cargo.toml`, under `[workspace.dependencies]`:

```toml
serde = { version = "1", features = ["derive"] }
toml = "1"
```

`crates/codecite-index/Cargo.toml`, under `[dependencies]`:

```toml
serde.workspace = true
toml.workspace = true
```

Run: `cargo deny check`
Expected: green. `toml` 1 pulls `serde_spanned`, `toml_datetime`, `toml_parser`, `toml_writer`, `winnow`, `indexmap`, `hashbrown` 0.17.1 (already in the lock), all MIT and/or Apache-2.0. If a new duplicate version appears, prefer `toml = { version = "1", default-features = false, features = ["parse", "serde", "std"] }` over a `skip` entry.

- [ ] **Step 2: Write the failing tests**

Create `crates/codecite-index/src/manifest.rs` with only the test module (and `use super::*;`), plus `mod manifest;` in `lib.rs`:

```rust
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

        let Source::Git(git) = &m.sources[0] else { panic!("{:?}", m.sources[0]) };
        assert_eq!(git.url, "https://github.com/panva/openid-client");
        assert_eq!(git.commit, COMMIT);
        assert_eq!(git.name, None);
        assert_eq!(git.license.as_deref(), Some("MIT"));
        assert_eq!(git.license_class, LicenseClass::Adapt);
        assert_eq!(git.include, ["lib/**", "docs/**"]);
        assert_eq!(git.exclude, ["lib/vendor/**"]);

        let Source::Local(local) = &m.sources[1] else { panic!("{:?}", m.sources[1]) };
        assert_eq!(local.path, Path::new("/srv/identity-model"));
        assert_eq!(local.license, None);
        assert_eq!(m.sources[1].name(), Some("identity-model"));
        assert_eq!(m.sources[1].license_class(), LicenseClass::ApproachOnly);

        let Source::Document(doc) = &m.sources[2] else { panic!("{:?}", m.sources[2]) };
        assert_eq!(doc.name, "openid-connect-core-1_0");
        assert_eq!(doc.sha256, SHA256);
        assert!(doc.current);
        assert_eq!(doc.aliases, ["OIDC Core", "Core"]);
        assert_eq!(doc.obsoleted_by, None);
        assert_eq!(doc.updated_by.as_deref(), Some("openid-connect-core-1_0-errata2"));
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
        let Source::Git(git) = &m.sources[0] else { panic!() };
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
             `git`, `local`, `document`"
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
            err.to_string().starts_with("source `me`: unknown variant `copy`"),
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
        assert!(matches!(err, crate::Error::Manifest(ManifestError::Toml(_))));
    }

    #[test]
    fn load_reads_a_file() {
        let path = std::env::temp_dir().join(format!("codecite-manifest-{}.toml", std::process::id()));
        std::fs::write(&path, "name = \"from-file\"\n").unwrap();
        let m = Manifest::load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(m.name, "from-file");

        let err = Manifest::load(Path::new("/nonexistent/codecite.toml")).unwrap_err();
        assert!(matches!(err, crate::Error::Io(_)), "{err:?}");
    }
}
```

The exact `expected one of …` lists come from serde and follow field declaration order; declare the struct fields in the order the test lists them. If serde's wording differs on this toolchain, keep the source prefix assertion exact and loosen only the serde tail.

Run: `cargo test -p codecite-index manifest`
Expected: does not compile (`Manifest`, `Source`, … not found).

- [ ] **Step 3: Write the types**

Above the test module in `manifest.rs`:

```rust
//! Corpus manifests: the committed TOML file that says what a corpus indexes,
//! at which commit or hash, and how agents may use each source.

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
```

Learn — `#[serde(tag = "kind")]`: an *internally tagged* enum. serde reads the `kind` key, picks the variant whose snake_case name matches, and hands the remaining keys to that variant's struct. The tag can't have a default, which is why `parse` inserts `kind = "git"` itself (Step 4).

```rust
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
```

Learn — `deny_unknown_fields`: serde ignores keys a struct doesn't declare unless you ask it not to. With it, a typo such as `comit = "…"` is an error instead of a silently missing pin. `#[serde(rename = "use")]` maps the TOML key `use`, a Rust keyword, onto a field with a usable name. `Option<T>` fields are optional without `#[serde(default)]`; `Vec` and `bool` fields need it.

`LocalSource` has `name: Option<String>`, `path: PathBuf`, `license`, `license_class`, `include`, `exclude`, in that order. `DocumentSource` has, in this order, `name: String`, `url: String`, `sha256: String`, `license: Option<String>`, `license_class` (renamed `use`), `#[serde(default)] current: bool`, `#[serde(default)] aliases: Vec<String>`, `obsoleted_by: Option<String>`, `updated_by: Option<String>`. Each gets `#[serde(deny_unknown_fields)]` and a doc comment per field.

```rust
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

impl Source {
    /// The source's name, when the manifest gives one.
    pub fn name(&self) -> Option<&str> { … }  // Git/Local: name.as_deref(); Document: Some(&name)

    /// How agents may use this source.
    pub fn license_class(&self) -> LicenseClass { … }
}
```

The raw top level, private:

```rust
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
```

Learn — `#[derive(Deserialize)]`: the derive writes the `Deserialize` impl from the struct's shape, so the TOML format (via `toml`) and the type stay independent. `#[serde(default = "path")]` calls a function for a missing key when the type's own `Default` (0 for `u32`) is not the default you want.

- [ ] **Step 4: Write `ManifestError`, `parse`, `load` and `expand_home`**

```rust
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
    Source { source_name: String, message: String },

    /// A `path` starts with `~` but there is no home directory to expand it to.
    #[error("source {source_name}: `path` starts with `~` but the home directory is unknown")]
    NoHome { source_name: String },
}
```

Doc-comment each field (`missing_docs` covers variant fields). Task 2 adds the validation variants.

`parse`:

```rust
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
        Ok(Manifest { name: raw.name, max_age_days: raw.max_age_days, sources })
    }

    /// Reads and parses the manifest at `path`.
    pub fn load(path: &Path) -> crate::Result<Self> {
        Ok(Self::parse(&std::fs::read_to_string(path)?)?)
    }
}
```

`std::env::home_dir` is not deprecated on this toolchain. If clippy or rustc warns that it is, use `std::env::var_os("HOME").map(PathBuf::from)` instead.

`parse_source(index, mut table, home) -> Result<Source, ManifestError>`:
1. `let source_name = label(index, &table);`, where `label` returns `` `name` ``, else `` `url` ``, else `` `path` `` (the first of those keys holding a string), else `#{index + 1}`.
2. `table.entry("kind").or_insert_with(|| "git".into());`
3. `let mut source = Source::deserialize(table).map_err(|e| ManifestError::Source { source_name: source_name.clone(), message: e.to_string().trim_end().to_owned() })?;` (`toml::Table` is a serde `Deserializer`. Its errors end in a newline and have no position, which is why the label matters.)
4. For `Source::Local(local)`: `local.path = expand_home(path_str, home).ok_or(ManifestError::NoHome { source_name })?;`. Deserialize `path` as `PathBuf`; take `local.path.to_str()` for the check, and leave a non-UTF-8 path unchanged.
5. Return the source.

Learn — `collect::<Result<Vec<_>, _>>()`: collecting an iterator of `Result`s into one `Result` stops at the first `Err`. This is the same idiom #15 used.

```rust
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
```

`lib.rs`: `mod manifest;` and `pub use manifest::{DocumentSource, GitSource, LicenseClass, LocalSource, Manifest, ManifestError, Source};`. Update the crate doc to mention that it also reads corpus manifests.

`error.rs`, new variant:

```rust
    /// A corpus manifest is invalid.
    #[error(transparent)]
    Manifest(#[from] crate::ManifestError),
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p codecite-index`
Expected: all pass. If a serde error text differs from the test, check the actual text (`cargo test -- --nocapture`) and fix the test's serde tail only, keeping the `source …:` prefix exact.

- [ ] **Step 6: Gates and commit**

Run the gates. Commit `Cargo.toml`, `Cargo.lock`, `crates/codecite-index/Cargo.toml`, `crates/codecite-index/src/{lib,error,manifest}.rs`:

```
feat(index): parse corpus manifests with serde (#17)
```

---

### Task 2: Validation and the design doc

**Files:**
- Modify: `crates/codecite-index/src/manifest.rs`
- Modify: `docs/design.md` (`## Corpus manifests and licensing`)

**Interfaces:**
- Consumes: Task 1's types and `parse_source`.
- Produces: `ManifestError::{Field, SeveralCurrent, NoCurrent}`; `Manifest::parse` calls `validate`.

- [ ] **Step 1: Write the failing tests**

Add to the test module:

```rust
    fn git(commit: &str) -> String {
        manifest(&format!(
            "[[source]]\nname = \"client\"\nurl = \"u\"\ncommit = \"{commit}\"\nuse = \"adapt\"\n"
        ))
    }

    fn document(name: &str, sha256: &str, current: Option<bool>) -> String {
        let current = current.map(|c| format!("current = {c}\n")).unwrap_or_default();
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
                format!("source `client`: `commit` must be 40 lowercase hex digits, got {commit:?}")
            );
        }
        assert!(Manifest::parse(&git(COMMIT)).is_ok());
    }

    #[test]
    fn bad_sha256_is_an_error() {
        for sha in [&SHA256[..63], &SHA256.to_uppercase(), "sha256:abc"] {
            let err = Manifest::parse(&manifest(&document("rfc9449", sha, Some(true)))).unwrap_err();
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
```

Run: `cargo test -p codecite-index manifest`
Expected: the four error tests fail (`parse` returns `Ok`); `one_current_version_among_several_is_valid` passes.

- [ ] **Step 2: Add the variants and `validate`**

```rust
    /// A field has a value of the right type but the wrong form.
    #[error("source {source_name}: `{field}` must be {expected}, got {value:?}")]
    Field { source_name: String, field: &'static str, expected: &'static str, value: String },

    /// Several versions of one document are marked current.
    #[error("document `{name}`: {count} versions have `current = true`; mark exactly one")]
    SeveralCurrent { name: String, count: usize },

    /// No version of a document is marked current.
    #[error("document `{name}`: no version has `current = true`; mark exactly one")]
    NoCurrent { name: String },
```

In `parse_source`, after deserializing, check the pins (the label is already at hand there):

```rust
fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
```

`Source::Git(g)` with `!is_lower_hex(&g.commit, 40)` → `Field { field: "commit", expected: "40 lowercase hex digits", value: g.commit.clone(), .. }`. `Source::Document(d)` with `!is_lower_hex(&d.sha256, 64)` → `Field { field: "sha256", expected: "64 lowercase hex digits", .. }`.

After all sources parse, in `Manifest::parse`, call `check_current(&sources)?`:

```rust
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
            0 => return Err(ManifestError::NoCurrent { name: name.to_owned() }),
            count => return Err(ManifestError::SeveralCurrent { name: name.to_owned(), count }),
        }
    }
    Ok(())
}
```

Learn — `entry(..).or_default()`: one lookup that inserts the default (0) the first time a key is seen and returns a `&mut` to the count. `usize::from(bool)` is 0 or 1, without an `as` cast.

Run: `cargo test -p codecite-index`
Expected: all pass.

- [ ] **Step 3: Update `docs/design.md`**

In `## Corpus manifests and licensing`:
- The example's git comment `# adapt | approach_only | excluded` → `# adapt | approach_only | quote | excluded`, so it lists all four classes as the table below it does.
- After the `kind = "local"` paragraph, add one paragraph:

> Each source needs `use`; `license` is optional until SPDX detection lands. A git source needs `url` and a 40-character lowercase `commit`; a local source needs `path` (`~` expands to the home directory); a document needs `name`, `url` and a 64-character lowercase `sha256`, and exactly one version of each document name has `current = true`. `name` is optional for git and local sources. Unknown keys are errors, and every error names the source and the field.

- [ ] **Step 4: Gates and commit**

Run the gates. Commit `crates/codecite-index/src/manifest.rs` and `docs/design.md`:

```
feat(index): validate manifest pins and current documents (#17)
```
