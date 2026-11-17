//! The per-corpus SQLite file: opening it, and bringing it to the latest
//! schema with numbered migrations.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::{Error, Result, Role};

mod cite;
pub use cite::{
    CheckOptions, CitationChecker, CitationProblem, CiteOptions, CiteResult, Finding, Problem,
    TextCheck,
};

mod context;
pub use context::{Context, ContextQuery, ContextTarget, MAX_CONTEXT_BYTES, Rule, RuleScope};

mod corpora;
pub use corpora::{Corpora, CorpusInfo, ManifestPin, PinReader, StalePin, corpora};

mod definition;
pub use definition::{Definition, Definitions};

mod diff;
pub use diff::{DiffOptions, DiffResult, VersionChange};

mod grep;
pub use grep::{GrepMatch, GrepQuery, GrepResults};

mod read;
pub use read::{
    APPROACH_ONLY_INSTRUCTION, DEFAULT_MAX_LINES, MAX_READ_BYTES, MAX_READ_LINES,
    QUOTE_INSTRUCTION, QUOTE_TRUNCATED_INSTRUCTION, ReadOptions, ReadResult, defuse_instruction,
};

mod lines;
use lines::LineStarts;

mod requirements;
pub use requirements::{Requirement, RequirementSection, Requirements, RequirementsOptions};

mod search;
pub use search::{
    DEFAULT_MAX_AGE_DAYS, DEFAULT_SEARCH_LIMIT, Freshness, MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES,
    MAX_SNIPPET_LINES, Scope, SearchQuery, SearchResult, SearchResults,
};

mod run;
pub use run::{FTS_TOKENIZER, IndexRun, RunCounts};

mod write;
pub use write::{ChunkRecord, DocumentRecord, SourceRecord, StoredDocument, SymbolRecord};

/// One numbered migration.
struct Migration {
    /// The SQL, compiled into the binary, so it needs no migrations
    /// directory at run time.
    sql: &'static str,
    /// Whether this build's queries can read a file that stops just before
    /// this migration: it only adds what no query reads, or changes
    /// derived data an index run rewrites. A query opens a file older than
    /// [`SCHEMA_VERSION`] with a warning when every migration it lacks is
    /// read-compatible, and refuses it otherwise. A migration stops being
    /// read-compatible once a query reads what it adds.
    read_compatible: bool,
}

/// The migrations, oldest first. Index `i` brings a file to version `i + 1`.
///
/// A migration that changes how documents are split or their terms made
/// sets `document.chunker_version` or `document.terms_version` to 0 for
/// the documents it affects, from version 13 on, so the next index run
/// splits them again. Raising `codecite_index::CHUNKER_VERSION` or
/// [`TERMS_VERSION`](crate::TERMS_VERSION) needs no migration at all.
const MIGRATIONS: &[Migration] = &[
    Migration {
        sql: include_str!("../migrations/0001_initial.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0002_incremental_index.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0003_document_versions.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0004_read.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0005_spec_fts.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0006_guidance.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0007_heading_anchors.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0008_refill_fts.sql"),
        read_compatible: false,
    },
    Migration {
        sql: include_str!("../migrations/0009_one_ranking.sql"),
        read_compatible: false,
    },
    // Search reads the role weights and name keys it adds.
    Migration {
        sql: include_str!("../migrations/0010_rank_inputs.sql"),
        read_compatible: false,
    },
    // Adds the index run log, which no query reads.
    Migration {
        sql: include_str!("../migrations/0011_index_run.sql"),
        read_compatible: true,
    },
    // Drops `chunk.terms`, which no query reads, and clears chunk text a
    // document's text holds. Queries read a chunk's text as
    // `coalesce(c.text, codecite_lines(...))`, which an older file, whose
    // chunks all keep their text, answers the same way.
    Migration {
        sql: include_str!("../migrations/0012_derive_chunk_text.sql"),
        read_compatible: true,
    },
    // Adds the derived-data versions and `meta.tokenizer`, which no query
    // reads.
    Migration {
        sql: include_str!("../migrations/0013_derived_versions.sql"),
        read_compatible: true,
    },
];

/// The version whose migration clears the chunk text a document's text
/// holds. A file brought up past it is vacuumed, so the space comes back.
const DERIVE_CHUNK_TEXT: u32 = 12;

/// The `meta` key that marks a file a migration freed space in: set in the
/// migration's transaction and deleted only once a vacuum succeeds, so a
/// vacuum that fails is tried again on the next read-write open.
const VACUUM_PENDING: &str = "vacuum_pending";

