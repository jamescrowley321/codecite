//! Experiment-only diagnostic for #287: where a labelled answer sits in
//! codecite's *untruncated* candidate list, and whether the index holds it
//! at all. It is not part of the product and adds no public flag.
//!
//! It calls the production `Store::search` and `Store::definition` on a
//! read-only store, so term generation, scope, scoring and tie-breaks are
//! the shipped ones. The only difference from `codecite search` is the
//! limit: `u32::MAX` instead of the CLI's cap of 50, so a missing answer is
//! never inferred from a capped list. A second read-only SQLite connection
//! reads the index rows that cover each target line.
//!
//! Usage (compiled with rustc against the locked release rlibs; see README):
//!
//! ```text
//! candidates <db> search <query> <path-glob or -> <target>...
//! candidates <db> def <symbol> - <target>...
//! ```
//!
//! A target is `path:line`. Output is tab-separated, one record per line:
//!
//! ```text
//! total   <candidates>
//! top     <rank> <path> <start> <end> <kind> <role> <symbol> <score>
//! target  <path:line> <best rank or 0> <start> <end> <kind> <role> <symbol> <score>
//! doc     <path:line> <present 0|1> <language> <role> <chunks> <chunk kinds> <symbols covering the line>
//! error   <message>
//! ```

use std::path::Path;

use codecite_core::{SearchQuery, Store};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

const TOP: usize = 10;

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// `repo@sha:path#L10-L42` -> (path, 10, 42); `#L10` -> (path, 10, 10).
fn parse_citation(citation: &str) -> Option<(String, u32, u32)> {
    let (_, rest) = citation.split_once('@')?;
    let (_, rest) = rest.split_once(':')?;
    let (path, lines) = rest.rsplit_once("#L")?;
    let (start, end) = match lines.split_once("-L") {
        Some((a, b)) => (a.parse().ok()?, b.parse().ok()?),
        None => {
            let n = lines.parse().ok()?;
            (n, n)
        }
    };
    Some((path.to_owned(), start, end))
}

fn parse_target(target: &str) -> Option<(String, u32)> {
    let (path, line) = target.rsplit_once(':')?;
    Some((path.to_owned(), line.parse().ok()?))
}

