//! CodeGraph's side of the spike: run the extraction kernel (through the
//! local fork's napi-free `codegraph_kernel::api`) over the files codecite's
//! walk kept, and write every node it extracts.
//!
//!   kernel-extract <checkout> <files.tsv> <out dir> [--threads N]
//!
//! A file is routed by extension: codecite's own extension table first (so
//! the parity comparison sees the same files), then CodeGraph's table for the
//! languages only the kernel has. Writes `<out>/kernel.jsonl` (one line per
//! node, file nodes excluded), `<out>/kernel-files.jsonl` (one line per file
//! that did not extract, with the reason) and `<out>/kernel-stats.json`.
//!
//! The kernel runs bare: no CodeGraph TS-side `preParse` (C/C++ macro
//! blanking, C# #237), no `.h` C++/Objective-C sniffing, no wasm fallback. A
//! file the kernel defers (a parse tree with errors) or times out on yields
//! nothing, which is what a Rust caller of the kernel alone would get.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use codegraph_kernel::api::{self, ExtractError};
use serde_json::json;

/// Extension → (kernel language, true when codecite also handles it).
fn route(path: &str) -> Option<(&'static str, bool)> {
    let file = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = file.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    Some(match ext.to_ascii_lowercase().as_str() {
        // codecite's table (codecite-core/src/language.rs)
        "rs" => ("rust", true),
        "py" | "pyi" => ("python", true),
        "ts" | "mts" | "cts" => ("typescript", true),
        "tsx" => ("tsx", true),
        "js" | "mjs" | "cjs" => ("javascript", true),
        "jsx" => ("jsx", true),
        "go" => ("go", true),
        "c" | "h" => ("c", true),
        "cs" => ("csharp", true),
        "java" => ("java", true),
        // kernel-only languages (CodeGraph src/extraction/grammars.ts)
        "pyw" => ("python", false),
        "xsjs" | "xsjslib" => ("javascript", false),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => ("cpp", false),
        "rb" | "rake" => ("ruby", false),
        "php" => ("php", false),
        "swift" => ("swift", false),
        "kt" | "kts" => ("kotlin", false),
        "dart" => ("dart", false),
        "r" => ("r", false),
        "scala" | "sc" => ("scala", false),
        "lua" => ("lua", false),
        "luau" => ("luau", false),
        _ => return None,
    })
}

fn clip(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|s| s.chars().take(400).collect())
}

#[derive(Default, Clone)]
struct LangStats {
    files: u64,
    ok: u64,
    defer: u64,
    timeout: u64,
    other: u64,
    non_utf8: u64,
    nodes: u64,
    extract_ns: u128,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: kernel-extract <checkout> <files.tsv> <out dir> [--threads N]");
        std::process::exit(2);
    }
    let root = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[3]);
    let threads: usize = args
        .iter()
        .position(|a| a == "--threads")
        .map(|i| args[i + 1].parse().unwrap())
        .unwrap_or(1);
    std::fs::create_dir_all(&out).unwrap();
    let files: Vec<(String, &'static str, bool)> = BufReader::new(File::open(&args[2]).unwrap())
        .lines()
        .filter_map(|l| {
            let l = l.unwrap();
            let path = l.split('\t').next().unwrap().to_string();
            route(&path).map(|(lang, shared)| (path, lang, shared))
        })
        .collect();
    let files = Arc::new(files);
    let next = Arc::new(AtomicUsize::new(0));
    let stats: Arc<Mutex<BTreeMap<String, LangStats>>> = Arc::default();
    let (tx, rx) = mpsc::sync_channel::<(String, String)>(4096);

    let writer = {
        let out = out.clone();
        std::thread::spawn(move || {
            let mut nodes = BufWriter::new(File::create(out.join("kernel.jsonl")).unwrap());
            let mut bad = BufWriter::new(File::create(out.join("kernel-files.jsonl")).unwrap());
            for (which, line) in rx {
                let w = if which == "n" { &mut nodes } else { &mut bad };
                writeln!(w, "{line}").unwrap();
            }
        })
    };

    let t0 = Instant::now();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (files, next, stats, tx, root) =
                (files.clone(), next.clone(), stats.clone(), tx.clone(), root.clone());
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    let mut local: BTreeMap<String, LangStats> = BTreeMap::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((path, lang, shared)) = files.get(i) else { break };
                        let key = format!("{lang}{}", if *shared { "" } else { "*" });
                        let st = local.entry(key).or_default();
                        st.files += 1;
                        let Ok(text) = std::fs::read(root.join(path)).map(String::from_utf8) else { continue };
                        let Ok(text) = text else {
                            st.non_utf8 += 1;
                            continue;
                        };
                        let t = Instant::now();
                        let r = api::extract(path, &text, lang, Some(10_000));
                        st.extract_ns += t.elapsed().as_nanos();
                        match r {
                            Ok(x) => {
                                st.ok += 1;
                                for n in x.nodes.iter().filter(|n| n.kind != "file") {
                                    st.nodes += 1;
                                    let line = json!({
                                        "p": path, "lang": lang, "k": n.kind, "n": n.name,
                                        "qn": n.qualified_name, "s": n.start_line, "e": n.end_line,
                                        "doc": clip(&n.docstring), "sig": clip(&n.signature), "ret": clip(&n.return_type),
                                        "doclen": n.docstring.as_ref().map(|d| d.len()),
                                    });
                                    tx.send(("n".into(), line.to_string())).unwrap();
                                }
                            }
                            Err(e) => {
                                let (why, msg) = match e {
                                    ExtractError::Defer(m) => { st.defer += 1; ("defer", m) }
                                    ExtractError::Timeout(m) => { st.timeout += 1; ("timeout", m) }
                                    ExtractError::Other(m) => { st.other += 1; ("other", m) }
                                };
                                let line = json!({"p": path, "lang": lang, "why": why, "msg": msg});
                                tx.send(("f".into(), line.to_string())).unwrap();
                            }
                        }
                    }
                    let mut g = stats.lock().unwrap();
                    for (k, v) in local {
                        let e = g.entry(k).or_default();
                        e.files += v.files;
                        e.ok += v.ok;
                        e.defer += v.defer;
                        e.timeout += v.timeout;
                        e.other += v.other;
                        e.non_utf8 += v.non_utf8;
                        e.nodes += v.nodes;
                        e.extract_ns += v.extract_ns;
                    }
                })
                .unwrap()
        })
        .collect();
    drop(tx);
    for w in workers {
        w.join().unwrap();
    }
    writer.join().unwrap();
    let wall = t0.elapsed();
    let g = stats.lock().unwrap();
    let per: serde_json::Map<String, serde_json::Value> = g
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                json!({"files": v.files, "ok": v.ok, "defer": v.defer, "timeout": v.timeout,
                       "other": v.other, "non_utf8": v.non_utf8, "nodes": v.nodes,
                       "extract_seconds": Duration::from_nanos(v.extract_ns as u64).as_secs_f64()}),
            )
        })
        .collect();
    let stats = json!({"threads": threads, "wall_seconds": wall.as_secs_f64(), "per_language": per,
                       "note": "a language key ending in * is kernel-only (codecite has no grammar for it)"});
    std::fs::write(out.join("kernel-stats.json"), serde_json::to_string_pretty(&stats).unwrap()).unwrap();
}
