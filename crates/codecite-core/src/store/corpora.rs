//! Listing the corpora in a data directory, with how fresh each one is:
//! how old the index is, and whether its manifest still pins what it holds.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::Serialize;

use super::{Store, known_corpora};
use crate::Result;

/// One revision a manifest pins: a git source's commit, a document
/// version's or a pinned guidance file's sha256, under the name the
/// corpus stores the source by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestPin {
    /// The source's name, as results cite it.
    pub source: String,
    /// The pinned commit or sha256.
    pub revision: String,
}

/// Reads the pins of the manifest at a path, or `None` when it can't be
/// read. `codecite_index::manifest_pins` is the one codecite uses; the
/// manifest format lives in that crate.
pub type PinReader = fn(&Path) -> Option<Vec<ManifestPin>>;

/// A pin the corpus doesn't hold: the manifest was changed after the last
/// index run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct StalePin {
    /// The source's name.
    pub source: String,
    /// The revision the manifest pins.
    pub pinned: String,
    /// The revisions the corpus holds under that name, sorted; empty when
    /// it holds none.
    pub stored: Vec<String>,
}

/// The corpora in a data directory: what `codecite corpora` prints and the
/// MCP `corpora` tool returns, as the same JSON.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Corpora {
    /// One entry per corpus file, sorted by name.
    pub corpora: Vec<CorpusInfo>,
}

/// One corpus: its size and, when its file can be read, its freshness and
/// how many sources it holds. One that can't be read has an `error`
/// instead, so a broken file never hides the others.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CorpusInfo {
    /// The corpus's name, which `search` and `read` take.
    pub corpus: String,
    /// The corpus file's size in bytes.
    pub size_bytes: u64,
    /// When the last index run finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<String>,
    /// Days since that run, to two decimal places.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_days: Option<f64>,
    /// The manifest's `max_age_days` at that run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    /// Whether the index is older than its maximum age, so that queries
    /// refuse it without `allow_stale`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
    /// Whether the manifest the corpus was last indexed from pins a
    /// revision the corpus doesn't hold, so it needs indexing again. Left
    /// out when that manifest can't be read. Queries still answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pins_stale: Option<bool>,
    /// The pins the corpus doesn't hold, when there are any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_pins: Option<Vec<StalePin>>,
    /// Why the corpus is read with a caveat: a schema older than this
    /// build's, which queries can still read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// How many sources it holds; each pinned version of a document counts
    /// as one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<u64>,
    /// Why the corpus file can't be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The local source root that contains the path asked for. Set only by
    /// [`corpora_for`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_root: Option<String>,
    /// Every distinct source name the corpus holds, sorted. Set only by
    /// [`corpora_for`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_names: Option<Vec<String>>,
}

/// Lists every corpus in `dir`, sorted by name. A missing directory holds
/// none. With `pins`, each corpus's pins are read from the manifest it was
/// last indexed from and checked against it.
pub fn corpora(dir: &Path, pins: Option<PinReader>) -> Result<Corpora> {
    let mut corpora = Vec::new();
    for corpus in known_corpora(dir)? {
        corpora.push(describe(dir, corpus, pins)?);
    }
    Ok(Corpora { corpora })
}

/// The corpora in `dir` with a `local` source whose root contains `path`,
/// sorted by name, in the shape [`corpora`] returns. Each carries the
/// matched `local_root` and its `source_names`. `path` need not exist: its
/// longest existing ancestor is resolved through symlinks. A corpus that
/// can't be opened is left out.
pub fn corpora_for(dir: &Path, path: &Path, pins: Option<PinReader>) -> Result<Corpora> {
    let path = resolve(path);
    let mut found = Vec::new();
    for corpus in known_corpora(dir)? {
        let Ok(store) = Store::open_read_only(&dir.join(format!("{corpus}.db"))) else {
            continue;
        };
        let Ok(roots) = store.local_roots() else {
            continue;
        };
        let Some(root) = roots
            .into_iter()
            .find(|root| path.starts_with(resolve(Path::new(root))))
        else {
            continue;
        };
        let Ok(source_names) = store.all_source_names() else {
            continue;
        };
        drop(store);
        let mut info = describe(dir, corpus, pins)?;
        info.local_root = Some(root);
        info.source_names = Some(source_names);
        found.push(info);
    }
    Ok(Corpora { corpora: found })
}

