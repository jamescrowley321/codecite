//! The `codecite` command-line tool.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand};
use codecite_core::{SearchQuery, SearchResult, Store, is_corpus_name};
use codecite_index::{Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};

/// Command-line arguments.
#[derive(Parser)]
#[command(version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The subcommands. Later milestones add more here.
#[derive(Subcommand)]
enum Command {
    /// Index a corpus's sources into its corpus file.
    Index {
        /// The corpus to index, as named in its manifest.
        corpus: String,
        /// The manifest to read [default: corpora/<CORPUS>.toml].
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Download each document source and print its sha256 without
        /// checking it against the manifest or indexing anything.
        #[arg(long)]
        print_hashes: bool,
    },
    /// Search a corpus and print ranked, cited results.
    Search {
        /// The corpus to search.
        corpus: String,
        /// What to search for: words, identifiers or both.
        query: String,
        /// Only files in this language, such as `rust` or `markdown`.
        #[arg(long)]
        language: Option<String>,
        /// Only files whose path matches this glob; `*` matches `/` too.
        #[arg(long)]
        path: Option<String>,
        /// The most results to print.
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
        limit: u32,
        /// Print the results as a JSON array.
        #[arg(long)]
        json: bool,
        /// Search an index older than its manifest's `max_age_days`.
        #[arg(long)]
        allow_stale: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Index {
            corpus,
            manifest,
            print_hashes,
        } => index(&corpus, manifest, print_hashes),
        Command::Search {
            corpus,
            query,
            language,
            path,
            limit,
            json,
            allow_stale,
        } => {
            let query = SearchQuery {
                text: &query,
                language: language.as_deref(),
                path: path.as_deref(),
                limit,
                allow_stale,
            };
            search(&corpus, &query, json)
        }
    }
}

/// `codecite index`: reads the manifest, indexes it into the corpus file in
/// the user's data directory, and prints a summary. With `print_hashes`, it
/// prints each document's hash instead.
fn index(corpus: &str, manifest: Option<PathBuf>, print_hashes: bool) -> anyhow::Result<()> {
    check_corpus_name(corpus)?;
    let manifest_path =
        manifest.unwrap_or_else(|| Path::new("corpora").join(format!("{corpus}.toml")));
    let manifest = Manifest::load(&manifest_path)
        .with_context(|| format!("reading manifest {}", manifest_path.display()))?;
    if manifest.name != corpus {
        bail!(
            "manifest {} is for corpus `{}`, not `{corpus}`",
            manifest_path.display(),
            manifest.name
        );
    }

    let dirs = project_dirs()?;
    let fetcher = Fetcher::new(dirs.cache_dir().join("documents"));
    if print_hashes {
        return print_document_hashes(&manifest, &fetcher);
    }

    let path = dirs.data_dir().join(format!("{corpus}.db"));
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut store = Store::open_read_write(&path)
        .with_context(|| format!("opening corpus file {}", path.display()))?;
    let summary = index_corpus(&mut store, &manifest, &WalkOptions::default(), &fetcher)
        .with_context(|| format!("indexing corpus `{corpus}`"))?;
    println!("indexed corpus `{corpus}` into {}", path.display());
    println!("{summary}");
    Ok(())
}

/// A corpus name becomes a file name, so it may hold only ASCII letters,
/// digits, `-`, `_` and `.`, and may not start with `.`.
fn check_corpus_name(corpus: &str) -> anyhow::Result<()> {
    if !is_corpus_name(corpus) {
        return Err(codecite_core::Error::InvalidCorpusName {
            name: corpus.into(),
        }
        .into());
    }
    Ok(())
}

/// `codecite search`: opens the corpus read-only and prints its results,
/// as JSON or in a short human form.
fn search(corpus: &str, query: &SearchQuery<'_>, json: bool) -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    let store = Store::open_corpus(dirs.data_dir(), corpus)?;
    let results = store.search(query)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        print!("{}", human(&results));
    }
    Ok(())
}

/// How many lines of each snippet the human form shows.
const HUMAN_SNIPPET_LINES: usize = 3;

/// The short human form of the results: per result, its rank and citation,
/// then what it is, then the start of its snippet.
fn human(results: &[SearchResult]) -> String {
    use std::fmt::Write as _;

    if results.is_empty() {
        return "no results\n".into();
    }
    let mut out = String::new();
    for (rank, r) in (1..).zip(results) {
        // `write!` to a `String` can't fail.
        let _ = writeln!(out, "{rank}. {}", r.citation);
        let mut about = r.kind.clone();
        if let Some(symbol) = &r.symbol {
            let _ = write!(about, " {symbol}");
        }
        let _ = write!(
            about,
            " · score {:.2} · {} · indexed {}",
            r.score, r.license_class, r.indexed_at
        );
        if r.dirty == Some(true) {
            about.push_str(" · dirty");
        }
        let _ = writeln!(out, "   {about}");
        for line in r.snippet.lines().take(HUMAN_SNIPPET_LINES) {
            let _ = writeln!(out, "     {line}");
        }
    }
    out
}

