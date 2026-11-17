//! An index run: the lock that keeps a second run off the corpus, the one
//! transaction every write of a run goes into, and the `index_run` row that
//! records it.
//!
//! A run is [`Store::start_run`], then [`Store::begin_publish`] before its
//! first write, then [`Store::publish_run`], or [`Store::fail_run`] on any
//! error. Queries keep reading the last published corpus until the commit,
//! and a run that fails, or is killed, leaves it as it was.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, params};

use super::{BUSY_TIMEOUT, Store};
use crate::{Error, Result, TERMS_VERSION};

/// The full-text tokenizer `chunk_fts` splits terms with, as its schema
/// names it.
pub const FTS_TOKENIZER: &str = "unicode61";

/// What an index run records about how it indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRun<'a> {
    /// The codecite build's version.
    pub codecite_version: &'a str,
    /// The sha256 of the manifest's text, in hex.
    pub manifest_sha256: &'a str,
    /// The version of the chunker that split the files.
    pub chunker_version: u32,
}

/// What an index run wrote, recorded on its row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunCounts {
    /// Documents written.
    pub written: usize,
    /// Documents left as they were.
    pub unchanged: usize,
    /// Documents deleted.
    pub removed: usize,
    /// Chunks written.
    pub chunks: usize,
}

impl Store {
    /// Takes the corpus's index lock, then opens it as
    /// [`Store::open_read_write`] does, so a second run fails on the lock
    /// rather than on the first one's migrations or writes.
    pub fn open_for_index(path: &Path) -> Result<Store> {
        let lock = lock(path)?;
        let mut store = Store::open_read_write(path)?;
        store.lock = Some(lock);
        Ok(store)
    }

    /// Takes the exclusive lock on `<corpus>.db.lock`, held until the store
    /// is dropped, unless the store already holds it. A lock another holder
    /// keeps, in this process or another, for longer than the store's busy
    /// timeout is [`Error::CorpusLocked`]: a run never waits for another
    /// run to finish.
    pub fn lock_for_index(&mut self) -> Result<()> {
        if self.lock.is_none() {
            self.lock = Some(lock(&self.path)?);
        }
        Ok(())
    }

    /// Records the start of a run, in its own transaction, and returns its
    /// ID. Its status is `running` until [`Store::publish_run`] or
    /// [`Store::fail_run`]; a run that is killed stays `running`, with no
    /// `finished_at`.
    pub fn start_run(&mut self, run: &IndexRun<'_>) -> Result<i64> {
        let id = self.conn.query_row(
            "INSERT INTO index_run (started_at, status, codecite_version, manifest_sha256,
               chunker_version, terms_version, tokenizer)
             VALUES (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 'running', ?1, ?2, ?3, ?4, ?5)
             RETURNING id",
            params![
                run.codecite_version,
                run.manifest_sha256,
                run.chunker_version,
                TERMS_VERSION,
                FTS_TOKENIZER,
            ],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Begins the run's one transaction, taking SQLite's write lock now.
    /// Every write until [`Store::publish_run`] goes into it.
    pub fn begin_publish(&mut self) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(())
    }