/// `path` made absolute and resolved through symlinks as far as it exists:
/// its longest existing ancestor is canonicalized and the rest re-appended.
/// A path that can't be resolved at all is returned as given.
fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut rest = Vec::new();
    let mut ancestor = absolute.as_path();
    loop {
        if let Ok(mut resolved) = ancestor.canonicalize() {
            resolved.extend(rest.iter().rev());
            return resolved;
        }
        match (ancestor.parent(), ancestor.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                ancestor = parent;
            }
            _ => return absolute,
        }
    }
}

/// The info for the corpus `corpus` in `dir`, as [`corpora`] lists it.
fn describe(dir: &Path, corpus: String, pins: Option<PinReader>) -> Result<CorpusInfo> {
    let path = dir.join(format!("{corpus}.db"));
    let size_bytes = std::fs::metadata(&path)?.len();
    let mut info = CorpusInfo {
        corpus,
        size_bytes,
        indexed_at: None,
        age_days: None,
        max_age_days: None,
        stale: None,
        pins_stale: None,
        stale_pins: None,
        warning: None,
        sources: None,
        error: None,
        local_root: None,
        source_names: None,
    };
    let described = Store::open_read_only(&path).and_then(|store| {
        let stale_pins = match pins {
            Some(read) => store.stale_pins_of_manifest(read)?,
            None => None,
        };
        Ok((
            store.freshness()?,
            store.source_count()?,
            stale_pins,
            store.schema_warning().map(str::to_owned),
        ))
    });
    match described {
        Ok((freshness, sources, stale_pins, warning)) => {
            info.stale = Some(freshness.is_stale());
            info.age_days = Some((freshness.age_days * 100.0).round() / 100.0);
            info.max_age_days = Some(freshness.max_age_days);
            info.indexed_at = Some(freshness.indexed_at);
            info.sources = Some(sources);
            info.pins_stale = stale_pins.as_ref().map(|s| !s.is_empty());
            info.stale_pins = stale_pins.filter(|s| !s.is_empty());
            info.warning = warning;
        }
        Err(err) => info.error = Some(with_causes(&err)),
    }
    Ok(info)
}

/// `err`'s message followed by each of its causes', so that a SQLite or
/// I/O error says what went wrong and not only where.
fn with_causes(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut cause = err.source();
    while let Some(err) = cause {
        text.push_str(": ");
        text.push_str(&err.to_string());
        cause = err.source();
    }
    text
}

impl Store {
    /// Records the manifest an index run read, as an absolute path, so the
    /// freshness check can read its pins later without being told where it
    /// is. A path that isn't UTF-8 can't be stored as text exactly, so it
    /// clears the record instead, and the pins go unchecked.
    pub fn set_manifest_path(&mut self, path: &Path) -> Result<()> {
        match path.to_str() {
            Some(path) => self.conn.execute(
                "INSERT INTO meta (key, value) VALUES ('manifest_path', ?1)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [path],
            )?,
            None => self
                .conn
                .execute("DELETE FROM meta WHERE key = 'manifest_path'", [])?,
        };
        Ok(())
    }

