//! Indexing a corpus: walk each local source, chunk the files whose content
//! changed and write them into the store, one transaction per document.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use codecite_core::{ChunkRecord, DocumentRecord, SourceRecord, Store, SymbolRecord};
use sha2::{Digest, Sha256};

use crate::{
    Chunker, Error, Fetcher, Language, LicenseClass, LocalSource, LocalTree, Manifest, Result,
    SkipReason, Source, WalkOptions, walk_local,
};

/// What an index run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexSummary {
    /// Documents written: new, or with changed content.
    pub written: usize,
    /// Documents left as they were because their content hash is unchanged.
    pub unchanged: usize,
    /// Documents deleted because their file is gone or now skipped.
    pub removed: usize,
    /// Chunks written, over every written document.
    pub chunks: usize,
    /// Document sources downloaded and checked against their pin.
    pub downloaded: usize,
    /// Document sources read from the cache.
    pub cached: usize,
    /// Files not indexed, counted by reason.
    pub skipped: BTreeMap<SkipReason, usize>,
    /// Manifest sources not indexed, with the reason.
    pub skipped_sources: Vec<(String, &'static str)>,
    /// Sources deleted from the store because the run did not index them.
    pub removed_sources: Vec<String>,
    /// How long the run took.
    pub elapsed: Duration,
}

/// Fetches every document source in `manifest` and indexes every local
/// source into `store`.
///
/// Documents are fetched first, from `fetcher`'s cache when they are there,
/// so a document whose bytes don't match its pinned `sha256` stops the run
/// before anything is written. Documents are not indexed yet.
///
/// A file whose content hash matches the stored one is not rewritten, and a
/// stored file that is gone, or is now skipped, is deleted. Git and document
/// sources are reported in [`IndexSummary::skipped_sources`], as are sources
/// marked `excluded`. Every source the run did not index is then deleted
/// from the store.
pub fn index_corpus(
    store: &mut Store,
    manifest: &Manifest,
    options: &WalkOptions,
    fetcher: &Fetcher,
) -> Result<IndexSummary> {
    let started = Instant::now();
    let mut summary = IndexSummary::default();
    for source in &manifest.sources {
        if let Source::Document(doc) = source
            && doc.license_class != LicenseClass::Excluded
        {
            if fetcher.fetch_pinned(doc)?.from_cache {
                summary.cached += 1;
            } else {
                summary.downloaded += 1;
            }
        }
    }
    let mut chunker = Chunker::new()?;
    let mut indexed: Vec<String> = Vec::new();
    let mut keep: Vec<i64> = Vec::new();
    for source in &manifest.sources {
        let local = match source {
            Source::Local(local) if local.license_class != LicenseClass::Excluded => local,
            Source::Local(local) => {
                let name = local
                    .name
                    .clone()
                    .unwrap_or_else(|| local.path.display().to_string());
                summary.skipped_sources.push((name, "use = \"excluded\""));
                continue;
            }
            Source::Git(git) => {
                let name = git.name.clone().unwrap_or_else(|| git.url.clone());
                summary
                    .skipped_sources
                    .push((name, "git sources are not indexed yet"));
                continue;
            }
            Source::Document(doc) => {
                let reason = if doc.license_class == LicenseClass::Excluded {
                    "use = \"excluded\""
                } else {
                    "document sources are fetched and checked, not indexed yet"
                };
                summary.skipped_sources.push((doc.name.clone(), reason));
                continue;
            }
        };
        let tree = walk_local(local, options)?;
        let name = source_name(local, &tree);
        if indexed.contains(&name) {
            return Err(Error::DuplicateSource { name });
        }
        keep.push(index_local(
            store,
            &mut chunker,
            local,
            &name,
            &tree,
            &mut summary,
        )?);
        indexed.push(name);
    }
    summary.removed_sources = store.delete_sources_except(&keep)?;
    store.set_indexed_at()?;
    summary.elapsed = started.elapsed();
    Ok(summary)
}