#[cfg(test)]
thread_local! {
    /// Makes the next vacuum on this thread fail, as a full disk would.
    static FAIL_VACUUM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Vacuums the file if a migration left it flagged, then deletes the flag.
/// VACUUM rewrites the whole file and can't run inside a transaction, so
/// this runs after the migrations commit; a failure, such as too little
/// disk for the copy or another writer holding the lock past the busy
/// timeout, leaves the flag for the next open. Returns SQLite's own error,
/// whose message says what went wrong.
fn vacuum_if_pending(conn: &Connection) -> rusqlite::Result<()> {
    let pending: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [VACUUM_PENDING],
            |row| row.get(0),
        )
        .optional()?;
    if pending.is_none() {
        return Ok(());
    }
    #[cfg(test)]
    if FAIL_VACUUM.get() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            Some("injected vacuum failure".into()),
        ));
    }
    conn.execute_batch("VACUUM")?;
    conn.execute("DELETE FROM meta WHERE key = ?1", [VACUUM_PENDING])?;
    Ok(())
}

/// The version whose migration empties the full-text tables, to drop the
/// totals an older SQLite left wrong. The same transaction refills
/// `chunk_fts` from the stored chunks, so the file never commits without
/// it and a failed index run after the upgrade leaves it searchable.
/// (`spec_fts` stays empty; version 9 drops it.)
const REFILL_FTS: u32 = 8;

/// Writes `chunk_fts`'s row for every stored chunk, by
/// [`write::fts_row`], the rule `insert_chunks` writes by. A chunk's text
/// is read as queries read it, so this works on any version's rows.
fn refill_fts(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let mut chunks = tx.prepare(
        "SELECT c.id, sym.display_name, d.path,
           coalesce(c.text, codecite_lines(d.text, c.start_line, c.end_line))
         FROM chunk c
         JOIN document d ON d.id = c.document_id
         LEFT JOIN symbol sym ON sym.id = c.symbol_id
         ORDER BY c.id",
    )?;
    let mut insert =
        tx.prepare("INSERT INTO chunk_fts (rowid, symbol, path, terms) VALUES (?1, ?2, ?3, ?4)")?;
    let mut rows = chunks.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let symbol: Option<String> = row.get(1)?;
        let path: String = row.get(2)?;
        let text: String = row.get(3)?;
        let [symbol, path, terms] = write::fts_row(symbol.as_deref(), &path, &text);
        insert.execute(rusqlite::params![id, symbol, path, terms])?;
    }
    Ok(())
}

/// Clears `chunk.text` wherever the chunk's document stores its text and
/// the chunk is that text's lines, as `insert_chunks` writes a chunk from
/// version 12 on. A chunk whose text differs from its lines keeps it.
fn clear_derivable_text(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let mut documents = tx.prepare("SELECT id, text FROM document WHERE text IS NOT NULL")?;
    let mut chunks = tx.prepare(
        "SELECT id, start_line, end_line, text FROM chunk
         WHERE document_id = ?1 AND text IS NOT NULL",
    )?;
    let mut clear = tx.prepare("UPDATE chunk SET text = NULL WHERE id = ?1")?;
    let mut rows = documents.query([])?;
    while let Some(row) = rows.next()? {
        let document_id: i64 = row.get(0)?;
        let text: String = row.get(1)?;
        let lines = LineStarts::new(&text);
        let mut derivable = Vec::new();
        let mut chunk_rows = chunks.query([document_id])?;
        while let Some(c) = chunk_rows.next()? {
            let own: String = c.get(3)?;
            if lines.span(c.get(1)?, c.get(2)?) == Some(own.as_str()) {
                derivable.push(c.get::<_, i64>(0)?);
            }
        }
        drop(chunk_rows);
        for id in derivable {
            clear.execute([id])?;
        }
    }
    Ok(())
}

/// `PRAGMA application_id` of a codecite corpus: `cdct` in ASCII. A
/// codecite file written before it was set has 0.
pub const APPLICATION_ID: i32 = 0x6364_6374;

/// The version whose migration stores what search ranks by: each
/// document's role and weight and each symbol's name key.
const RANK_INPUTS: u32 = 10;