    /// Stamps every source with the run, writes `meta.manifest_hash`,
    /// marks the run `succeeded` with `counts`, and commits the run's
    /// transaction. On an error the transaction is still open, for
    /// [`Store::fail_run`] to roll back.
    pub fn publish_run(&mut self, run_id: i64, counts: &RunCounts) -> Result<()> {
        let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        self.conn
            .execute("UPDATE source SET index_run_id = ?1", [run_id])?;
        self.conn.execute(
            "INSERT INTO meta (key, value)
             SELECT 'manifest_hash', manifest_sha256 FROM index_run WHERE id = ?1
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [run_id],
        )?;
        self.conn.execute(
            "UPDATE index_run SET status = 'succeeded',
               finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
               sources = (SELECT count(*) FROM source),
               written = ?2, unchanged = ?3, removed = ?4, chunks = ?5
             WHERE id = ?1",
            params![
                run_id,
                count(counts.written),
                count(counts.unchanged),
                count(counts.removed),
                count(counts.chunks),
            ],
        )?;
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    /// Rolls back the run's transaction, if it is open, so the corpus is as
    /// the last published run left it, then marks the run `failed` with
    /// `error`.
    ///
    /// The store never stays inside the run's transaction: when `ROLLBACK`
    /// itself fails, the connection is closed, which rolls the transaction
    /// back and frees SQLite's write lock, and the file is opened again.
    /// The run is marked `failed` whether or not the rollback needed that,
    /// and the first error of the two steps is returned.
    pub fn fail_run(&mut self, run_id: i64, error: &str) -> Result<()> {
        self.fail_run_with(run_id, error, |conn| conn.execute_batch("ROLLBACK"))
    }

    /// [`Store::fail_run`], rolling back with `rollback`, so a test can
    /// make it fail.
    fn fail_run_with(
        &mut self,
        run_id: i64,
        error: &str,
        rollback: impl FnOnce(&Connection) -> rusqlite::Result<()>,
    ) -> Result<()> {
        let rolled_back = if self.conn.is_autocommit() {
            Ok(())
        } else {
            rollback(&self.conn).or_else(|e| {
                self.reopen()?;
                Err(e.into())
            })
        };
        let recorded = self
            .conn
            .execute(
                "UPDATE index_run SET status = 'failed',
                   finished_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), error = ?2
                 WHERE id = ?1",
                params![run_id, error],
            )
            .map(drop)
            .map_err(Error::from);
        rolled_back.and(recorded)
    }

    /// Closes the connection, which rolls back any transaction it has open,
    /// and opens the file again as [`Store::open_read_write`] does. Until the
    /// new connection is open, the store holds an empty in-memory database,
    /// so if opening fails, every later call on the store fails too rather
    /// than writing into a transaction that is never committed.
    fn reopen(&mut self) -> Result<()> {
        drop(std::mem::replace(
            &mut self.conn,
            Connection::open_in_memory()?,
        ));
        let fresh = Store::open_read_write(&self.path)?;
        self.conn = fresh.conn;
        Ok(())
    }
}

/// The lock file of the corpus at `path`: `<corpus>.db.lock`.
fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Opens the corpus's lock file, creating it, and locks it exclusively.
///
/// A lock that is held is tried again until [`BUSY_TIMEOUT`] runs out, then
/// refused. The short wait is for a lock that is only briefly held: a
/// process forked by another thread holds a copy of the lock's file until
/// it runs its program, which closes it.
///
/// Any other failure to take the lock, such as a filesystem that has no
/// file locks, is [`Error::CorpusLockFailed`]: the run stops rather than
/// go on unlocked, since SQLite's own locking, which WAL mode relies on, is
/// no safer there.
fn lock(path: &Path) -> Result<File> {
    let lock_path = lock_path(path);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| lock_error(&lock_path, TryLockError::Error(e)))?;
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(lock_error(&lock_path, e)),
        }
    }
}