fn describe_doc(conn: &Connection, path: &str, line: u32) -> rusqlite::Result<String> {
    let doc: Option<(i64, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT d.id, d.language, d.role FROM document d
             JOIN source s ON s.id = d.source_id
             WHERE s.is_current AND d.path = ?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((id, language, role)) = doc else {
        return Ok("0\t-\t-\t0\t-\t-".to_owned());
    };
    let mut chunks = conn.prepare(
        "SELECT c.kind, coalesce(sym.display_name, '') FROM chunk c
         LEFT JOIN symbol sym ON sym.id = c.symbol_id
         WHERE c.document_id = ?1 AND c.start_line <= ?2 AND c.end_line >= ?2
         ORDER BY c.end_line - c.start_line",
    )?;
    let kinds: Vec<String> = chunks
        .query_map(rusqlite::params![id, line], |r| {
            Ok(format!(
                "{}:{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut symbols = conn.prepare(
        "SELECT display_name, coalesce(kind, ''), start_line, end_line FROM symbol
         WHERE document_id = ?1 AND start_line <= ?2 AND end_line >= ?2
         ORDER BY end_line - start_line",
    )?;
    let syms: Vec<String> = symbols
        .query_map(rusqlite::params![id, line], |r| {
            Ok(format!(
                "{}|{}|{}-{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(format!(
        "1\t{}\t{}\t{}\t{}\t{}",
        language.unwrap_or_else(|| "-".into()),
        role.unwrap_or_else(|| "-".into()),
        kinds.len(),
        clean(&kinds.join(";")),
        clean(&syms.join(";"))
    ))
}

struct Row {
    path: String,
    start: u32,
    end: u32,
    kind: String,
    role: String,
    symbol: String,
    score: f64,
}

fn run(args: &[String]) -> Result<(), String> {
    let [db, mode, text, path_glob, targets @ ..] = args else {
        return Err("usage: candidates <db> search|def <text> <glob|-> <path:line>...".into());
    };
    let store = Store::open_read_only(Path::new(db)).map_err(|e| e.to_string())?;
    let rows: Vec<Row> = match mode.as_str() {
        "search" => {
            let query = SearchQuery {
                text,
                language: None,
                path: (path_glob != "-").then_some(path_glob.as_str()),
                only: None,
                limit: u32::MAX,
                allow_stale: true,
            };
            store
                .search(&query)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter_map(|r| {
                    let (path, start, end) = parse_citation(&r.citation)?;
                    Some(Row {
                        path,
                        start,
                        end,
                        kind: r.kind,
                        role: r.role.to_owned(),
                        symbol: r.symbol.unwrap_or_default(),
                        score: r.score,
                    })
                })
                .collect()
        }
        "def" => match store.definition(text, true) {
            Ok(defs) => {
                println!("deftotal\t{}", defs.total);
                defs.results
                    .into_iter()
                    .filter_map(|d| {
                        let (path, start, end) = parse_citation(&d.citation)?;
                        Some(Row {
                            path,
                            start,
                            end,
                            kind: d.kind,
                            role: d.role.to_owned(),
                            symbol: d.symbol,
                            score: 0.0,
                        })
                    })
                    .collect()
            }
            Err(e) => {
                println!("deferror\t{}", clean(&e.to_string()));
                Vec::new()
            }
        },
        other => return Err(format!("unknown mode {other}")),
    };
    println!("total\t{}", rows.len());
    for (i, r) in rows.iter().take(TOP).enumerate() {
        println!(
            "top\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            i + 1,
            clean(&r.path),
            r.start,
            r.end,
            r.kind,
            r.role,
            clean(&r.symbol),
            r.score
        );
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    for target in targets {
        let (path, line) = parse_target(target).ok_or(format!("bad target {target}"))?;
        let best = rows
            .iter()
            .enumerate()
            .find(|(_, r)| r.path == path && r.start <= line && line <= r.end);
        match best {
            Some((i, r)) => println!(
                "target\t{target}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                i + 1,
                r.start,
                r.end,
                r.kind,
                r.role,
                clean(&r.symbol),
                r.score
            ),
            None => println!("target\t{target}\t0\t-\t-\t-\t-\t-\t-"),
        }
        // Every candidate chunk that covers the line, so the definition's
        // own chunk is told apart from an enclosing one.
        for (i, r) in rows.iter().enumerate() {
            if r.path == path && r.start <= line && line <= r.end {
                println!(
                    "cover\t{target}\t{}\t{}\t{}\t{}\t{}\t{}",
                    i + 1,
                    r.start,
                    r.end,
                    r.kind,
                    clean(&r.symbol),
                    r.score
                );
            }
        }
        // What ranks above the best covering chunk, by role and kind, and
        // how long those chunks are.
        if let Some((best_rank, _)) = best {
            let above = &rows[..best_rank];
            let mut roles = std::collections::BTreeMap::<&str, usize>::new();
            let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
            let mut spans: Vec<u32> = Vec::new();
            for r in above {
                *roles.entry(r.role.as_str()).or_default() += 1;
                *kinds.entry(r.kind.as_str()).or_default() += 1;
                spans.push(r.end - r.start + 1);
            }
            spans.sort_unstable();
            let fmt = |m: &std::collections::BTreeMap<&str, usize>| {
                m.iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            println!(
                "above\t{target}\t{}\t{}\t{}\t{}",
                fmt(&roles),
                fmt(&kinds),
                above.iter().filter(|r| r.path == path).count(),
                spans.get(spans.len() / 2).copied().unwrap_or(0)
            );
        }
        let doc = describe_doc(&conn, &path, line).map_err(|e| e.to_string())?;
        println!("doc\t{target}\t{doc}");
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        println!("error\t{}", clean(&e));
        std::process::exit(2);
    }
}