/// The manifest's `name`, else the checkout directory's name.
fn source_name(local: &LocalSource, tree: &LocalTree) -> String {
    local.name.clone().unwrap_or_else(|| {
        tree.root
            .file_name()
            .map_or_else(|| "local".to_owned(), |n| n.to_string_lossy().into_owned())
    })
}

/// Writes one walked local source into the store and returns its ID.
fn index_local(
    store: &mut Store,
    chunker: &mut Chunker,
    local: &LocalSource,
    name: &str,
    tree: &LocalTree,
    summary: &mut IndexSummary,
) -> Result<i64> {
    let url = tree.root.to_string_lossy();
    let source_id = store.upsert_source(&SourceRecord {
        name,
        kind: "local",
        url: &url,
        revision: &tree.head,
        license_spdx: local.license.as_deref(),
        license_class: local.license_class.as_str(),
        is_current: true,
        obsoleted_by: None,
        updated_by: None,
    })?;
    for (reason, count) in tree.skip_counts() {
        *summary.skipped.entry(reason).or_default() += count;
    }

    let stored = store.document_hashes(source_id)?;
    let mut kept = HashSet::new();
    for file in &tree.files {
        let bytes = match read(&tree.root, &file.path)? {
            Some(bytes) => bytes,
            None => {
                *summary.skipped.entry(SkipReason::Deleted).or_default() += 1;
                continue;
            }
        };
        let hash = Sha256::digest(&bytes);
        if stored
            .get(&file.path)
            .is_some_and(|old| old[..] == hash[..])
        {
            store.set_dirty(source_id, &file.path, file.dirty)?;
            kept.insert(file.path.as_str());
            summary.unchanged += 1;
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            *summary
                .skipped
                .entry(SkipReason::NonUtf8Content)
                .or_default() += 1;
            continue;
        };
        let chunks = chunker.chunk(&file.path, &text);
        let symbol_ids: Vec<Option<String>> = chunks
            .iter()
            .map(|c| c.symbol.as_ref().map(|s| s.scip_id(name, &file.path)))
            .collect();
        let document = DocumentRecord {
            path: &file.path,
            language: Language::detect(&file.path, &text).map(Language::name),
            content_hash: hash.as_slice(),
            size_bytes: text.len() as u64,
            dirty: file.dirty,
            chunks: chunks
                .iter()
                .zip(symbol_ids)
                .map(|(c, id)| ChunkRecord {
                    kind: c.kind.as_str(),
                    start_line: c.start_line,
                    end_line: c.end_line,
                    text: c.text,
                    number: None,
                    anchor: None,
                    title: c.title.as_deref(),
                    parent: None,
                    strength: &[],
                    symbol: c.symbol.as_ref().zip(id).map(|(s, id)| SymbolRecord {
                        id,
                        display_name: &s.name,
                        kind: s.kind.as_str(),
                    }),
                })
                .collect(),
        };
        summary.chunks += store.write_document(source_id, &document)?;
        summary.written += 1;
        kept.insert(file.path.as_str());
    }

    for path in stored.keys().filter(|p| !kept.contains(p.as_str())) {
        store.delete_document(source_id, path)?;
        summary.removed += 1;
    }
    Ok(source_id)
}

/// Reads a walked file, or `None` when it was deleted after the walk.
fn read(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    match std::fs::read(root.join(path)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::ReadFile {
            path: root.join(path),
            source,
        }),
    }
}

