//! Synthetic FTS row rewrites; only invoke on a private copy of a corpus.
use codecite_core::{identifier_terms, text_terms};
use rusqlite::{Connection, params};
use std::{env, error::Error, time::Instant};

fn span(text: &str, start: usize, end: usize) -> Option<&str> {
    if start == 0 || end < start {
        return None;
    }
    let mut starts = text.match_indices('\n').map(|(i, _)| i + 1);
    let from = if start == 1 {
        0
    } else {
        starts.nth(start - 2)?
    };
    if from >= text.len() && start > 1 {
        return None;
    }
    let to = starts.nth(end - start).unwrap_or(text.len());
    let result = &text[from..to];
    let result = result.strip_suffix('\n').unwrap_or(result);
    Some(result.strip_suffix('\r').unwrap_or(result))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 4 {
        return Err("usage: incremental_rewrite PRIVATE_DB PERCENT control|optimize".into());
    }
    let percent: i64 = args[2].parse()?;
    if !(0..=100).contains(&percent) || !["control", "optimize"].contains(&args[3].as_str()) {
        return Err("invalid arguments".into());
    }
    let mut db = Connection::open(&args[1])?;
    db.execute_batch("PRAGMA wal_autocheckpoint=0;")?;
    let total: i64 = db.query_row("SELECT count(*) FROM chunk", [], |r| r.get(0))?;
    let count = total * percent / 100;
    let start = Instant::now();
    let tx = db.transaction()?;
    {
        let mut read = tx.prepare("SELECT c.id, s.display_name, d.path, c.text, d.text, c.start_line, c.end_line FROM chunk c JOIN document d ON d.id=c.document_id LEFT JOIN symbol s ON s.id=c.symbol_id ORDER BY c.id LIMIT ?1")?;
        let mut rows = read.query([count])?;
        let mut remove = tx.prepare("DELETE FROM chunk_fts WHERE rowid=?1")?;
        let mut insert =
            tx.prepare("INSERT INTO chunk_fts(rowid,symbol,path,terms) VALUES(?1,?2,?3,?4)")?;
        let mut seen = 0;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let name: Option<String> = row.get(1)?;
            let path: String = row.get(2)?;
            let own: Option<String> = row.get(3)?;
            let document: Option<String> = row.get(4)?;
            let begin = usize::try_from(row.get::<_, i64>(5)?)?;
            let end = usize::try_from(row.get::<_, i64>(6)?)?;
            let text = match own.as_deref() {
                Some(text) => text,
                None => span(
                    document.as_deref().ok_or("missing document text")?,
                    begin,
                    end,
                )
                .ok_or("invalid span")?,
            };
            remove.execute([id])?;
            insert.execute(params![
                id,
                name.as_deref().map_or_else(String::new, identifier_terms),
                text_terms(&path),
                text_terms(text)
            ])?;
            seen += 1;
        }
        if seen != count {
            return Err("unexpected number of rows".into());
        }
    }
    let rewritten = start.elapsed().as_secs_f64();
    if args[3] == "optimize" {
        tx.execute("INSERT INTO chunk_fts(chunk_fts) VALUES('optimize')", [])?;
    }
    let optimized = start.elapsed().as_secs_f64();
    tx.commit()?;
    println!(
        "{{\"chunks\":{total},\"rewritten\":{count},\"rewrite_seconds\":{rewritten},\"before_commit_seconds\":{optimized},\"transaction_seconds\":{}}}",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::span;

    #[test]
    fn span_preserves_intermediate_line_endings() {
        let text = "one\r\ntwo\r\nthree\n";
        assert_eq!(span(text, 1, 2), Some("one\r\ntwo"));
        assert_eq!(span(text, 3, 3), Some("three"));
        assert_eq!(span(text, 4, 4), None);
        assert_eq!(span(text, 0, 1), None);
        assert_eq!(span(text, 3, 2), None);
    }
}