/// The error for a lock on `lock_path` that couldn't be taken.
fn lock_error(lock_path: &Path, error: TryLockError) -> Error {
    match error {
        TryLockError::WouldBlock => Error::CorpusLocked {
            path: lock_path.to_owned(),
        },
        TryLockError::Error(source) => Error::CorpusLockFailed {
            path: lock_path.to_owned(),
            source,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceRecord;

    const RUN: IndexRun<'static> = IndexRun {
        codecite_version: "0.0.0-test",
        manifest_sha256: "ab12",
        chunker_version: 7,
    };

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

    /// One `index_run` row: status, error, manifest sha256, versions,
    /// tokenizer, counts and whether it finished.
    type RunRow = (
        String,
        Option<String>,
        String,
        u32,
        u32,
        String,
        Option<i64>,
        Option<i64>,
        bool,
    );

    fn run_row(store: &Store, id: i64) -> RunRow {
        store
            .conn
            .query_row(
                "SELECT status, error, manifest_sha256, chunker_version, terms_version, tokenizer,
                        sources, written, finished_at IS NOT NULL
                 FROM index_run WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .unwrap()
    }

    #[test]
    fn second_lock_fails_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpus.db");
        let first = Store::open_for_index(&path).unwrap();
        let Err(err) = Store::open_for_index(&path) else {
            panic!("a second run took the lock");
        };
        assert!(
            matches!(&err, Error::CorpusLocked { path: p } if p.ends_with("corpus.db.lock")),
            "{err:?}"
        );
        assert!(err.to_string().contains("another codecite index run"));
        // A store opened without the lock can't take it either.
        let mut other = Store::open_read_write(&path).unwrap();
        assert!(matches!(
            other.lock_for_index(),
            Err(Error::CorpusLocked { .. })
        ));
        drop(first);
        other.lock_for_index().unwrap();
        // Taking it again is a no-op.
        other.lock_for_index().unwrap();
    }

    #[test]
    fn failed_run_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_for_index(&dir.path().join("corpus.db")).unwrap();
        let run = store.start_run(&RUN).unwrap();
        store.begin_publish().unwrap();
        store.upsert_source(&source("app")).unwrap();
        store.set_indexed_at(30).unwrap();
        store.fail_run(run, "boom").unwrap();

        let sources: i64 = store
            .conn
            .query_row("SELECT count(*) FROM source", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sources, 0);
        let meta: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM meta WHERE key <> 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(meta, 0);
        let row = run_row(&store, run);
        assert_eq!((row.0.as_str(), row.1.as_deref()), ("failed", Some("boom")));
        assert!(row.8, "a failed run has a finish time");
        // Failing before the transaction began is fine too.
        let early = store.start_run(&RUN).unwrap();
        store.fail_run(early, "no network").unwrap();
        assert_eq!(run_row(&store, early).0, "failed");
    }

    #[test]
    fn published_run_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_for_index(&dir.path().join("corpus.db")).unwrap();
        let run = store.start_run(&RUN).unwrap();
        assert_eq!(run_row(&store, run).0, "running");
        store.begin_publish().unwrap();
        let id = store.upsert_source(&source("app")).unwrap();
        let counts = RunCounts {
            written: 3,
            unchanged: 1,
            removed: 0,
            chunks: 9,
        };
        store.publish_run(run, &counts).unwrap();
        assert!(store.conn.is_autocommit(), "the run committed");

        assert_eq!(
            run_row(&store, run),
            (
                "succeeded".into(),
                None,
                "ab12".into(),
                7,
                TERMS_VERSION,
                FTS_TOKENIZER.into(),
                Some(1),
                Some(3),
                true
            )
        );
        let stamped: Option<i64> = store
            .conn
            .query_row("SELECT index_run_id FROM source WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stamped, Some(run));
        let hash: String = store
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'manifest_hash'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hash, "ab12");
    }

    #[test]
    fn a_failed_rollback_closes_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpus.db");
        let mut store = Store::open_for_index(&path).unwrap();
        let run = store.start_run(&RUN).unwrap();
        store.begin_publish().unwrap();
        store.upsert_source(&source("app")).unwrap();

        let err = store
            .fail_run_with(run, "boom", |_| Err(rusqlite::Error::InvalidQuery))
            .unwrap_err();
        assert!(matches!(err, Error::Sqlite(_)), "{err:?}");
        // The run's transaction is gone with the old connection: nothing
        // it wrote is kept, and another writer can take the write lock.
        assert!(store.conn.is_autocommit());
        let sources: i64 = store
            .conn
            .query_row("SELECT count(*) FROM source", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sources, 0);
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch("BEGIN IMMEDIATE; ROLLBACK").unwrap();
        // The run is still recorded, and the store still works.
        let row = run_row(&store, run);
        assert_eq!((row.0.as_str(), row.1.as_deref()), ("failed", Some("boom")));
        store.upsert_source(&source("app")).unwrap();
    }

    #[test]
    fn a_lock_that_cannot_be_taken_names_the_lock_file() {
        let path = Path::new("/data/identity.db.lock");
        let err = lock_error(
            path,
            TryLockError::Error(std::io::Error::from(std::io::ErrorKind::Unsupported)),
        );
        assert!(
            matches!(&err, Error::CorpusLockFailed { path: p, .. } if p == path),
            "{err:?}"
        );
        let message = err.to_string();
        assert!(message.contains("/data/identity.db.lock"), "{message}");
        assert!(message.contains("local filesystem"), "{message}");

        // Nor can a lock file that can't be created.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-dir").join("corpus.db");
        let Err(err) = Store::open_for_index(&missing) else {
            panic!("locked a corpus in a missing directory");
        };
        assert!(matches!(err, Error::CorpusLockFailed { .. }), "{err:?}");
    }

    #[test]
    fn the_lock_file_sits_next_to_the_corpus() {
        assert_eq!(
            lock_path(Path::new("/data/identity.db")),
            Path::new("/data/identity.db.lock")
        );
    }
}
