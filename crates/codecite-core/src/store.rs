//! The per-corpus SQLite file: opening it, and bringing it to the latest
//! schema with numbered migrations.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::{Error, Result};

mod corpora;
pub use corpora::{Corpora, CorpusInfo, corpora};

mod read;
pub use read::{DEFAULT_MAX_LINES, MAX_READ_LINES, QUOTE_INSTRUCTION, ReadOptions, ReadResult};

mod search;
pub use search::{
    DEFAULT_MAX_AGE_DAYS, DEFAULT_SEARCH_LIMIT, Freshness, MAX_SEARCH_LIMIT, MAX_SNIPPET_LINES,
    Scope, SearchQuery, SearchResult, SearchResults,
};

mod write;
pub use write::{ChunkRecord, DocumentRecord, SourceRecord, SymbolRecord};

/// The migrations, oldest first. Index `i` brings a file to version `i + 1`.
/// The files are compiled into the binary, so it needs no migrations
/// directory at run time.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_incremental_index.sql"),
    include_str!("../migrations/0003_document_versions.sql"),
    include_str!("../migrations/0004_read.sql"),
];

/// The schema version this build creates and reads.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// How long an open waits for another process's write lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// An open corpus file at the latest schema version.
pub struct Store {
    conn: Connection,
    path: PathBuf,
}

impl Store {
    /// Opens the corpus at `path` for indexing, creating the file if it is
    /// missing and applying any migrations it lacks.
    ///
    /// Refuses, without changing the file, one that isn't a codecite corpus
    /// or that has a newer schema than this build supports.
    pub fn open_read_write(path: &Path) -> Result<Store> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // Closing the last connection checkpoints any WAL a crashed writer
        // left into the main file. Hold that off until the file is accepted,
        // so a refused one stays as it was.
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)?;
        // A migration that rebuilds a table drops the old one, and with
        // foreign keys on, dropping a table first deletes its rows, which
        // its children's keys refuse. The bundled SQLite turns them on by
        // default, and the pragma can't change inside a transaction.
        conn.pragma_update(None, "foreign_keys", false)?;

        // `BEGIN IMMEDIATE` takes the write lock before reading the version,
        // so two indexers racing on a fresh file migrate it one after the
        // other. Any early return drops `tx`, which rolls it back.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found = read_version(&tx, path)?;
        if found > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.to_path_buf(),
                found,
                supported: SCHEMA_VERSION,
            });
        }
        for (version, sql) in (1..).zip(MIGRATIONS).skip(found as usize) {
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [version.to_string()],
            )?;
        }
        // With the keys off, nothing checked the rebuilt tables' rows.
        let orphans: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if orphans > 0 {
            return Err(Error::InvalidCorpus {
                path: path.to_path_buf(),
                reason: format!("{orphans} rows refer to rows that don't exist"),
            });
        }
        tx.commit()?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, false)?;
        // Every write from here on checks foreign keys and runs their
        // `ON DELETE CASCADE`.
        conn.pragma_update(None, "foreign_keys", true)?;

        // Switching to WAL can't run inside a transaction, and doing it only
        // now leaves a refused file's header untouched.
        enable_wal(&conn, path)?;
        // In WAL mode, NORMAL syncs at checkpoints rather than at every
        // commit. A power cut can lose the last commits but never corrupts
        // the file, and an index run commits once per document.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        search::register_functions(&conn)?;

        Ok(Store {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Opens the corpus at `path` for queries. SQLite itself rejects every
    /// write through the returned handle.
    ///
    /// Never creates or changes the file, so it accepts only a file already
    /// at [`SCHEMA_VERSION`]: an older one must be upgraded by re-running
    /// `codecite index`.
    pub fn open_read_only(path: &Path) -> Result<Store> {
        // SQLite's own error for a missing file doesn't name the path.
        if !path.exists() {
            return Err(Error::InvalidCorpus {
                path: path.to_path_buf(),
                reason: "no such file".into(),
            });
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;

        let found = read_version(&conn, path)?;
        if found > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.to_path_buf(),
                found,
                supported: SCHEMA_VERSION,
            });
        }
        if found < SCHEMA_VERSION {
            return Err(Error::InvalidCorpus {
                path: path.to_path_buf(),
                reason: format!(
                    "schema version {found} is older than {SCHEMA_VERSION}; re-run codecite index to upgrade it"
                ),
            });
        }
        search::register_functions(&conn)?;

        Ok(Store {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Opens the corpus named `corpus` in `dir`, the file
    /// `<dir>/<corpus>.db`, for queries.
    ///
    /// A name that isn't a plain file name is [`Error::InvalidCorpusName`],
    /// and one with no file is [`Error::UnknownCorpus`], listing the corpora
    /// `dir` does hold.
    pub fn open_corpus(dir: &Path, corpus: &str) -> Result<Store> {
        if !is_corpus_name(corpus) {
            return Err(Error::InvalidCorpusName {
                name: corpus.into(),
            });
        }
        let path = dir.join(format!("{corpus}.db"));
        if !path.exists() {
            return Err(Error::UnknownCorpus {
                corpus: corpus.into(),
                known: known_corpora(dir)?,
            });
        }
        Store::open_read_only(&path)
    }

    /// Opens `corpus` as [`Store::open_corpus`] does or, when it is `None`
    /// or empty, the one corpus `dir` holds. With none or several to
    /// choose from, that is [`Error::NoCorpusNamed`], listing them: never a
    /// guess.
    pub fn open_corpus_or_only(dir: &Path, corpus: Option<&str>) -> Result<Store> {
        if let Some(corpus) = corpus.filter(|name| !name.is_empty()) {
            return Store::open_corpus(dir, corpus);
        }
        match known_corpora(dir)?.as_slice() {
            [only] => Store::open_corpus(dir, only),
            known => Err(Error::NoCorpusNamed {
                known: known.to_vec(),
            }),
        }
    }

    /// The schema version stored in the file.
    pub fn schema_version(&self) -> Result<u32> {
        read_version(&self.conn, &self.path)
    }

    /// Where the corpus file is.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Whether `name` can name a corpus. It becomes a file name, so it may
/// hold only ASCII letters, digits, `-`, `_` and `.`, and may not start
/// with `.`.
pub fn is_corpus_name(name: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    !name.is_empty() && !name.starts_with('.') && name.chars().all(allowed)
}

/// The corpora in `dir`: the names of its `.db` files, sorted. A missing
/// directory holds none.
fn known_corpora(dir: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "db")
            && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            && is_corpus_name(name)
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Puts the file in WAL mode.
///
/// The switch takes a read lock and then upgrades it to a write lock. SQLite
/// fails that upgrade with `SQLITE_BUSY` at once, without calling the busy
/// handler, when another opener holds the write lock, so this retries until
/// [`BUSY_TIMEOUT`] runs out.
fn enable_wal(conn: &Connection, path: &Path) -> Result<()> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    let mode: String = loop {
        match conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0)) {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == ErrorCode::DatabaseBusy && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => break result?,
        }
    };
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(Error::InvalidCorpus {
            path: path.to_path_buf(),
            reason: format!("could not switch to WAL mode (journal mode is {mode})"),
        });
    }
    Ok(())
}