impl fmt::Display for IndexSummary {
    /// The summary `codecite index` prints, one fact per line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "documents: {} written, {} unchanged, {} removed",
            self.written, self.unchanged, self.removed
        )?;
        writeln!(f, "chunks: {} written", self.chunks)?;
        if self.downloaded + self.cached > 0 {
            writeln!(
                f,
                "fetched: {} downloaded, {} from the cache",
                self.downloaded, self.cached
            )?;
        }
        if !self.skipped.is_empty() {
            let parts: Vec<String> = self
                .skipped
                .iter()
                .map(|(reason, count)| format!("{count} {}", reason.label()))
                .collect();
            writeln!(f, "skipped: {}", parts.join(", "))?;
        }
        for (name, reason) in &self.skipped_sources {
            writeln!(f, "source {name} not indexed: {reason}")?;
        }
        for name in &self.removed_sources {
            writeln!(f, "source {name} removed: the run no longer indexes it")?;
        }
        write!(f, "elapsed: {:.2}s", self.elapsed.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentSource;
    use crate::test_repo::Repo;
    use crate::test_server::Server;
    use rusqlite::Connection;
    use std::path::PathBuf;

    fn local(repo: &Repo, name: Option<&str>) -> Source {
        Source::Local(LocalSource {
            name: name.map(str::to_owned),
            path: repo.path().to_owned(),
            license: Some("MIT".into()),
            license_class: LicenseClass::Adapt,
            include: Vec::new(),
            exclude: Vec::new(),
        })
    }

    fn document(url: String, sha256: &str) -> Source {
        Source::Document(DocumentSource {
            name: "rfc9449".into(),
            url,
            sha256: sha256.into(),
            license: None,
            license_class: LicenseClass::Quote,
            current: true,
            aliases: Vec::new(),
            obsoleted_by: None,
            updated_by: None,
        })
    }

    const SPEC: &str = "<h1>RFC 9449</h1>\n";
    /// The sha256 of [`SPEC`].
    const SPEC_SHA256: &str = "adcd06b613eb4e660e403fbec3d11011983dec4501489279e191b728e2fe6b2b";

    fn manifest(sources: Vec<Source>) -> Manifest {
        Manifest {
            name: "test".into(),
            max_age_days: 30,
            sources,
        }
    }

    /// A checkout with a Rust file, a Markdown file and a text file.
    fn fixture() -> Repo {
        let repo = Repo::new();
        repo.write(
            "src/lib.rs",
            "pub fn validate_token() {}\n\npub struct Client;\n",
        );
        repo.write("README.md", "# App\n\nValidates the token.\n");
        repo.write("notes.txt", "refresh sessions daily\n");
        repo.commit_all(&["src/lib.rs", "README.md", "notes.txt"]);
        repo
    }

    /// The corpus file for one test, deleted with the directory.
    struct Corpus {
        dir: tempfile::TempDir,
    }

    impl Corpus {
        fn new() -> Self {
            Corpus {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self) -> PathBuf {
            self.dir.path().join("corpus.db")
        }

        fn index(&self, manifest: &Manifest) -> IndexSummary {
            self.try_index(manifest).unwrap()
        }

        fn try_index(&self, manifest: &Manifest) -> Result<IndexSummary> {
            let mut store = Store::open_read_write(&self.path()).unwrap();
            let fetcher = Fetcher::new(self.dir.path().join("cache"));
            index_corpus(&mut store, manifest, &WalkOptions::default(), &fetcher)
        }

        fn sql(&self) -> Connection {
            Connection::open(self.path()).unwrap()
        }

        /// Each document's chunk IDs, by path.
        fn chunk_ids(&self) -> BTreeMap<String, Vec<i64>> {
            let conn = self.sql();
            let mut stmt = conn
                .prepare(
                    "SELECT d.path, c.id FROM chunk c JOIN document d ON d.id = c.document_id
                     ORDER BY c.id",
                )
                .unwrap();
            let mut ids: BTreeMap<String, Vec<i64>> = BTreeMap::new();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap();
            for row in rows {
                let (path, id) = row.unwrap();
                ids.entry(path).or_default().push(id);
            }
            ids
        }

        fn matches(&self, query: &str) -> i64 {
            self.sql()
                .query_row(
                    "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH ?1",
                    [query],
                    |row| row.get(0),
                )
                .unwrap()
        }
    }

    #[test]
    fn indexes_a_local_source() {
        let repo = fixture();
        let corpus = Corpus::new();
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (3, 0, 0)
        );
        assert_eq!(summary.chunks, 4);
        let ids = corpus.chunk_ids();
        assert_eq!(
            ids.keys().collect::<Vec<_>>(),
            ["README.md", "notes.txt", "src/lib.rs"]
        );
        assert_eq!(corpus.matches("symbol:validate"), 1);
        assert_eq!(corpus.matches("token"), 2);
        let (name, kind, revision, class): (String, String, String, String) = corpus
            .sql()
            .query_row(
                "SELECT name, kind, revision, license_class FROM source",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), kind.as_str(), class.as_str()),
            ("app", "local", "adapt")
        );
        assert_eq!(revision.len(), 40);
        let symbol: String = corpus
            .sql()
            .query_row(
                "SELECT id FROM symbol WHERE display_name = 'Client'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(symbol, "codecite . app . src/`lib.rs`/Client#");
    }

    #[test]
    fn reindexing_rewrites_only_the_changed_document() {
        let repo = fixture();
        let corpus = Corpus::new();
        let manifest = manifest(vec![local(&repo, Some("app"))]);
        corpus.index(&manifest);
        let before = corpus.chunk_ids();

        repo.write("src/lib.rs", "pub fn refresh_session() {}\n");
        let summary = corpus.index(&manifest);

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (1, 2, 0)
        );
        assert_eq!(summary.chunks, 1);
        let after = corpus.chunk_ids();
        assert_eq!(after["README.md"], before["README.md"]);
        assert_eq!(after["notes.txt"], before["notes.txt"]);
        // SQLite may reuse the freed chunk IDs, so check the content instead.
        assert_eq!(after["src/lib.rs"].len(), 1);
        assert_eq!(corpus.matches("symbol:validate"), 0);
        assert_eq!(corpus.matches("symbol:refresh"), 1);
        let dirty: Vec<(String, bool)> = {
            let conn = corpus.sql();
            let mut stmt = conn
                .prepare("SELECT path, dirty FROM document ORDER BY path")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            dirty,
            [
                ("README.md".to_owned(), false),
                ("notes.txt".to_owned(), false),
                ("src/lib.rs".to_owned(), true)
            ]
        );
    }

    #[test]
    fn a_deleted_file_loses_its_chunks() {
        let repo = fixture();
        let corpus = Corpus::new();
        let manifest = manifest(vec![local(&repo, Some("app"))]);
        corpus.index(&manifest);
        assert_eq!(corpus.matches("refresh"), 1);

        std::fs::remove_file(repo.path().join("notes.txt")).unwrap();
        let summary = corpus.index(&manifest);

        assert_eq!(
            (summary.written, summary.unchanged, summary.removed),
            (0, 2, 1)
        );
        assert_eq!(summary.skipped[&SkipReason::Deleted], 1);
        assert!(!corpus.chunk_ids().contains_key("notes.txt"));
        assert_eq!(corpus.matches("refresh"), 0);
    }

    #[test]
    fn a_file_that_becomes_skipped_is_removed() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        repo.write("notes.txt", b"binary\0now\n");
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));
        assert_eq!(summary.removed, 1);
        assert_eq!(summary.skipped[&SkipReason::Binary], 1);

        repo.write("notes.txt", b"caf\xe9\n");
        repo.write("README.md", "# App\n");
        let summary = corpus.index(&manifest(vec![local(&repo, Some("app"))]));
        assert_eq!(summary.skipped[&SkipReason::NonUtf8Content], 1);
        assert_eq!((summary.written, summary.removed), (1, 0));
        assert_eq!(
            corpus.chunk_ids().keys().collect::<Vec<_>>(),
            ["README.md", "src/lib.rs"]
        );
    }

    #[test]
    fn sources_the_run_does_not_index_are_removed() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, Some("app"))]));

        let mut excluded = local(&repo, Some("app"));
        if let Source::Local(l) = &mut excluded {
            l.license_class = LicenseClass::Excluded;
        }
        let summary = corpus.index(&manifest(vec![excluded]));
        assert_eq!(summary.removed_sources, ["app"]);
        assert_eq!(
            summary.skipped_sources,
            [("app".to_owned(), "use = \"excluded\"")]
        );
        assert!(corpus.chunk_ids().is_empty());
        assert_eq!(corpus.matches("token"), 0);
    }

    #[test]
    fn the_name_defaults_to_the_directory_and_must_be_unique() {
        let repo = fixture();
        let corpus = Corpus::new();
        corpus.index(&manifest(vec![local(&repo, None)]));
        let dir = repo.path().canonicalize().unwrap();
        let name: String = corpus
            .sql()
            .query_row("SELECT name FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, dir.file_name().unwrap().to_str().unwrap());

        let twice = manifest(vec![local(&repo, Some("app")), local(&repo, Some("app"))]);
        let err = corpus.try_index(&twice).unwrap_err();
        assert!(
            matches!(&err, Error::DuplicateSource { name } if name == "app"),
            "{err:?}"
        );
    }

    #[test]
    fn documents_are_fetched_once_and_not_indexed_yet() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let corpus = Corpus::new();
        let manifest = manifest(vec![document(server.url("/rfc9449.html"), SPEC_SHA256)]);

        let first = corpus.index(&manifest);
        assert_eq!((first.downloaded, first.cached), (1, 0));
        assert_eq!(
            first.skipped_sources,
            [(
                "rfc9449".to_owned(),
                "document sources are fetched and checked, not indexed yet"
            )]
        );
        let second = corpus.index(&manifest);
        assert_eq!((second.downloaded, second.cached), (0, 1));
        assert_eq!(server.requests(), 1);
    }

    #[test]
    fn a_changed_document_stops_the_run_before_any_write() {
        let repo = fixture();
        let server = Server::start();
        server.set("/rfc9449.html", "<h1>RFC 9449, edited</h1>\n");
        let corpus = Corpus::new();
        let manifest = manifest(vec![
            local(&repo, Some("app")),
            document(server.url("/rfc9449.html"), SPEC_SHA256),
        ]);

        let err = corpus.try_index(&manifest).unwrap_err();
        assert!(
            matches!(&err, Error::Fetch(crate::FetchError::Mismatch { name, .. }) if name == "rfc9449"),
            "{err:?}"
        );
        let sources: i64 = corpus
            .sql()
            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sources, 0);
    }

    #[test]
    fn excluded_documents_are_not_fetched() {
        let server = Server::start();
        let corpus = Corpus::new();
        let mut doc = document(server.url("/rfc9449.html"), SPEC_SHA256);
        if let Source::Document(d) = &mut doc {
            d.license_class = LicenseClass::Excluded;
        }
        let summary = corpus.index(&manifest(vec![doc]));
        assert_eq!(server.requests(), 0);
        assert_eq!(
            summary.skipped_sources,
            [("rfc9449".to_owned(), "use = \"excluded\"")]
        );
    }

    #[test]
    fn summary_prints_each_count() {
        let summary = IndexSummary {
            written: 2,
            unchanged: 5,
            removed: 1,
            chunks: 9,
            downloaded: 1,
            cached: 2,
            skipped: BTreeMap::from([(SkipReason::Ignored, 3), (SkipReason::Binary, 1)]),
            skipped_sources: vec![("rfc9449".into(), "document sources are not indexed yet")],
            removed_sources: vec!["old".into()],
            elapsed: Duration::from_millis(1250),
        };
        assert_eq!(
            summary.to_string(),
            "documents: 2 written, 5 unchanged, 1 removed\n\
             chunks: 9 written\n\
             fetched: 1 downloaded, 2 from the cache\n\
             skipped: 3 ignored, 1 binary\n\
             source rfc9449 not indexed: document sources are not indexed yet\n\
             source old removed: the run no longer indexes it\n\
             elapsed: 1.25s"
        );
    }
}
