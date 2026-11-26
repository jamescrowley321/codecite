//! codecite's side of the CodeGraph kernel spike: walk a checkout exactly as
//! `codecite index` does (`walk_local`), chunk every kept file with
//! codecite's own `Chunker`, and write the definitions and the file list.
//!
//!   codecite-extract <checkout> <out dir>
//!
//! Writes `<out>/files.tsv` (path, size), `<out>/codecite.jsonl` (one line
//! per definition chunk with a symbol) and `<out>/codecite-stats.json`
//! (files per language, chunk wall time). No store, no network.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use codecite_index::{Chunker, Language, LicenseClass, LocalSource, WalkOptions, walk_local};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, root, out] = args.as_slice() else {
        eprintln!("usage: codecite-extract <checkout> <out dir>");
        std::process::exit(2);
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let source = LocalSource {
        name: Some("spike".into()),
        path: PathBuf::from(root),
        license: None,
        license_class: LicenseClass::Adapt,
        include: vec![],
        exclude: vec![".codegraph/**".into()],
        scip: None,
        scip_sha256: None,
    };
    let t_walk = Instant::now();
    let tree = walk_local(&source, &WalkOptions::default()).expect("walk");
    let walk = t_walk.elapsed();

    let mut files = BufWriter::new(File::create(out.join("files.tsv")).unwrap());
    let mut defs = BufWriter::new(File::create(out.join("codecite.jsonl")).unwrap());
    let mut chunker = Chunker::new().expect("chunker");
    let mut per_lang: BTreeMap<&str, (u64, u64)> = BTreeMap::new(); // files, definitions
    let mut chunk_time = Duration::ZERO;
    let mut non_utf8 = 0u64;
    let t_all = Instant::now();
    for f in &tree.files {
        writeln!(files, "{}\t{}", f.path, f.size).unwrap();
        let Ok(bytes) = std::fs::read(tree.root.join(&f.path)) else { continue };
        let Ok(text) = String::from_utf8(bytes) else {
            non_utf8 += 1;
            continue;
        };
        let Some(lang) = Language::detect(&f.path, &text) else { continue };
        if matches!(lang, Language::Markdown | Language::Html) {
            continue;
        }
        let t = Instant::now();
        let chunks = chunker.chunk(&f.path, &text);
        chunk_time += t.elapsed();
        let e = per_lang.entry(lang.name()).or_default();
        e.0 += 1;
        for c in chunks {
            let Some(sym) = c.symbol else { continue };
            e.1 += 1;
            let line = json!({
                "p": f.path, "lang": lang.name(), "k": sym.kind.as_str(), "ck": c.kind.as_str(),
                "n": sym.name, "s": c.start_line, "e": c.end_line,
            });
            writeln!(defs, "{line}").unwrap();
        }
    }
    let stats = json!({
        "walk_seconds": walk.as_secs_f64(),
        "chunk_seconds": chunk_time.as_secs_f64(),
        "read_and_chunk_seconds": t_all.elapsed().as_secs_f64(),
        "kept_files": tree.files.len(),
        "skipped_files": tree.skipped.len(),
        "non_utf8": non_utf8,
        "per_language": per_lang.iter().map(|(k, (f, d))| (k.to_string(), json!({"files": f, "definitions": d}))).collect::<serde_json::Map<_, _>>(),
    });
    std::fs::write(out.join("codecite-stats.json"), serde_json::to_string_pretty(&stats).unwrap()).unwrap();
}