/// Reads `meta.schema_version`. A file with no schema objects at all is
/// version 0; any other file without a `meta` table isn't a codecite corpus.
fn read_version(conn: &Connection, path: &Path) -> Result<u32> {
    let invalid = |reason: &str| Error::InvalidCorpus {
        path: path.to_path_buf(),
        reason: reason.into(),
    };

    let has_meta: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_meta {
        let objects: i64 =
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
        return if objects == 0 {
            Ok(0)
        } else {
            Err(invalid("not a codecite corpus"))
        };
    }

    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    value
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| invalid("unreadable schema_version"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fresh, empty scratch directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("codecite-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Opens the file directly, bypassing `Store`.
    fn raw(path: &Path) -> Connection {
        Connection::open(path).unwrap()
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// A corpus at the latest version, then bumped one version past it.
    fn newer_schema_file(dir: &Path) -> PathBuf {
        let path = dir.join("corpus.db");
        drop(Store::open_read_write(&path).unwrap());
        raw(&path)
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                [(SCHEMA_VERSION + 1).to_string()],
            )
            .unwrap();
        path
    }

    #[test]
    fn fresh_file_gets_latest_version() {
        let dir = scratch("fresh");
        let store = Store::open_read_write(&dir.join("corpus.db")).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let tables = table_names(&store.conn);
        for want in [
            "meta",
            "source",
            "document",
            "chunk",
            "chunk_fts",
            "symbol",
            "edge",
        ] {
            assert!(tables.iter().any(|t| t == want), "missing table {want}");
        }
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reopening_is_a_no_op() {
        let dir = scratch("reopen");
        let path = dir.join("corpus.db");
        let store = Store::open_read_write(&path).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO source (name, kind, url, revision, license_class, indexed_at)
                 VALUES ('s', 'local', '.', 'abc', 'adapt', 'now')",
                [],
            )
            .unwrap();
        drop(store);

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let count: i64 = store
            .conn
            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_write_refuses_newer_schema_unchanged() {
        let dir = scratch("rw-newer");
        let path = newer_schema_file(&dir);
        let before = fs::read(&path).unwrap();

        let err = Store::open_read_write(&path).err().unwrap();
        assert!(
            matches!(
                err,
                Error::SchemaTooNew { found, supported, .. }
                    if found == SCHEMA_VERSION + 1 && supported == SCHEMA_VERSION
            ),
            "{err:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A newer-schema corpus whose version bump sits, committed but not yet
    /// checkpointed, in its WAL: the files a writer leaves when it dies.
    fn newer_schema_in_wal(dir: &Path) -> PathBuf {
        let live = dir.join("live.db");
        drop(Store::open_read_write(&live).unwrap());
        let writer = raw(&live);
        writer
            .execute_batch(&format!(
                "PRAGMA wal_autocheckpoint = 0;
                 UPDATE meta SET value = '{}' WHERE key = 'schema_version';",
                SCHEMA_VERSION + 1
            ))
            .unwrap();
        // Copy the files while the writer is still open, so its close-time
        // checkpoint can't reach them.
        let path = dir.join("corpus.db");
        fs::copy(&live, &path).unwrap();
        fs::copy(dir.join("live.db-wal"), dir.join("corpus.db-wal")).unwrap();
        drop(writer);
        path
    }

    #[test]
    fn read_write_refuses_newer_schema_in_wal_unchanged() {
        let dir = scratch("rw-newer-wal");
        let path = newer_schema_in_wal(&dir);
        let before = fs::read(&path).unwrap();

        let err = Store::open_read_write(&path).err().unwrap();
        assert!(matches!(err, Error::SchemaTooNew { .. }), "{err:?}");
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_refuses_newer_schema_in_wal_unchanged() {
        let dir = scratch("ro-newer-wal");
        let path = newer_schema_in_wal(&dir);
        let before = fs::read(&path).unwrap();

        let err = Store::open_read_only(&path).err().unwrap();
        assert!(matches!(err, Error::SchemaTooNew { .. }), "{err:?}");
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_switch_waits_for_another_writer() {
        let dir = scratch("wal-busy");
        let path = dir.join("corpus.db");
        // A migrated file still in rollback-journal mode: the state between
        // an open's migration commit and its WAL switch.
        raw(&path)
            .execute_batch(&format!(
                "{}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '{SCHEMA_VERSION}');",
                MIGRATIONS.join("\n")
            ))
            .unwrap();

        // A second opener holds the write lock for a moment.
        let other = raw(&path);
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            other.execute_batch("COMMIT").unwrap();
        });

        let conn = raw(&path);
        conn.busy_timeout(BUSY_TIMEOUT).unwrap();
        enable_wal(&conn, &path).unwrap();
        release.join().unwrap();
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_refuses_newer_schema() {
        let dir = scratch("ro-newer");
        let path = newer_schema_file(&dir);
        let err = Store::open_read_only(&path).err().unwrap();
        assert!(matches!(err, Error::SchemaTooNew { .. }), "{err:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_handle_cannot_write() {
        let dir = scratch("ro-write");
        let path = dir.join("corpus.db");
        drop(Store::open_read_write(&path).unwrap());

        let store = Store::open_read_only(&path).unwrap();
        let err = store
            .conn
            .execute("INSERT INTO meta VALUES ('k', 'v')", [])
            .unwrap_err();
        assert!(
            matches!(
                err,
                rusqlite::Error::SqliteFailure(e, _) if e.code == rusqlite::ErrorCode::ReadOnly
            ),
            "{err:?}"
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_missing_file_is_invalid_and_not_created() {
        let dir = scratch("ro-missing");
        let path = dir.join("corpus.db");
        let err = Store::open_read_only(&path).err().unwrap();
        assert!(matches!(err, Error::InvalidCorpus { .. }), "{err:?}");
        assert!(!path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_refuses_older_schema() {
        let dir = scratch("ro-older");
        let path = dir.join("corpus.db");
        raw(&path)
            .execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO meta VALUES ('schema_version', '0');",
            )
            .unwrap();

        let err = Store::open_read_only(&path).err().unwrap();
        assert!(
            matches!(&err, Error::InvalidCorpus { reason, .. } if reason.contains("codecite index")),
            "{err:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_write_refuses_non_sqlite_file_unchanged() {
        let dir = scratch("rw-text");
        let path = dir.join("corpus.db");
        let before = b"not a database, just text\n";
        fs::write(&path, before).unwrap();

        assert!(Store::open_read_write(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_write_refuses_foreign_database_unchanged() {
        let dir = scratch("rw-foreign");
        let path = dir.join("corpus.db");
        raw(&path).execute_batch("CREATE TABLE notes(x)").unwrap();
        let before = fs::read(&path).unwrap();

        let err = Store::open_read_write(&path).err().unwrap();
        assert!(matches!(err, Error::InvalidCorpus { .. }), "{err:?}");
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn concurrent_first_opens_both_succeed() {
        let dir = scratch("concurrent");
        let path = dir.join("corpus.db");
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || Store::open_read_write(&path))
            })
            .collect();
        for handle in handles {
            let store = handle.join().unwrap().unwrap();
            assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fts_rows_delete_by_rowid() {
        let dir = scratch("fts");
        let store = Store::open_read_write(&dir.join("corpus.db")).unwrap();
        let matches = || -> i64 {
            store
                .conn
                .query_row(
                    "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH 'validate'",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        };
        store
            .conn
            .execute(
                "INSERT INTO chunk_fts (rowid, symbol, path, terms)
                 VALUES (1, '', '', 'validate token validateToken')",
                [],
            )
            .unwrap();
        assert_eq!(matches(), 1);
        store
            .conn
            .execute("DELETE FROM chunk_fts WHERE rowid = 1", [])
            .unwrap();
        assert_eq!(matches(), 0);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_3_keeps_the_rows_and_cascades_deletes() {
        let dir = scratch("v2");
        let path = dir.join("corpus.db");
        // A version 2 corpus with one source, document, symbol and chunk.
        raw(&path)
            .execute_batch(&format!(
                "{}
                 {}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '2');
                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
                   VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then');
                 INSERT INTO document (id, source_id, path, content_hash, dirty)
                   VALUES (1, 1, 'src/lib.rs', x'00', 1);
                 INSERT INTO symbol (id, display_name, document_id) VALUES ('s', 'validate', 1);
                 INSERT INTO chunk (id, document_id, start_line, end_line, kind, symbol_id, text, terms)
                   VALUES (7, 1, 1, 1, 'function', 's', 'fn validate() {{}}', 'validate');
                 INSERT INTO chunk_fts (rowid, symbol, path, terms) VALUES (7, 'validate', 'lib', 'validate');",
                MIGRATIONS[0], MIGRATIONS[1]
            ))
            .unwrap();

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let (chunk, dirty): (i64, bool) = store
            .conn
            .query_row(
                "SELECT c.id, d.dirty FROM chunk c JOIN document d ON d.id = c.document_id
                 JOIN symbol s ON s.id = c.symbol_id JOIN source ON source.id = d.source_id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((chunk, dirty), (7, true));
        let fts = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH 'validate'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(fts(&store.conn), 1);

        store.conn.execute("DELETE FROM source", []).unwrap();
        for table in ["document", "symbol", "chunk"] {
            let rows: i64 = store
                .conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
        assert_eq!(fts(&store.conn), 0);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_4_makes_the_next_run_rewrite_code_files() {
        let dir = scratch("v3");
        let path = dir.join("corpus.db");
        // A version 3 corpus with a code file and a spec version.
        raw(&path)
            .execute_batch(&format!(
                "{}
                 {}
                 {}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '3');
                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
                   VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then'),
                          (2, 'rfc9449', 'document', 'https://x', 'def', 'quote', 'then');
                 INSERT INTO document (id, source_id, path, content_hash)
                   VALUES (1, 1, 'src/lib.rs', x'01'), (2, 2, 'rfc9449', x'02');",
                MIGRATIONS[0], MIGRATIONS[1], MIGRATIONS[2]
            ))
            .unwrap();

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let rows: Vec<(String, Vec<u8>, Option<String>)> = {
            let mut stmt = store
                .conn
                .prepare("SELECT path, content_hash, text FROM document ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            rows,
            [
                ("src/lib.rs".to_owned(), vec![], None),
                ("rfc9449".to_owned(), vec![2], None)
            ]
        );
        assert!(table_names(&store.conn).iter().any(|t| t == "alias"));
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_migration_that_leaves_orphans_is_refused_unchanged() {
        let dir = scratch("orphans");
        let path = dir.join("corpus.db");
        raw(&path)
            .execute_batch(&format!(
                "PRAGMA foreign_keys = OFF;
                 {}
                 {}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '2');
                 INSERT INTO document (id, source_id, path, content_hash) VALUES (1, 99, 'a', x'00');",
                MIGRATIONS[0], MIGRATIONS[1]
            ))
            .unwrap();
        let before = fs::read(&path).unwrap();

        let err = Store::open_read_write(&path).err().unwrap();
        assert!(
            matches!(&err, Error::InvalidCorpus { reason, .. } if reason.contains("1 rows")),
            "{err:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = scratch("fk");
        let store = Store::open_read_write(&dir.join("corpus.db")).unwrap();
        let err = store
            .conn
            .execute(
                "INSERT INTO document (source_id, path, content_hash) VALUES (99, 'a', x'00')",
                [],
            )
            .unwrap_err();
        assert!(
            matches!(
                &err,
                rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ConstraintViolation
            ),
            "{err:?}"
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corpus_names_are_plain_file_names() {
        for good in ["identity", "oidc-impls", "rfc_9449", "v1.2"] {
            assert!(is_corpus_name(good), "{good}");
        }
        for bad in ["", ".hidden", "..", "../x", "a/b", "a\\b", "名前", "a b"] {
            assert!(!is_corpus_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn open_corpus_names_the_known_corpora() {
        let dir = scratch("open-corpus");
        let err = Store::open_corpus(&dir.join("missing"), "identity")
            .err()
            .unwrap();
        assert!(
            matches!(&err, Error::UnknownCorpus { known, .. } if known.is_empty()),
            "{err:?}"
        );
        drop(Store::open_read_write(&dir.join("specs.db")).unwrap());
        drop(Store::open_read_write(&dir.join("identity.db")).unwrap());
        fs::write(dir.join("notes.txt"), "not a corpus").unwrap();
        let err = Store::open_corpus(&dir, "identty").err().unwrap();
        assert!(
            matches!(&err, Error::UnknownCorpus { corpus, known }
                if corpus == "identty" && known == &["identity", "specs"]),
            "{err:?}"
        );
        let err = Store::open_corpus(&dir, "../specs").err().unwrap();
        assert!(matches!(err, Error::InvalidCorpusName { .. }), "{err:?}");
        let store = Store::open_corpus(&dir, "specs").unwrap();
        assert_eq!(store.path(), dir.join("specs.db"));
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unnamed_corpus_is_the_only_one() {
        let dir = scratch("only-corpus");
        let err = Store::open_corpus_or_only(&dir.join("missing"), None)
            .err()
            .unwrap();
        assert!(
            matches!(&err, Error::NoCorpusNamed { known } if known.is_empty()),
            "{err:?}"
        );
        drop(Store::open_read_write(&dir.join("specs.db")).unwrap());
        let store = Store::open_corpus_or_only(&dir, None).unwrap();
        assert_eq!(store.path(), dir.join("specs.db"));
        drop(store);
        // An empty name is no name.
        let store = Store::open_corpus_or_only(&dir, Some("")).unwrap();
        assert_eq!(store.path(), dir.join("specs.db"));
        drop(store);
        drop(Store::open_read_write(&dir.join("identity.db")).unwrap());
        let err = Store::open_corpus_or_only(&dir, None).err().unwrap();
        assert!(
            matches!(&err, Error::NoCorpusNamed { known } if known == &["identity", "specs"]),
            "{err:?}"
        );
        let store = Store::open_corpus_or_only(&dir, Some("identity")).unwrap();
        assert_eq!(store.path(), dir.join("identity.db"));
        drop(store);
        let err = Store::open_corpus_or_only(&dir, Some("identty"))
            .err()
            .unwrap();
        assert!(matches!(err, Error::UnknownCorpus { .. }), "{err:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_migration_file_is_registered() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".sql"))
            .collect();
        names.sort();
        for (i, name) in names.iter().enumerate() {
            let prefix = format!("{:04}_", i + 1);
            assert!(
                name.starts_with(&prefix),
                "{name} should start with {prefix}"
            );
        }
        assert_eq!(names.len(), SCHEMA_VERSION as usize);
    }
}