/// `codecite index --print-hashes`: downloads each document source that is
/// not `excluded` and prints `<sha256>  <name>  <url>`, plus `(pinned)` when
/// the hash is the one the manifest pins, so the owner can pin a new or
/// changed document on purpose.
fn print_document_hashes(manifest: &Manifest, fetcher: &Fetcher) -> anyhow::Result<()> {
    for source in &manifest.sources {
        let Source::Document(doc) = source else {
            continue;
        };
        if doc.license_class == LicenseClass::Excluded {
            continue;
        }
        let hash = fetcher.fetch_hash(doc)?;
        let pinned = if hash == doc.sha256 { "  (pinned)" } else { "" };
        println!("{hash}  {}  {}{pinned}", doc.name, doc.url);
    }
    Ok(())
}

/// Where codecite keeps its files, never in a repository: corpus files in
/// the data directory (`~/.local/share/codecite/<corpus>.db` on Linux) and
/// fetched documents in the cache directory (`~/.cache/codecite/documents`).
fn project_dirs() -> anyhow::Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "codecite")
        .context("no home directory to keep corpus files in")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> anyhow::Result<()> {
        let err = codecite_core::Error::from(std::io::Error::other("disk on fire"));
        Err(err).context("opening corpus /tmp/corpus")
    }

    #[test]
    fn context_chain_prints_each_cause_once() {
        let err = open().unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "opening corpus /tmp/corpus: I/O error: disk on fire"
        );
        assert!(err.downcast_ref::<codecite_core::Error>().is_some());
    }

    #[test]
    fn a_bad_corpus_name_is_a_typed_error() {
        assert!(check_corpus_name("identity").is_ok());
        let err = check_corpus_name("../x").unwrap_err();
        assert!(matches!(
            err.downcast_ref(),
            Some(codecite_core::Error::InvalidCorpusName { .. })
        ));
    }

    #[test]
    fn index_requires_a_corpus() {
        assert!(Cli::try_parse_from(["codecite", "index"]).is_err());
        let cli =
            Cli::try_parse_from(["codecite", "index", "identity", "--manifest", "m.toml"]).unwrap();
        let Command::Index {
            corpus,
            manifest,
            print_hashes,
        } = cli.command
        else {
            panic!("not index");
        };
        assert_eq!(corpus, "identity");
        assert_eq!(manifest, Some(PathBuf::from("m.toml")));
        assert!(!print_hashes);
        let cli = Cli::try_parse_from(["codecite", "index", "identity", "--print-hashes"]).unwrap();
        let Command::Index { print_hashes, .. } = cli.command else {
            panic!("not index");
        };
        assert!(print_hashes);
    }

    #[test]
    fn search_takes_a_corpus_a_query_and_filters() {
        assert!(Cli::try_parse_from(["codecite", "search", "identity"]).is_err());
        let cli =
            Cli::try_parse_from(["codecite", "search", "identity", "validate token"]).unwrap();
        let Command::Search {
            corpus,
            query,
            language,
            path,
            limit,
            json,
            allow_stale,
        } = cli.command
        else {
            panic!("not search");
        };
        assert_eq!(
            (corpus.as_str(), query.as_str()),
            ("identity", "validate token")
        );
        assert_eq!((language, path, limit), (None, None, 10));
        assert!(!json && !allow_stale);
        let cli = Cli::try_parse_from([
            "codecite",
            "search",
            "identity",
            "jti",
            "--language",
            "rust",
            "--path",
            "src/*",
            "--limit",
            "3",
            "--json",
            "--allow-stale",
        ])
        .unwrap();
        let Command::Search {
            language,
            path,
            limit,
            json,
            allow_stale,
            ..
        } = cli.command
        else {
            panic!("not search");
        };
        assert_eq!(language.as_deref(), Some("rust"));
        assert_eq!(path.as_deref(), Some("src/*"));
        assert_eq!(limit, 3);
        assert!(json && allow_stale);
        let zero = ["codecite", "search", "identity", "jti", "--limit", "0"];
        assert!(Cli::try_parse_from(zero).is_err());
    }

    #[test]
    fn the_human_form_shows_rank_citation_and_snippet() {
        let result = SearchResult {
            citation: "app@0123456:src/lib.rs#L1-L5".into(),
            kind: "function".into(),
            symbol: Some("validate".into()),
            score: 2.5,
            license_class: "adapt".into(),
            indexed_at: "2026-10-20T06:00:00Z".into(),
            dirty: Some(true),
            snippet: "fn validate() {\n    a();\n    b();\n    c();\n}".into(),
        };
        assert_eq!(
            human(&[result]),
            "1. app@0123456:src/lib.rs#L1-L5
   function validate · score 2.50 · adapt · indexed 2026-10-20T06:00:00Z · dirty
     fn validate() {
         a();
         b();
"
        );
        assert_eq!(human(&[]), "no results\n");
    }
}
