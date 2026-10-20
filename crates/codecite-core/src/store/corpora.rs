//! Listing the corpora in a data directory, with how fresh each one is.

use std::path::Path;

use schemars::JsonSchema;
use serde::Serialize;

use super::{Store, known_corpora};
use crate::Result;

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
    /// How many sources it holds; each pinned version of a document counts
    /// as one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<u64>,
    /// Why the corpus file can't be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Lists every corpus in `dir`, sorted by name. A missing directory holds
/// none.
pub fn corpora(dir: &Path) -> Result<Corpora> {
    let mut corpora = Vec::new();
    for corpus in known_corpora(dir)? {
        let path = dir.join(format!("{corpus}.db"));
        let size_bytes = std::fs::metadata(&path)?.len();
        let mut info = CorpusInfo {
            corpus,
            size_bytes,
            indexed_at: None,
            age_days: None,
            max_age_days: None,
            stale: None,
            sources: None,
            error: None,
        };
        let described = Store::open_read_only(&path)
            .and_then(|store| Ok((store.freshness()?, store.source_count()?)));
        match described {
            Ok((freshness, sources)) => {
                info.stale = Some(freshness.is_stale());
                info.age_days = Some((freshness.age_days * 100.0).round() / 100.0);
                info.max_age_days = Some(freshness.max_age_days);
                info.indexed_at = Some(freshness.indexed_at);
                info.sources = Some(sources);
            }
            Err(err) => info.error = Some(with_causes(&err)),
        }
        corpora.push(info);
    }
    Ok(Corpora { corpora })
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
        let listed = corpora(&dir.path().join("nowhere")).unwrap();
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

        let listed = corpora(dir.path()).unwrap().corpora;
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

        let listed = corpora(dir.path()).unwrap().corpora;
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
            sources: None,
            error: Some("invalid corpus".into()),
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
            sources: Some(2),
            error: None,
        };
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"corpus":"app","size_bytes":4096,"indexed_at":"2026-10-20T06:00:00Z","age_days":1.25,"max_age_days":30,"stale":false,"sources":2}"#
        );
    }
}
