//! Isolated path-only FTS5 storage comparison, using the production term splitter.
use codecite_core::text_terms;
use rusqlite::{Connection, OpenFlags, params};
use std::{env, error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 4 {
        return Err("usage: path_postings SOURCE_DB NEW_CHUNK_DB NEW_DOCUMENT_DB".into());
    }
    if Path::new(&args[2]).exists() || Path::new(&args[3]).exists() {
        return Err("output database already exists".into());
    }
    let source = Connection::open_with_flags(&args[1], OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut chunks = Connection::open(&args[2])?;
    let mut documents = Connection::open(&args[3])?;
    for db in [&chunks, &documents] {
        db.execute_batch("CREATE VIRTUAL TABLE paths USING fts5(path, content='', contentless_delete=1, tokenize='unicode61');")?;
    }
    let chunk_tx = chunks.transaction()?;
    let doc_tx = documents.transaction()?;
    let mut query = source.prepare("SELECT d.id, d.path, c.id FROM document d JOIN chunk c ON c.document_id=d.id ORDER BY d.id, c.id")?;
    let mut rows = query.query([])?;
    let mut last = None;
    let (mut doc_count, mut chunk_count) = (0_u64, 0_u64);
    let mut path = String::new();
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if last != Some(id) {
            path = text_terms(&row.get::<_, String>(1)?);
            doc_tx.execute("INSERT INTO paths(rowid,path) VALUES(?1,?2)", params![id, path])?;
            last = Some(id);
            doc_count += 1;
        }
        let chunk: i64 = row.get(2)?;
        chunk_tx.execute("INSERT INTO paths(rowid,path) VALUES(?1,?2)", params![chunk, path])?;
        chunk_count += 1;
    }
    chunk_tx.commit()?;
    doc_tx.commit()?;
    println!("documents={doc_count} chunks={chunk_count}");
    Ok(())
}
