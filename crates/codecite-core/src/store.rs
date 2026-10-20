//! The per-corpus SQLite file: opening it, and bringing it to the latest
//! schema with numbered migrations.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::{Error, Result};

/// The migrations, oldest first. Index `i` brings a file to version `i + 1`.
/// The files are compiled into the binary, so it needs no migrations
/// directory at run time.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_initial.sql")];

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
        tx.commit()?;

        // Switching to WAL can't run inside a transaction, and doing it only
        // now leaves a refused file's header untouched.
        let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::InvalidCorpus {
                path: path.to_path_buf(),
                reason: format!("could not switch to WAL mode (journal mode is {mode})"),
            });
        }

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

        Ok(Store {
            conn,
            path: path.to_path_buf(),
        })
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