    /// The manifest the last index run read, when it recorded one.
    pub fn manifest_path(&self) -> Result<Option<PathBuf>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'manifest_path'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.map(PathBuf::from))
    }

    /// The pins the corpus doesn't hold: for each, no stored source of its
    /// name has its revision. In the order given.
    pub fn stale_pins(&self, pins: &[ManifestPin]) -> Result<Vec<StalePin>> {
        let mut stmt = self
            .conn
            .prepare("SELECT revision FROM source WHERE name = ?1 ORDER BY revision")?;
        let mut stale = Vec::new();
        for pin in pins {
            let stored: Vec<String> = stmt
                .query_map([&pin.source], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            if !stored.contains(&pin.revision) {
                stale.push(StalePin {
                    source: pin.source.clone(),
                    pinned: pin.revision.clone(),
                    stored,
                });
            }
        }
        Ok(stale)
    }

    /// [`Store::stale_pins`] for the pins `read` finds in the manifest the
    /// corpus was last indexed from; `None` when there is no such manifest
    /// recorded, or it can't be read.
    fn stale_pins_of_manifest(&self, read: PinReader) -> Result<Option<Vec<StalePin>>> {
        let Some(pins) = self.manifest_path()?.and_then(|path| read(&path)) else {
            return Ok(None);
        };
        self.stale_pins(&pins).map(Some)
    }

    /// The roots of the corpus's `local` sources, as stored: the index
    /// records a local source's canonical root as its `url`.
    pub fn local_roots(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT url FROM source WHERE kind = 'local'")?;
        let roots = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(roots)
    }

    /// Every distinct source name the corpus holds, sorted.
    pub fn all_source_names(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT name FROM source ORDER BY name")?;
        let names = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(names)
    }

    /// How many sources the corpus holds; each pinned version of a
    /// document counts as one.
    pub fn source_count(&self) -> Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceRecord;
    use std::fs;

    fn source(name: &str) -> SourceRecord<'_> {
        SourceRecord {
            name,
            kind: "local",
            url: "/src/app",
            revision: "0123456789abcdef0123456789abcdef01234567",
            license_spdx: Some("MIT"),
            license_class: "adapt",
            is_current: true,
            obsoleted_by: None,
            updated_by: None,
        }
    }

    /// A corpus `name` in `dir` holding `sources` sources, indexed now.
    fn indexed(dir: &Path, name: &str, sources: &[&str]) {
        let mut store = Store::open_read_write(&dir.join(format!("{name}.db"))).unwrap();
        for s in sources {
            store.upsert_source(&source(s)).unwrap();
        }
        store.set_indexed_at(30).unwrap();
    }

    #[test]
    fn a_missing_directory_holds_no_corpora() {
        let dir = tempfile::tempdir().unwrap();
        let listed = corpora(&dir.path().join("nowhere"), None).unwrap();
        assert!(listed.corpora.is_empty());
        let json = serde_json::to_string(&listed).unwrap();
        assert_eq!(json, r#"{"corpora":[]}"#);
    }

    #[test]
    fn each_corpus_has_its_age_size_sources_and_staleness() {
        let dir = tempfile::tempdir().unwrap();
        indexed(dir.path(), "specs", &["rfc9449"]);
        indexed(dir.path(), "app", &["app", "lib"]);
        // Indexed 40 days ago, past its maximum age of 30.
        indexed(dir.path(), "old", &["app"]);
        rusqlite::Connection::open(dir.path().join("old.db"))
            .unwrap()
            .execute(
                "UPDATE meta SET value = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-40 days')
                 WHERE key = 'indexed_at'",
                [],
            )
            .unwrap();
        // Not corpora: no `.db` extension, or a name that isn't a corpus's.
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        fs::write(dir.path().join(".hidden.db"), "x").unwrap();

        let listed = corpora(dir.path(), None).unwrap().corpora;
        let names: Vec<&str> = listed.iter().map(|c| c.corpus.as_str()).collect();
        assert_eq!(names, ["app", "old", "specs"]);

        let app = &listed[0];
        assert_eq!(app.sources, Some(2));
        assert_eq!(app.stale, Some(false));
        assert_eq!(app.age_days, Some(0.0));
        assert_eq!(app.max_age_days, Some(30));
        assert!(app.indexed_at.as_deref().unwrap().ends_with('Z'));
        assert_eq!(
            app.size_bytes,
            fs::metadata(dir.path().join("app.db")).unwrap().len()
        );
        assert!(app.size_bytes > 0);
        assert_eq!(app.error, None);

        let old = &listed[1];
        assert_eq!(old.stale, Some(true));
        assert_eq!(old.age_days, Some(40.0));
        assert_eq!(listed[2].sources, Some(1));
    }

    #[test]
    fn a_corpus_that_cant_be_read_has_an_error_and_no_freshness() {
        let dir = tempfile::tempdir().unwrap();
        indexed(dir.path(), "good", &["app"]);
        fs::write(dir.path().join("junk.db"), "not a database at all").unwrap();
        // Created, but its first index run never finished.
        drop(Store::open_read_write(&dir.path().join("partial.db")).unwrap());

        let listed = corpora(dir.path(), None).unwrap().corpora;
        let names: Vec<&str> = listed.iter().map(|c| c.corpus.as_str()).collect();
        assert_eq!(names, ["good", "junk", "partial"]);
        assert_eq!(listed[0].error, None);
        let junk = listed[1].error.as_deref().unwrap();
        assert!(
            junk.starts_with("SQLite error: file is not a database"),
            "{junk}"
        );
        for broken in &listed[1..] {
            assert!(broken.error.is_some(), "{broken:?}");
            assert_eq!(
                (broken.indexed_at.as_ref(), broken.stale, broken.sources),
                (None, None, None)
            );
        }
        assert!(
            listed[2]
                .error
                .as_deref()
                .unwrap()
                .contains("never fully indexed"),
            "{:?}",
            listed[2].error
        );
    }

    #[test]
    fn the_json_leaves_out_fields_with_no_value() {
        let info = CorpusInfo {
            corpus: "junk".into(),
            size_bytes: 21,
            indexed_at: None,
            age_days: None,
            max_age_days: None,
            stale: None,
            pins_stale: None,
            stale_pins: None,
            warning: None,
            sources: None,
            error: Some("invalid corpus".into()),
            local_root: None,
            source_names: None,
        };
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"corpus":"junk","size_bytes":21,"error":"invalid corpus"}"#
        );
        let info = CorpusInfo {
            corpus: "app".into(),
            size_bytes: 4096,
            indexed_at: Some("2026-10-20T06:00:00Z".into()),
            age_days: Some(1.25),
            max_age_days: Some(30),
            stale: Some(false),
            pins_stale: None,
            stale_pins: None,
            warning: None,
            sources: Some(2),
            error: None,
            local_root: None,
            source_names: None,
        };
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"corpus":"app","size_bytes":4096,"indexed_at":"2026-10-20T06:00:00Z","age_days":1.25,"max_age_days":30,"stale":false,"sources":2}"#
        );
    }

    /// Pins for the tests' manifests: a manifest here is a file of
    /// `<source> <revision>` lines.
    fn read_pins(path: &Path) -> Option<Vec<ManifestPin>> {
        let text = fs::read_to_string(path).ok()?;
        Some(
            text.lines()
                .filter_map(|line| line.split_once(' '))
                .map(|(source, revision)| ManifestPin {
                    source: source.into(),
                    revision: revision.into(),
                })
                .collect(),
        )
    }

    #[test]
    fn a_pin_the_corpus_does_not_hold_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("app.db")).unwrap();
        store.upsert_source(&source("app")).unwrap();
        for revision in ["aaa", "bbb"] {
            store
                .upsert_source(&SourceRecord {
                    kind: "document",
                    revision,
                    ..source("rfc9449")
                })
                .unwrap();
        }
        let pin = |source: &str, revision: &str| ManifestPin {
            source: source.into(),
            revision: revision.into(),
        };
        let stale = store
            .stale_pins(&[
                pin("app", "0123456789abcdef0123456789abcdef01234567"),
                pin("app", "fedcba9876543210fedcba9876543210fedcba98"),
                pin("rfc9449", "bbb"),
                pin("rfc9449", "ccc"),
                pin("new", "ddd"),
            ])
            .unwrap();
        assert_eq!(
            stale,
            [
                StalePin {
                    source: "app".into(),
                    pinned: "fedcba9876543210fedcba9876543210fedcba98".into(),
                    stored: vec!["0123456789abcdef0123456789abcdef01234567".into()],
                },
                StalePin {
                    source: "rfc9449".into(),
                    pinned: "ccc".into(),
                    stored: vec!["aaa".into(), "bbb".into()],
                },
                StalePin {
                    source: "new".into(),
                    pinned: "ddd".into(),
                    stored: vec![],
                },
            ]
        );
    }

    #[test]
    fn corpora_check_the_pins_of_the_manifest_last_indexed_from() {
        let dir = tempfile::tempdir().unwrap();
        indexed(dir.path(), "app", &["app"]);
        indexed(dir.path(), "moved", &["app"]);
        indexed(dir.path(), "unrecorded", &["app"]);
        let held = "app 0123456789abcdef0123456789abcdef01234567\n";
        let manifest = dir.path().join("app.pins");
        fs::write(&manifest, held).unwrap();
        let mut app = Store::open_read_write(&dir.path().join("app.db")).unwrap();
        app.set_manifest_path(&manifest).unwrap();
        drop(app);
        let mut moved = Store::open_read_write(&dir.path().join("moved.db")).unwrap();
        moved
            .set_manifest_path(&dir.path().join("gone.pins"))
            .unwrap();
        drop(moved);

        let listed = corpora(dir.path(), Some(read_pins)).unwrap().corpora;
        assert_eq!(listed[0].pins_stale, Some(false));
        assert_eq!(listed[0].stale_pins, None);
        // A manifest that moved, or was never recorded, says nothing.
        for unknown in &listed[1..] {
            assert_eq!((unknown.pins_stale, &unknown.stale_pins), (None, &None));
            assert_eq!(unknown.error, None);
        }
        // Without a reader, pins aren't checked.
        assert_eq!(
            corpora(dir.path(), None).unwrap().corpora[0].pins_stale,
            None
        );

        // The manifest moves on to a commit not indexed yet.
        fs::write(&manifest, "app fedcba9876543210fedcba9876543210fedcba98\n").unwrap();
        let app = &corpora(dir.path(), Some(read_pins)).unwrap().corpora[0];
        assert_eq!(app.pins_stale, Some(true));
        assert_eq!(app.stale, Some(false));
        let json = serde_json::to_value(app).unwrap();
        assert_eq!(
            json["stale_pins"],
            serde_json::json!([{
                "source": "app",
                "pinned": "fedcba9876543210fedcba9876543210fedcba98",
                "stored": ["0123456789abcdef0123456789abcdef01234567"],
            }])
        );
    }

    #[test]
    fn a_corpus_from_before_the_scip_layer_reports_an_error() {
        // One version back lacks migration 14, which isn't read-compatible,
        // so `corpora` reports the file as unreadable rather than warning.
        let dir = tempfile::tempdir().unwrap();
        indexed(dir.path(), "app", &["app"]);
        rusqlite::Connection::open(dir.path().join("app.db"))
            .unwrap()
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                [(crate::SCHEMA_VERSION - 1).to_string()],
            )
            .unwrap();
        let app = &corpora(dir.path(), None).unwrap().corpora[0];
        assert_eq!(app.warning, None);
        assert!(
            app.error
                .as_deref()
                .is_some_and(|e| e.contains("re-run codecite index")),
            "{app:?}"
        );
    }

    fn names(found: &Corpora) -> Vec<&str> {
        found.corpora.iter().map(|c| c.corpus.as_str()).collect()
    }

    fn canon(path: PathBuf) -> String {
        path.canonicalize().unwrap().to_str().unwrap().to_owned()
    }

    /// A corpus `name` in `data` with one local source rooted at `root`.
    fn indexed_at_root(data: &Path, name: &str, root: &Path) {
        let mut store = Store::open_read_write(&data.join(format!("{name}.db"))).unwrap();
        store
            .upsert_source(&SourceRecord {
                url: &canon(root.to_path_buf()),
                ..source(name)
            })
            .unwrap();
        store.set_indexed_at(30).unwrap();
    }

    /// `tmp/data` plus corpora `app` and `application` rooted at
    /// `tmp/app` and `tmp/application`.
    fn two_roots() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let (data, root) = (tmp.path().join("data"), tmp.path().to_path_buf());
        fs::create_dir_all(&data).unwrap();
        for name in ["app", "application"] {
            fs::create_dir_all(root.join(name).join("src")).unwrap();
            indexed_at_root(&data, name, &root.join(name));
        }
        (tmp, root, data)
    }

    #[test]
    fn corpora_for_matches_a_subdirectory_through_a_symlink() {
        let (_tmp, tmp, data) = two_roots();
        std::os::unix::fs::symlink(tmp.join("app"), tmp.join("link")).unwrap();
        // The file need not exist.
        let found = corpora_for(&data, &tmp.join("link/src/x.rs"), None).unwrap();
        assert_eq!(names(&found), ["app"]);
        assert_eq!(
            found.corpora[0].local_root.as_deref(),
            Some(canon(tmp.join("app")).as_str())
        );
        assert_eq!(
            found.corpora[0].source_names.as_deref(),
            Some(&["app".to_owned()][..])
        );
        assert_eq!(found.corpora[0].sources, Some(1));
        // The root itself matches too.
        let found = corpora_for(&data, &tmp.join("app"), None).unwrap();
        assert_eq!(names(&found), ["app"]);
    }

    #[test]
    fn corpora_for_needs_a_whole_path_component() {
        let (_tmp, tmp, data) = two_roots();
        let found = corpora_for(&data, &tmp.join("application/lib.rs"), None).unwrap();
        assert_eq!(names(&found), ["application"]);
        assert!(
            corpora_for(&data, &tmp.join("ap"), None)
                .unwrap()
                .corpora
                .is_empty()
        );
    }

    #[test]
    fn corpora_for_compares_a_vanished_root_as_stored() {
        let (_tmp, tmp, data) = two_roots();
        let root = tmp.join("app");
        let stored = canon(root.clone());
        fs::remove_dir_all(&root).unwrap();
        let found = corpora_for(&data, &Path::new(&stored).join("src/x.rs"), None).unwrap();
        assert_eq!(names(&found), ["app"]);
    }

    #[test]
    fn corpora_for_skips_unreadable_corpora() {
        let (_tmp, tmp, data) = two_roots();
        fs::write(data.join("broken.db"), "not a database at all").unwrap();
        let found = corpora_for(&data, &tmp.join("app/src"), None).unwrap();
        assert_eq!(names(&found), ["app"]);
        // A missing data directory holds none.
        assert!(
            corpora_for(&tmp.join("nowhere"), &tmp, None)
                .unwrap()
                .corpora
                .is_empty()
        );
    }

    #[test]
    fn corpora_leaves_the_for_fields_out() {
        let (_tmp, _root, data) = two_roots();
        let all = serde_json::to_value(corpora(&data, None).unwrap()).unwrap();
        assert!(all["corpora"][0].get("local_root").is_none());
        assert!(all["corpora"][0].get("source_names").is_none());
    }
}