/// Sets every stored document's `role` and `role_weight` and every
/// symbol's `name_key`, as the write path sets them, so an upgraded corpus
/// ranks as a fresh index does. Version [`RANK_INPUTS`] adds the columns.
fn fill_rank_inputs(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let documents: Vec<(i64, String, String, bool)> = tx
        .prepare(
            "SELECT d.id, d.path, s.kind, d.text IS NOT NULL
             FROM document d JOIN source s ON s.id = d.source_id",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut set_role =
        tx.prepare("UPDATE document SET role = ?2, role_weight = ?3 WHERE id = ?1")?;
    for (id, path, source_kind, has_text) in documents {
        let role = Role::of_stored(&path, &source_kind, has_text);
        set_role.execute(rusqlite::params![id, role.as_str(), role.weight()])?;
    }
    let symbols: Vec<(String, String)> = tx
        .prepare("SELECT id, display_name FROM symbol")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut set_key = tx.prepare("UPDATE symbol SET name_key = ?2 WHERE id = ?1")?;
    for (id, display_name) in symbols {
        set_key.execute(rusqlite::params![id, search::name_key(&display_name)])?;
    }
    Ok(())
}

/// The schema version this build creates and reads.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// How long an open waits for another process's write lock before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// An open corpus file at the latest schema version.
pub struct Store {
    conn: Connection,
    path: PathBuf,
    /// Why a file opened for queries is read at an older schema.
    schema_warning: Option<String>,
    vacuum_error: Option<String>,
    /// The exclusive lock on `<path>.lock` an index run holds, released
    /// when the store is dropped.
    lock: Option<std::fs::File>,
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
        // Migrations read chunk text through `codecite_lines`.
        search::register_functions(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found = read_version(&tx, path)?;
        if found > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.to_path_buf(),
                found,
                supported: SCHEMA_VERSION,
            });
        }
        for (version, migration) in (1..).zip(MIGRATIONS).skip(found as usize) {
            tx.execute_batch(migration.sql)?;
            match version {
                REFILL_FTS => refill_fts(&tx)?,
                // What search ranks by, for the rows stored before it.
                RANK_INPUTS => fill_rank_inputs(&tx)?,
                DERIVE_CHUNK_TEXT => {
                    clear_derivable_text(&tx)?;
                    // A file that was empty had nothing to free.
                    if found > 0 {
                        tx.execute(
                            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')",
                            [VACUUM_PENDING],
                        )?;
                    }
                }
                _ => {}
            }
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [version.to_string()],
            )?;
        }
        let id: i32 = tx.query_row("PRAGMA application_id", [], |row| row.get(0))?;
        if id != APPLICATION_ID {
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
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
        // Clearing chunk text frees pages but doesn't shrink the file. The
        // migrations are committed whatever the vacuum does, so a failed one
        // doesn't fail the open: the caller hears why, and the next open
        // tries again.
        let vacuum_error = vacuum_if_pending(&conn).err().map(|e| e.to_string());
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, false)?;
        // Every write from here on checks foreign keys and runs their
        // `ON DELETE CASCADE`.
        conn.pragma_update(None, "foreign_keys", true)?;

        // Switching to WAL can't run inside a transaction, and doing it only
        // now leaves a refused file's header untouched.
        enable_wal(&conn, path)?;
        // In WAL mode, NORMAL syncs at checkpoints rather than at every
        // commit. A power cut can lose the last commits but never corrupts
        // the file, and an index run commits once, when it publishes.
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        Ok(Store {
            conn,
            path: path.to_path_buf(),
            schema_warning: None,
            vacuum_error,
            lock: None,
        })
    }

    /// Opens the corpus at `path` for queries. SQLite itself rejects every
    /// write through the returned handle.
    ///
    /// Never creates or changes the file. An older file opens only when
    /// every migration it lacks is read-compatible, with a
    /// [`Store::schema_warning`] saying so; any other must be upgraded by
    /// re-running `codecite index`.
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
        let mut schema_warning = None;
        if found < SCHEMA_VERSION {
            let lacking = &MIGRATIONS[found as usize..];
            if found == 0 || !lacking.iter().all(|m| m.read_compatible) {
                return Err(Error::InvalidCorpus {
                    path: path.to_path_buf(),
                    reason: format!(
                        "schema version {found} is older than {SCHEMA_VERSION}; re-run codecite index to upgrade it"
                    ),
                });
            }
            schema_warning = Some(format!(
                "corpus file {} has schema version {found}, older than {SCHEMA_VERSION}; it can still be read, and re-running codecite index upgrades it",
                path.display()
            ));
        }
        search::register_functions(&conn)?;

        Ok(Store {
            conn,
            path: path.to_path_buf(),
            schema_warning,
            vacuum_error: None,
            lock: None,
        })
    }

    /// Why the vacuum [`Store::open_read_write`] owed the file failed, when
    /// it did. The open went ahead, the file is valid and fully migrated,
    /// and only its freed space stays in it until a later read-write open
    /// vacuums it.
    pub fn vacuum_error(&self) -> Option<&str> {
        self.vacuum_error.as_deref()
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

    /// For a file opened for queries at an older, read-compatible schema,
    /// a warning saying so, for the caller to show; `None` otherwise.
    pub fn schema_warning(&self) -> Option<&str> {
        self.schema_warning.as_deref()
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

/// Reads `meta.schema_version`. A file whose `application_id` is neither
/// codecite's nor 0 isn't a codecite corpus. Of the rest, which include
/// codecite files written before the id was set, a file with no schema
/// objects at all is version 0, and any other file without a `meta` table
/// isn't a codecite corpus.
fn read_version(conn: &Connection, path: &Path) -> Result<u32> {
    let invalid = |reason: &str| Error::InvalidCorpus {
        path: path.to_path_buf(),
        reason: reason.into(),
    };

    let id: i32 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if id != 0 && id != APPLICATION_ID {
        return Err(invalid(&format!(
            "not a codecite corpus (application_id {id:#010x})"
        )));
    }

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
    use crate::{identifier_terms, text_terms};
    use std::fs;

    /// A fresh, empty scratch directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("codecite-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The SQL of the first `n` migrations, which brings a fresh file to
    /// version `n`.
    fn up_to(n: usize) -> String {
        MIGRATIONS[..n]
            .iter()
            .map(|m| m.sql)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Opens the file directly, bypassing `Store`.
    fn raw(path: &Path) -> Connection {
        Connection::open(path).unwrap()
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info(?1)")
            .unwrap();
        stmt.query_map([table], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
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
                up_to(MIGRATIONS.len())
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
                MIGRATIONS[0].sql, MIGRATIONS[1].sql
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
                MIGRATIONS[0].sql, MIGRATIONS[1].sql, MIGRATIONS[2].sql
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
    fn a_version_4_corpus_upgrades_through_spec_fts_and_its_removal() {
        let dir = scratch("v4");
        let path = dir.join("corpus.db");
        // A version 4 corpus with a code chunk and a numbered spec chunk,
        // as a spec section is stored. Versions 5 and 8 fill spec_fts and
        // version 9 drops it, so the way up must run all three.
        raw(&path)
            .execute_batch(&format!(
                "{}
                 {}
                 {}
                 {}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '4');
                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
                   VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then'),
                          (2, 'rfc9449', 'document', 'https://x', 'def', 'quote', 'then');
                 INSERT INTO document (id, source_id, path, content_hash)
                   VALUES (1, 1, 'src/lib.rs', x'01'), (2, 2, 'rfc9449', x'02');
                 INSERT INTO chunk (id, document_id, start_line, end_line, kind, number, text,
                                    terms)
                   VALUES (1, 1, 1, 1, 'window', NULL, 'validate', 'validate'),
                          (2, 2, 1, 1, 'section', '1', 'validates', 'validates');
                 INSERT INTO chunk_fts (rowid, symbol, path, terms)
                   VALUES (1, '', 'src lib rs', 'validate'), (2, '', 'rfc9449', 'validates');",
                MIGRATIONS[0].sql, MIGRATIONS[1].sql, MIGRATIONS[2].sql, MIGRATIONS[3].sql
            ))
            .unwrap();

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let ids = |sql: &str| -> Vec<i64> {
            let mut stmt = store.conn.prepare(sql).unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        // spec_fts is gone, and both chunks stay in chunk_fts, the spec one
        // with its path's terms.
        assert_eq!(
            ids("SELECT count(*) FROM sqlite_master WHERE name LIKE 'spec_fts%'"),
            [0]
        );
        assert_eq!(
            ids("SELECT rowid FROM chunk_fts WHERE chunk_fts MATCH 'path:rfc9449'"),
            [2]
        );
        assert_eq!(
            ids(
                "SELECT rowid FROM chunk_fts WHERE chunk_fts MATCH 'validate OR validates' ORDER BY rowid"
            ),
            [1, 2]
        );
        // Nothing has to be indexed again.
        assert_eq!(
            ids("SELECT terms_version FROM document ORDER BY id"),
            [1, 1]
        );
        assert_eq!(
            ids("SELECT chunker_version FROM document ORDER BY id"),
            [1, 1]
        );
        assert_eq!(
            ids("SELECT count(*) FROM document WHERE content_hash = x''"),
            [0]
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_7_makes_markdown_documents_split_again() {
        let dir = scratch("v6");
        let path = dir.join("corpus.db");
        // A version 6 corpus with a Markdown file, a Rust file and a
        // guidance document, which keeps no text.
        raw(&path)
            .execute_batch(&format!(
                "{}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '6');
                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
                   VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then'),
                          (2, 'rules', 'document', 'https://x', 'def', 'quote', 'then');
                 INSERT INTO document (id, source_id, path, language, content_hash, text)
                   VALUES (1, 1, 'README.md', 'markdown', x'01', '# App'),
                          (2, 1, 'src/lib.rs', 'rust', x'02', 'fn a() {{}}'),
                          (3, 2, 'rules', 'markdown', x'03', NULL);",
                up_to(6)
            ))
            .unwrap();

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let mut stmt = store
            .conn
            .prepare("SELECT path FROM document WHERE content_hash = x'' ORDER BY id")
            .unwrap();
        let cleared: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(cleared, ["README.md"]);
        drop(stmt);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_8_refills_the_full_text_table_in_its_own_transaction() {
        let dir = scratch("v7");
        let path = dir.join("corpus.db");
        // A version 7 corpus with a function, a spec section and a generic
        // document's section, whose full-text rows are as `insert_chunks`
        // writes them: every chunk in chunk_fts, numbered ones in spec_fts.
        // Version 9 drops spec_fts, so only chunk_fts is checked after.
        // Its totals are wrong, as an older SQLite left them.
        let conn = raw(&path);
        conn.execute_batch(&format!(
            "{}
             INSERT INTO meta (key, value) VALUES ('schema_version', '7');
             INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
               VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then'),
                      (2, 'rfc', 'document', 'https://x', 'def', 'quote', 'then'),
                      (3, 'notes', 'document', 'https://y', 'ghi', 'quote', 'then');
             INSERT INTO document (id, source_id, path, language, content_hash, text)
               VALUES (1, 1, 'src/lib.rs', 'rust', x'01', 'pub fn validate_token() {{}}'),
                      (2, 2, 'rfc', 'html', x'02', NULL),
                      (3, 3, 'notes', 'markdown', x'03', '# Sessions');
             INSERT INTO symbol (id, display_name, kind, document_id, start_line, end_line)
               VALUES ('app . validate_token().', 'validate_token', 'function', 1, 1, 1);
             INSERT INTO chunk (id, document_id, start_line, end_line, kind, symbol_id, number,
                                text, terms)
               VALUES (1, 1, 1, 1, 'function', 'app . validate_token().', NULL,
                       'pub fn validate_token() {{}}', 'pub fn validate token validate_token'),
                      (2, 2, 1, 3, 'section', NULL, '1', 'Tokens are validated.',
                       'tokens are validated'),
                      (3, 3, 1, 1, 'section', NULL, NULL, '# Sessions', 'sessions');",
            up_to(7)
        ))
        .unwrap();
        // Every row's terms are its chunk's text's, as `insert_chunks`
        // wrote them, so the refill gives the same totals.
        let symbol = identifier_terms("validate_token");
        for (table, id, symbol, path, text) in [
            (
                "chunk_fts",
                1,
                symbol.as_str(),
                "src/lib.rs",
                "pub fn validate_token() {}",
            ),
            ("chunk_fts", 2, "", "rfc", "Tokens are validated."),
            ("chunk_fts", 3, "", "notes", "# Sessions"),
            ("spec_fts", 2, "", "rfc", "Tokens are validated."),
        ] {
            conn.execute(
                &format!(
                    "INSERT INTO {table} (rowid, symbol, path, terms) VALUES (?1, ?2, ?3, ?4)"
                ),
                rusqlite::params![id, symbol, text_terms(path), text_terms(text)],
            )
            .unwrap();
        }
        let totals = |conn: &Connection| -> Vec<u8> {
            conn.query_row("SELECT block FROM chunk_fts_data WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap()
        };
        let right = totals(&conn);
        // An older SQLite left deleted rows in the totals: the record's
        // first varint, the row count, too high.
        let mut block = totals(&conn);
        block[0] += 5;
        conn.execute("UPDATE chunk_fts_data SET block = ?1 WHERE id = 1", [block])
            .unwrap();
        assert_ne!(totals(&conn), right);
        drop(conn);

        // The index run that opens the file to upgrade it then fails, so
        // nothing is written after the migration commits. The corpus still
        // answers, with the right totals, and no document is behind.
        drop(Store::open_read_write(&path).unwrap());
        let store = Store::open_read_only(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(totals(&store.conn), right);
        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM chunk_fts_docsize"), 3);
        for (query, id) in [("validate", 1), ("validated", 2), ("sessions", 3)] {
            let found: i64 = store
                .conn
                .query_row(
                    "SELECT rowid FROM chunk_fts WHERE chunk_fts MATCH ?1",
                    [query],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, id, "{query}");
        }
        assert_eq!(
            count("SELECT count(*) FROM document WHERE terms_version = 1 AND chunker_version = 1"),
            3
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_9_drops_the_spec_only_full_text_table() {
        let dir = scratch("v8");
        let path = dir.join("corpus.db");
        // A version 8 corpus with a spec section in both full-text tables.
        raw(&path)
            .execute_batch(&format!(
                "{}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '8');
                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
                   VALUES (1, 'rfc', 'document', 'https://x', 'def', 'quote', 'then');
                 INSERT INTO document (id, source_id, path, language, content_hash, text)
                   VALUES (1, 1, 'rfc', 'html', x'01', NULL);
                 INSERT INTO chunk (id, document_id, start_line, end_line, kind, number, text,
                                    terms)
                   VALUES (1, 1, 1, 3, 'section', '1', 'Tokens are validated.',
                           'tokens are validated');
                 INSERT INTO chunk_fts (rowid, symbol, path, terms)
                   VALUES (1, '', 'rfc', 'tokens are validated');
                 INSERT INTO spec_fts (rowid, symbol, path, terms)
                   VALUES (1, '', 'rfc', 'tokens are validated');",
                up_to(8)
            ))
            .unwrap();

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let left: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'spec_fts%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        let found: i64 = store
            .conn
            .query_row(
                "SELECT rowid FROM chunk_fts WHERE chunk_fts MATCH 'validated'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_12_clears_chunk_text_its_document_holds_and_drops_terms() {
        let dir = scratch("v11");
        let path = dir.join("corpus.db");
        // A version 11 corpus: a code file whose function, nested method and
        // window are its lines, plus one chunk whose text isn't, and a spec
        // section with no document text. Every chunk has its full-text row.
        let code = "struct S;\r\nimpl S {\r\n    fn validate() {}\r\n}\n\n// tail\n";
        let chunks: [(i64, i64, u32, u32, &str, &str); 5] = [
            (1, 1, 1, 1, "class", "struct S;"),
            (2, 1, 2, 4, "class", "impl S {\r\n    fn validate() {}\r\n}"),
            (3, 1, 3, 3, "function", "    fn validate() {}"),
            (4, 1, 6, 6, "window", "// tail, as an older chunker cut it"),
            (5, 2, 1, 3, "section", "Tokens are validated."),
        ];
        let conn = raw(&path);
        conn.execute_batch(&format!(
            "{}
             INSERT INTO meta (key, value) VALUES ('schema_version', '11');
             INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
               VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then'),
                      (2, 'rfc', 'document', 'https://x', 'def', 'quote', 'then');
             INSERT INTO document (id, source_id, path, language, content_hash, text)
               VALUES (1, 1, 'src/lib.rs', 'rust', x'01', NULL),
                      (2, 2, 'rfc', NULL, x'02', NULL);",
            up_to(11)
        ))
        .unwrap();
        conn.execute("UPDATE document SET text = ?1 WHERE id = 1", [code])
            .unwrap();
        for (id, document, start, end, kind, text) in chunks {
            conn.execute(
                "INSERT INTO chunk (id, document_id, start_line, end_line, kind, text, terms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, document, start, end, kind, text, text_terms(text)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO chunk_fts (rowid, symbol, path, terms) VALUES (?1, '', '', ?2)",
                rusqlite::params![id, text_terms(text)],
            )
            .unwrap();
        }
        // A wide row on the overflow pages a vacuum gives back.
        let filler = "x".repeat(64 * 1024);
        conn.execute(
            "INSERT INTO chunk (id, document_id, start_line, end_line, kind, text, terms)
             VALUES (6, 1, 1, 1, 'window', ?1, ?1)",
            [&filler],
        )
        .unwrap();
        conn.execute("DELETE FROM chunk WHERE id = 6", []).unwrap();
        let free: i64 = conn
            .query_row("PRAGMA freelist_count", [], |row| row.get(0))
            .unwrap();
        assert!(free > 0);
        drop(conn);

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let own: Vec<(i64, Option<String>)> = {
            let mut stmt = store
                .conn
                .prepare("SELECT id, text FROM chunk ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            own,
            [
                (1, None),
                (2, None),
                (3, None),
                (4, Some(chunks[3].5.to_owned())),
                (5, Some(chunks[4].5.to_owned())),
            ]
        );
        // Every chunk reads back as written.
        for (id, _, _, _, _, text) in chunks {
            let read: String = store
                .conn
                .query_row(
                    "SELECT coalesce(c.text, codecite_lines(d.text, c.start_line, c.end_line))
                     FROM chunk c JOIN document d ON d.id = c.document_id WHERE c.id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(read, text, "chunk {id}");
        }
        let columns = table_columns(&store.conn, "chunk");
        assert!(!columns.iter().any(|c| c == "terms"), "{columns:?}");
        // The full-text rows, the indexes and the delete trigger survive.
        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(
            count("SELECT rowid FROM chunk_fts WHERE chunk_fts MATCH 'validate' AND rowid = 3"),
            3
        );
        assert_eq!(
            count(
                "SELECT count(*) FROM sqlite_master WHERE tbl_name = 'chunk'
                   AND name IN ('chunk_document', 'chunk_parent', 'chunk_symbol', 'chunk_fts_delete')"
            ),
            4
        );
        // The freed pages went back to the file system.
        assert_eq!(count("PRAGMA freelist_count"), 0);
        store.conn.execute("DELETE FROM chunk", []).unwrap();
        assert_eq!(
            count("SELECT count(*) FROM chunk_fts WHERE chunk_fts MATCH 'validate'"),
            0
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_vacuum_keeps_the_open_and_is_retried_on_the_next() {
        let dir = scratch("vacuum");
        let path = dir.join("corpus.db");
        // A version 11 corpus with free pages for a vacuum to give back.
        let conn = raw(&path);
        conn.execute_batch(&format!(
            "{}
             INSERT INTO meta (key, value) VALUES ('schema_version', '11');
             INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
               VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then');
             INSERT INTO document (id, source_id, path, language, content_hash, text)
               VALUES (1, 1, 'src/lib.rs', 'rust', x'01', 'fn a() {{}}');
             INSERT INTO chunk (id, document_id, start_line, end_line, kind, text, terms)
               VALUES (1, 1, 1, 1, 'function', 'fn a() {{}}', 'fn a');",
            up_to(11)
        ))
        .unwrap();
        let filler = "x".repeat(64 * 1024);
        conn.execute(
            "INSERT INTO chunk (id, document_id, start_line, end_line, kind, text, terms)
             VALUES (2, 1, 1, 1, 'window', ?1, ?1)",
            [&filler],
        )
        .unwrap();
        conn.execute("DELETE FROM chunk WHERE id = 2", []).unwrap();
        drop(conn);
        let pending = |store: &Store| -> bool {
            store
                .conn
                .query_row(
                    "SELECT count(*) FROM meta WHERE key = 'vacuum_pending'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                > 0
        };
        let free = |store: &Store| -> i64 {
            store
                .conn
                .query_row("PRAGMA freelist_count", [], |row| row.get(0))
                .unwrap()
        };

        // The vacuum fails, say for want of disk: the migration stays
        // committed, the open succeeds and says why, and the flag stays.
        FAIL_VACUUM.set(true);
        let opened = Store::open_read_write(&path);
        FAIL_VACUUM.set(false);
        let store = opened.unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(store.vacuum_error().is_some_and(|e| e.contains("injected")));
        assert!(pending(&store));
        assert!(free(&store) > 0);
        drop(store);

        // The next open vacuums and clears the flag.
        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.vacuum_error(), None);
        assert!(!pending(&store));
        assert_eq!(free(&store), 0);
        drop(store);

        // A fresh file has nothing to give back, so it never flags one.
        let store = Store::open_read_write(&dir.join("fresh.db")).unwrap();
        assert!(!pending(&store));
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_10_stores_rank_inputs_for_existing_rows() {
        let dir = scratch("v9");
        let path = dir.join("corpus.db");
        // A version 9 corpus with a document of every role and two symbols,
        // one of them with a non-ASCII name SQLite's lower() would not fold.
        raw(&path)
            .execute_batch(&format!(
                "{}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '9');
                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
                   VALUES (1, 'app', 'git', 'https://x', 'abc', 'adapt', 'then'),
                          (2, 'rfc', 'document', 'https://y', 'def', 'quote', 'then'),
                          (3, 'page', 'document', 'https://z', 'ghi', 'quote', 'then');
                 INSERT INTO document (id, source_id, path, language, content_hash, text)
                   VALUES (1, 1, 'src/lib.rs', 'rust', x'01', 'fn a() {{}}'),
                          (2, 1, 'tests/lib.rs', 'rust', x'02', 'fn b() {{}}'),
                          (3, 1, 'examples/demo.rs', 'rust', x'03', 'fn c() {{}}'),
                          (4, 1, 'README.md', 'markdown', x'04', '# App'),
                          (5, 2, 'rfc', 'html', x'05', NULL),
                          (6, 3, 'page', 'html', x'06', 'Sessions');
                 INSERT INTO symbol (id, display_name, kind, document_id)
                   VALUES ('a', 'validate_Token', 'function', 1),
                          ('b', 'ÄrgerHelper', 'function', 2);",
                up_to(9)
            ))
            .unwrap();

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let roles: Vec<(String, f64)> = store
            .conn
            .prepare("SELECT role, role_weight FROM document ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let expected: Vec<(String, f64)> = [
            ("src/lib.rs", "git", true),
            ("tests/lib.rs", "git", true),
            ("examples/demo.rs", "git", true),
            ("README.md", "git", true),
            ("rfc", "document", false),
            ("page", "document", true),
        ]
        .into_iter()
        .map(|(p, kind, has_text)| {
            let role = crate::Role::of_stored(p, kind, has_text);
            (role.as_str().to_owned(), role.weight())
        })
        .collect();
        assert_eq!(roles, expected);
        assert_eq!(
            roles.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>(),
            ["source", "test", "example", "docs", "spec", "docs"]
        );
        let keys: Vec<String> = store
            .conn
            .prepare("SELECT name_key FROM symbol ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(keys, ["validatetoken", "ärgerhelper"]);
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
                MIGRATIONS[0].sql, MIGRATIONS[1].sql
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

    /// A corpus at `version` with one source and two documents.
    fn older_file(path: &Path, version: usize) {
        raw(path)
            .execute_batch(&format!(
                "{}
                 INSERT INTO meta (key, value) VALUES ('schema_version', '{version}'),
                   ('indexed_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now')), ('max_age_days', '30');
                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
                   VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then');
                 INSERT INTO document (id, source_id, path, language, content_hash, text)
                   VALUES (1, 1, 'src/lib.rs', 'rust', x'01', 'fn a() {{}}'),
                          (2, 1, 'README.md', 'markdown', x'02', '# App');",
                up_to(version)
            ))
            .unwrap();
    }

    #[test]
    fn version_13_stamps_stored_documents_and_records_the_tokenizer() {
        let dir = scratch("v12-versions");
        let path = dir.join("corpus.db");
        older_file(&path, 12);

        let store = Store::open_read_write(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let versions: Vec<(u32, u32)> = {
            let mut stmt = store
                .conn
                .prepare("SELECT chunker_version, terms_version FROM document ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(versions, [(1, 1), (1, 1)]);
        // Nothing has to be split again for them.
        let cleared: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM document WHERE content_hash = x''",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cleared, 0);
        let tokenizer: String = store
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'tokenizer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tokenizer, FTS_TOKENIZER);
        let fts: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'chunk_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            fts.contains(&format!("tokenize='{FTS_TOKENIZER}'")),
            "{fts}"
        );
        drop(store);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn application_id(path: &Path) -> i32 {
        raw(path)
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_corpus_carries_codecites_application_id() {
        let dir = scratch("app-id");
        let path = dir.join("corpus.db");
        drop(Store::open_read_write(&path).unwrap());
        assert_eq!(application_id(&path), APPLICATION_ID);
        assert_eq!(&APPLICATION_ID.to_be_bytes(), b"cdct");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_foreign_application_id_is_refused_unchanged() {
        let dir = scratch("foreign-id");
        let path = dir.join("corpus.db");
        // Another program's file that happens to have a `meta` table like
        // codecite's.
        raw(&path)
            .execute_batch(
                "PRAGMA application_id = 42;
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO meta VALUES ('schema_version', '1');",
            )
            .unwrap();
        let before = fs::read(&path).unwrap();

        for err in [
            Store::open_read_only(&path).err().unwrap(),
            Store::open_read_write(&path).err().unwrap(),
        ] {
            assert!(
                matches!(&err, Error::InvalidCorpus { reason, .. }
                    if reason.contains("not a codecite corpus") && reason.contains("0x0000002a")),
                "{err:?}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_codecite_file_from_before_the_application_id_is_accepted_and_stamped() {
        let dir = scratch("no-app-id");
        let path = dir.join("corpus.db");
        drop(Store::open_read_write(&path).unwrap());
        raw(&path)
            .execute_batch("PRAGMA application_id = 0")
            .unwrap();
        assert_eq!(application_id(&path), 0);

        drop(Store::open_read_only(&path).unwrap());
        assert_eq!(application_id(&path), 0);
        drop(Store::open_read_write(&path).unwrap());
        assert_eq!(application_id(&path), APPLICATION_ID);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_opens_a_read_compatible_older_schema_with_a_warning() {
        let dir = scratch("ro-compatible");
        let path = dir.join("corpus.db");
        // Three read-compatible migrations behind: 11, 12 and 13. Its one
        // chunk keeps its text and `terms`, as every chunk did before 12.
        older_file(&path, 10);
        raw(&path)
            .execute_batch(
                "UPDATE document SET role = 'source', role_weight = 1.0;
                 INSERT INTO chunk (id, document_id, start_line, end_line, kind, text, terms)
                   VALUES (1, 1, 1, 1, 'function', 'fn a() {}', 'fn a');
                 INSERT INTO chunk_fts (rowid, symbol, path, terms)
                   VALUES (1, '', 'src lib rs', 'fn a validate');",
            )
            .unwrap();
        let before = fs::read(&path).unwrap();

        let store = Store::open_read_only(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 10);
        let warning = store.schema_warning().unwrap();
        assert!(
            warning.contains(&format!("schema version 10, older than {SCHEMA_VERSION}"))
                && warning.contains("codecite index"),
            "{warning}"
        );
        // It answers queries, a chunk's text read as this build reads it.
        assert!(!store.freshness().unwrap().is_stale());
        let found = store
            .search(&SearchQuery {
                text: "validate",
                language: None,
                path: None,
                only: None,
                limit: 5,
                allow_stale: false,
            })
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].snippet, "fn a() {}");
        drop(store);
        assert_eq!(fs::read(&path).unwrap(), before);

        // An upgraded file has no warning.
        drop(Store::open_read_write(&path).unwrap());
        assert_eq!(Store::open_read_only(&path).unwrap().schema_warning(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_only_refuses_an_older_schema_behind_an_incompatible_migration() {
        let dir = scratch("ro-incompatible");
        let path = dir.join("corpus.db");
        // Version 10 isn't read-compatible, so a version 9 file is refused.
        older_file(&path, 9);

        let err = Store::open_read_only(&path).err().unwrap();
        assert!(
            matches!(&err, Error::InvalidCorpus { reason, .. }
                if reason.contains("schema version 9") && reason.contains("codecite index")),
            "{err:?}"
        );
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
