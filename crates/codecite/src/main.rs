//! The `codecite` command-line tool.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand, ValueEnum};
use codecite_core::{
    DEFAULT_MAX_LINES, DEFAULT_SEARCH_LIMIT, Definitions, MAX_READ_LINES, MAX_SEARCH_LIMIT,
    ReadOptions, ReadResult, Scope, SearchQuery, SearchResult, SearchResults, Store,
    is_corpus_name,
};
use codecite_index::{Clones, Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};

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
        /// Only spec sections or only code. Without it, both.
        #[arg(long, value_enum)]
        only: Option<Only>,
        /// The most results to print, at most 50.
        #[arg(long, default_value_t = DEFAULT_SEARCH_LIMIT, value_parser = clap::value_parser!(u32).range(1..=i64::from(MAX_SEARCH_LIMIT)))]
        limit: u32,
        /// Print the results as JSON: `{"results": [...]}`, as the MCP
        /// `search` tool returns them.
        #[arg(long)]
        json: bool,
        /// Search an index older than its manifest's `max_age_days`.
        #[arg(long)]
        allow_stale: bool,
    },
    /// Print where a symbol is defined: every definition, cited.
    Def {
        /// The corpus to look in.
        corpus: String,
        /// The symbol's name as written, such as `callback`, or dotted with
        /// the names it is inside, such as `Client.callback`.
        symbol: String,
        /// Print the result as JSON, as the MCP `definition` tool returns it.
        #[arg(long)]
        json: bool,
        /// Look in an index older than its manifest's `max_age_days`.
        #[arg(long)]
        allow_stale: bool,
    },
    /// Print the exact text a citation points at, with its pinned citation.
    Read {
        /// The corpus to read from.
        corpus: String,
        /// A code or spec citation, as search prints it, as a spec URL, or
        /// as people write it: `RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`.
        citation: String,
        /// The most lines of text to print, at most 500.
        #[arg(long, default_value_t = DEFAULT_MAX_LINES, value_parser = clap::value_parser!(u32).range(1..=i64::from(MAX_READ_LINES)))]
        max_lines: u32,
        /// Print the result as JSON, as the MCP `read` tool returns it.
        #[arg(long)]
        json: bool,
        /// Read an index older than its manifest's `max_age_days`.
        #[arg(long)]
        allow_stale: bool,
    },
    /// List the corpora with their age, size, source count and staleness,
    /// as JSON.
    Corpora,
    /// Run the MCP server on stdin and stdout.
    Mcp,
}

/// The sources `search --only` keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Only {
    /// Spec sections and steps.
    Spec,
    /// Code from repositories and local directories.
    Code,
}

impl From<Only> for Scope {
    fn from(only: Only) -> Scope {
        match only {
            Only::Spec => Scope::Spec,
            Only::Code => Scope::Code,
        }
    }
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
            only,
            limit,
            json,
            allow_stale,
        } => {
            let query = SearchQuery {
                text: &query,
                language: language.as_deref(),
                path: path.as_deref(),
                only: only.map(Scope::from),
                limit,
                allow_stale,
            };
            search(&corpus, &query, json)
        }
        Command::Def {
            corpus,
            symbol,
            json,
            allow_stale,
        } => def(&corpus, &symbol, allow_stale, json),
        Command::Read {
            corpus,
            citation,
            max_lines,
            json,
            allow_stale,
        } => {
            let options = ReadOptions {
                max_lines,
                allow_stale,
            };
            read(&corpus, &citation, &options, json)
        }
        Command::Corpora => corpora(),
        Command::Mcp => mcp(),
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
    let clones = Clones::new(dirs.cache_dir().join("repos"));
    let summary = index_corpus(
        &mut store,
        &manifest,
        &WalkOptions::default(),
        &fetcher,
        &clones,
    )
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
    to_stdout(|out| write_results(out, results, json))
}

/// `codecite def`: opens the corpus read-only and prints every definition
/// of the symbol, as JSON or in a short human form.
fn def(corpus: &str, symbol: &str, allow_stale: bool, json: bool) -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    let store = Store::open_corpus(dirs.data_dir(), corpus)?;
    let found = store.definition(symbol, allow_stale)?;
    to_stdout(|out| write_definitions(out, &found, json))
}

/// `codecite read`: opens the corpus read-only and prints the cited text,
/// as JSON or in its human form.
fn read(corpus: &str, citation: &str, options: &ReadOptions, json: bool) -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    let store = Store::open_corpus(dirs.data_dir(), corpus)?;
    let result = store.read(citation, options)?;
    to_stdout(|out| write_read(out, &result, json))
}

/// `codecite corpora`: prints the corpora as the MCP `corpora` tool returns
/// them, byte for byte.
fn corpora() -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    let corpora = serde_json::to_value(codecite_core::corpora(dirs.data_dir())?)?;
    to_stdout(|out| write_json(out, corpora))
}

/// `codecite mcp`: serves the corpora in the data directory to one MCP
/// client over stdio until it closes stdin.
fn mcp() -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    let data_dir = dirs.data_dir().to_path_buf();
    // stdout carries the protocol, so this note goes to stderr.
    eprintln!(
        "codecite {}: serving the corpora in {} over stdio",
        env!("CARGO_PKG_VERSION"),
        data_dir.display()
    );
    codecite_mcp::serve_stdio(data_dir)?;
    Ok(())
}

/// Runs `write` on stdout. A reader that stops early, like `| head`, is
/// not a failure.
fn to_stdout(
    write: impl FnOnce(&mut std::io::StdoutLock<'static>) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    match write(&mut std::io::stdout().lock()) {
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        written => written.context("writing to stdout"),
    }
}

/// Writes `value` to `out` as the MCP server's tools write their text.
/// rmcp writes it as `serde_json::Value::to_string`, compact with sorted
/// keys, so going through a `Value` here prints the same bytes.
fn write_json(out: &mut impl std::io::Write, value: serde_json::Value) -> std::io::Result<()> {
    writeln!(out, "{value}")
}

/// Writes a read result to `out`: as JSON, or its `Display` form with
/// control characters escaped line by line.
fn write_read(
    out: &mut impl std::io::Write,
    result: &ReadResult,
    json: bool,
) -> std::io::Result<()> {
    if json {
        write_json(out, serde_json::to_value(result)?)?;
    } else {
        for line in result.to_string().lines() {
            writeln!(out, "{}", escape_controls(line))?;
        }
    }
    out.flush()
}

/// Writes the results to `out`, as `{"results": [...]}` JSON or in the
/// short human form. Unlike `println!`, which panics when stdout
/// is closed, a failed write comes back as an error.
fn write_results(
    out: &mut impl std::io::Write,
    results: Vec<SearchResult>,
    json: bool,
) -> std::io::Result<()> {
    if json {
        write_json(out, serde_json::to_value(SearchResults { results })?)?;
    } else {
        out.write_all(human(&results).as_bytes())?;
    }
    out.flush()
}

/// Writes definitions to `out`, as JSON or in the short human form.
fn write_definitions(
    out: &mut impl std::io::Write,
    found: &Definitions,
    json: bool,
) -> std::io::Result<()> {
    if json {
        write_json(out, serde_json::to_value(found)?)?;
    } else {
        out.write_all(human_definitions(found).as_bytes())?;
    }
    out.flush()
}

/// The short human form of definitions: a header with how many there are
/// and whether the name is ambiguous, then per definition one line with its
/// rank, citation, kind, symbol, role, license class and `indexed_at`, and
/// its snippet's first non-blank line.
fn human_definitions(found: &Definitions) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let n = found.total;
    // `write!` to a `String` can't fail.
    let _ = write!(
        out,
        "{n} definition{} of {}",
        if n == 1 { "" } else { "s" },
        escape_controls(&found.symbol)
    );
    if found.results.len() < n {
        let _ = write!(out, ", the first {}", found.results.len());
    }
    if found.ambiguous {
        out.push_str(" · ambiguous");
    }
    out.push('\n');
    for (rank, d) in (1..).zip(&found.results) {
        let _ = write!(
            out,
            "{rank}. {} · {} {} · {} · {} · indexed {}",
            escape_controls(&abbreviate_commit(&d.citation)),
            escape_controls(&d.kind),
            escape_controls(&d.symbol),
            d.role,
            escape_controls(&d.license_class),
            escape_controls(&d.indexed_at)
        );
        if d.dirty {
            out.push_str(" · dirty");
        }
        out.push('\n');
        if let Some(line) = d.snippet.lines().map(str::trim).find(|l| !l.is_empty()) {
            let _ = writeln!(
                out,
                "   {}",
                escape_controls(&cut(line, HUMAN_SNIPPET_CHARS))
            );
        }
    }
    out
}

/// The most characters of a snippet line the human form shows.
const HUMAN_SNIPPET_CHARS: usize = 120;

/// How many hex characters of a code citation's commit the human form
/// shows. `codecite read` accepts the abbreviation.
const HUMAN_COMMIT_CHARS: usize = 12;

/// The short human form of the results. A header carries the license class
/// and `indexed_at` when every result shares them; otherwise each result's
/// line does. Per result: one line with its rank, citation, kind, symbol
/// and score, then its snippet's first non-blank line.
fn human(results: &[SearchResult]) -> String {
    use std::fmt::Write as _;

    let Some(first) = results.first() else {
        return "no results\n".into();
    };
    let shared = results
        .iter()
        .all(|r| r.license_class == first.license_class && r.indexed_at == first.indexed_at);
    let mut out = String::new();
    // `write!` to a `String` can't fail.
    if shared {
        let n = results.len();
        let _ = writeln!(
            out,
            "{n} result{} · {} · indexed {}",
            if n == 1 { "" } else { "s" },
            escape_controls(&first.license_class),
            escape_controls(&first.indexed_at)
        );
    }
    for (rank, r) in (1..).zip(results) {
        let _ = write!(
            out,
            "{rank}. {} · {}",
            escape_controls(&abbreviate_commit(&r.citation)),
            escape_controls(&r.kind)
        );
        if let Some(symbol) = &r.symbol {
            let _ = write!(out, " {}", escape_controls(symbol));
        }
        let _ = write!(out, " · {:.1}", r.score);
        if r.dirty == Some(true) {
            out.push_str(" · dirty");
        }
        if !shared {
            let _ = write!(
                out,
                " · {} · indexed {}",
                escape_controls(&r.license_class),
                escape_controls(&r.indexed_at)
            );
        }
        out.push('\n');
        if let Some(line) = r.snippet.lines().map(str::trim).find(|l| !l.is_empty()) {
            let _ = writeln!(
                out,
                "   {}",
                escape_controls(&cut(line, HUMAN_SNIPPET_CHARS))
            );
        }
    }
    out
}

/// A code citation, `repo@<40 hex>:path#L…`, with its commit cut to
/// [`HUMAN_COMMIT_CHARS`]. Any other citation comes back unchanged.
fn abbreviate_commit(citation: &str) -> std::borrow::Cow<'_, str> {
    if let Some((repo, rest)) = citation.split_once('@')
        && let Some((commit, path)) = rest.split_once(':')
        && commit.len() == 40
        && commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return format!("{repo}@{}:{path}", &commit[..HUMAN_COMMIT_CHARS]).into();
    }
    citation.into()
}

/// `text` cut to at most `max` characters, with `…` appended when cut.
fn cut(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]).into(),
        None => text.into(),
    }
}

/// Indexed text is untrusted: an escape sequence in it could drive the
/// terminal, such as OSC 52 writing to the clipboard. Control characters
/// other than tab print as Rust escapes like `\u{1b}` instead.
fn escape_controls(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\t' {
                c.escape_unicode().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
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
/// the data directory (`~/.local/share/codecite/<corpus>.db` on Linux), and
/// fetched documents and git sources' clones in the cache directory
/// (`~/.cache/codecite/documents` and `~/.cache/codecite/repos`).
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
            only,
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
        assert_eq!((language, path, only, limit), (None, None, None, 5));
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
            "--only",
            "spec",
            "--limit",
            "3",
            "--json",
            "--allow-stale",
        ])
        .unwrap();
        let Command::Search {
            language,
            path,
            only,
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
        assert_eq!(only.map(Scope::from), Some(Scope::Spec));
        assert_eq!(limit, 3);
        assert!(json && allow_stale);
        // From 1 to the cap the MCP tool has too.
        for limit in ["0", "51", "4000000000"] {
            let args = ["codecite", "search", "identity", "jti", "--limit", limit];
            assert!(Cli::try_parse_from(args).is_err(), "{limit}");
        }
        let most = ["codecite", "search", "identity", "jti", "--limit", "50"];
        assert!(Cli::try_parse_from(most).is_ok());
        let other = ["codecite", "search", "identity", "jti", "--only", "docs"];
        assert!(Cli::try_parse_from(other).is_err());
    }

    #[test]
    fn read_takes_a_corpus_a_citation_and_a_line_cap() {
        assert!(Cli::try_parse_from(["codecite", "read", "identity"]).is_err());
        let cli = Cli::try_parse_from(["codecite", "read", "identity", "RFC 9449 §4.3"]).unwrap();
        let Command::Read {
            corpus,
            citation,
            max_lines,
            json,
            allow_stale,
        } = cli.command
        else {
            panic!("not read");
        };
        assert_eq!(
            (corpus.as_str(), citation.as_str()),
            ("identity", "RFC 9449 §4.3")
        );
        assert_eq!(max_lines, 80);
        assert!(!json && !allow_stale);
        let cli = Cli::try_parse_from([
            "codecite",
            "read",
            "identity",
            "§4.3",
            "--max-lines",
            "5",
            "--json",
            "--allow-stale",
        ])
        .unwrap();
        let Command::Read {
            max_lines,
            json,
            allow_stale,
            ..
        } = cli.command
        else {
            panic!("not read");
        };
        assert_eq!(max_lines, 5);
        assert!(json && allow_stale);
        for lines in ["0", "501"] {
            let args = ["codecite", "read", "identity", "§4.3", "--max-lines", lines];
            assert!(Cli::try_parse_from(args).is_err(), "{lines}");
        }
        let most = ["codecite", "read", "identity", "§4.3", "--max-lines", "500"];
        assert!(Cli::try_parse_from(most).is_ok());
    }

    #[test]
    fn def_takes_a_corpus_and_a_symbol() {
        assert!(Cli::try_parse_from(["codecite", "def", "identity"]).is_err());
        let cli = Cli::try_parse_from(["codecite", "def", "identity", "Client.callback"]).unwrap();
        let Command::Def {
            corpus,
            symbol,
            json,
            allow_stale,
        } = cli.command
        else {
            panic!("not def");
        };
        assert_eq!(
            (corpus.as_str(), symbol.as_str()),
            ("identity", "Client.callback")
        );
        assert!(!json && !allow_stale);
        let args = [
            "codecite",
            "def",
            "identity",
            "x",
            "--json",
            "--allow-stale",
        ];
        let Command::Def {
            json, allow_stale, ..
        } = Cli::try_parse_from(args).unwrap().command
        else {
            panic!("not def");
        };
        assert!(json && allow_stale);
    }

    fn definition(path: &str, role: &'static str) -> codecite_core::Definition {
        codecite_core::Definition {
            citation: format!("app@{SHA}:{path}#L1-L5"),
            symbol: "callback".into(),
            kind: "method".into(),
            role,
            license_class: "adapt".into(),
            indexed_at: "2026-10-20T06:00:00Z".into(),
            dirty: false,
            snippet: "\n  pub fn callback(&self) {\n}".into(),
            providers: vec!["treesitter"],
            confidence: 2,
        }
    }

    #[test]
    fn the_definitions_form_says_how_many_and_whether_ambiguous() {
        let mut test = definition("tests/a.rs", "test");
        test.dirty = true;
        let found = Definitions {
            symbol: "callback".into(),
            ambiguous: true,
            total: 3,
            results: vec![definition("src/a.rs", "source"), test],
        };
        assert_eq!(
            human_definitions(&found),
            "3 definitions of callback, the first 2 · ambiguous
1. app@0123456789ab:src/a.rs#L1-L5 · method callback · source · adapt · indexed 2026-10-20T06:00:00Z
   pub fn callback(&self) {
2. app@0123456789ab:tests/a.rs#L1-L5 · method callback · test · adapt · indexed 2026-10-20T06:00:00Z · dirty
   pub fn callback(&self) {
"
        );
        let mut one = definition("src/a\u{1b}.rs", "source");
        one.snippet = "fn x() {} \u{7}".into();
        let found = Definitions {
            symbol: "Client.callback\u{1b}".into(),
            ambiguous: false,
            total: 1,
            results: vec![one],
        };
        let out = human_definitions(&found);
        assert!(
            out.starts_with("1 definition of Client.callback\\u{1b}\n"),
            "{out}"
        );
        assert!(out.contains("src/a\\u{1b}.rs"), "{out}");
        assert!(out.contains("fn x() {} \\u{7}"), "{out}");
        assert!(!out.chars().any(|c| c.is_control() && c != '\n'), "{out}");
        let err = write_definitions(&mut ClosedPipe, &found, true).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn corpora_and_mcp_take_no_arguments() {
        let cli = Cli::try_parse_from(["codecite", "corpora"]).unwrap();
        assert!(matches!(cli.command, Command::Corpora));
        let cli = Cli::try_parse_from(["codecite", "mcp"]).unwrap();
        assert!(matches!(cli.command, Command::Mcp));
        assert!(Cli::try_parse_from(["codecite", "corpora", "identity"]).is_err());
        assert!(Cli::try_parse_from(["codecite", "mcp", "--stdio"]).is_err());
    }

    #[test]
    fn the_read_form_escapes_control_characters() {
        let result = ReadResult {
            citation: "app@0123456:a.md#L1-L2".into(),
            kind: "code".into(),
            title: None,
            strength: None,
            license_class: "adapt".into(),
            indexed_at: "2026-10-20T06:00:00Z".into(),
            dirty: Some(false),
            text: "# Title\u{1b}]52;c;ZWNobyBoaQ==\u{7}\n\tend".into(),
            truncated: false,
            instruction: None,
        };
        let mut out = Vec::new();
        write_read(&mut out, &result, false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "app@0123456:a.md#L1-L2
code · adapt · indexed 2026-10-20T06:00:00Z

# Title\\u{1b}]52;c;ZWNobyBoaQ==\\u{7}
\tend
"
        );
        let err = write_read(&mut ClosedPipe, &result, true).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn code(path: &str, symbol: Option<&str>, snippet: &str) -> SearchResult {
        SearchResult {
            citation: format!("app@{SHA}:{path}#L1-L5"),
            kind: "function".into(),
            symbol: symbol.map(Into::into),
            role: "source",
            score: 2.46,
            license_class: "adapt".into(),
            indexed_at: "2026-10-20T06:00:00Z".into(),
            dirty: Some(false),
            snippet: snippet.into(),
        }
    }

    #[test]
    fn the_human_form_puts_shared_fields_in_one_header() {
        let mut dirty = code("src/b.rs", None, "\n  \n    let x = 1;  \nmore");
        dirty.dirty = Some(true);
        dirty.score = 1.0;
        assert_eq!(
            human(&[
                code("src/a.rs", Some("validate"), "fn validate() {\n    a();\n}"),
                dirty
            ]),
            "2 results · adapt · indexed 2026-10-20T06:00:00Z
1. app@0123456789ab:src/a.rs#L1-L5 · function validate · 2.5
   fn validate() {
2. app@0123456789ab:src/b.rs#L1-L5 · function · 1.0 · dirty
   let x = 1;
"
        );
        assert_eq!(
            human(&[code("a.rs", None, "x")]),
            "1 result · adapt · indexed 2026-10-20T06:00:00Z
1. app@0123456789ab:a.rs#L1-L5 · function · 2.5
   x
"
        );
        assert_eq!(human(&[]), "no results\n");
    }

    #[test]
    fn the_human_form_puts_differing_fields_on_each_line() {
        let mut quote = code("b.rs", None, "  \n\t\n");
        quote.license_class = "quote".into();
        let mut later = code("c.rs", None, "y");
        later.indexed_at = "2026-10-21T06:00:00Z".into();
        assert_eq!(
            human(&[code("a.rs", None, "x"), quote]),
            "1. app@0123456789ab:a.rs#L1-L5 · function · 2.5 · adapt · indexed 2026-10-20T06:00:00Z
   x
2. app@0123456789ab:b.rs#L1-L5 · function · 2.5 · quote · indexed 2026-10-20T06:00:00Z
"
        );
        let out = human(&[code("a.rs", None, "x"), later]);
        assert!(out.starts_with("1. "), "{out}");
        assert!(out.contains("indexed 2026-10-21T06:00:00Z\n"), "{out}");
    }

    #[test]
    fn the_human_form_keeps_spec_and_other_citations_whole() {
        let spec = SearchResult {
            citation: "rfc9449@sha256:0123456789ab#section-4.3/step-2".into(),
            kind: "step".into(),
            symbol: None,
            role: "spec",
            score: 12.34,
            license_class: "quote".into(),
            indexed_at: "2026-10-20T06:00:00Z".into(),
            dirty: None,
            snippet: "Check the jti.".into(),
        };
        let mut short = code("a.rs", None, "x");
        short.citation = "app@0123456:a.rs#L1-L5".into();
        let mut not_hex = code("a.rs", None, "x");
        not_hex.citation = format!("app@{}g:a.rs#L1-L5", &SHA[..39]);
        let out = human(&[spec, short, not_hex]);
        assert!(
            out.contains(
                "1. rfc9449@sha256:0123456789ab#section-4.3/step-2 · step · 12.3 · quote ·"
            ),
            "{out}"
        );
        assert!(out.contains("2. app@0123456:a.rs#L1-L5 · "), "{out}");
        assert!(
            out.contains(&format!("3. app@{}g:a.rs#L1-L5 · ", &SHA[..39])),
            "{out}"
        );
    }

    #[test]
    fn the_human_form_cuts_a_long_snippet_line() {
        let long = "é".repeat(130);
        let out = human(&[code("a.rs", None, &long)]);
        let line = out.lines().nth(2).unwrap();
        assert_eq!(line, format!("   {}…", "é".repeat(120)));
        let exact = "a".repeat(120);
        let out = human(&[code("a.rs", None, &exact)]);
        assert_eq!(out.lines().nth(2).unwrap(), format!("   {exact}"));
    }

    /// A writer whose every write fails, like stdout after its reader has
    /// gone away.
    struct ClosedPipe;

    impl std::io::Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_stdout_is_an_error_not_a_panic() {
        for json in [true, false] {
            let err = write_results(&mut ClosedPipe, Vec::new(), json).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
        }
        let mut out = Vec::new();
        write_results(&mut out, Vec::new(), false).unwrap();
        assert_eq!(out, b"no results\n");
        let mut out = Vec::new();
        write_results(&mut out, Vec::new(), true).unwrap();
        assert_eq!(out, b"{\"results\":[]}\n");
    }

    #[test]
    fn the_human_form_escapes_control_characters() {
        let result = SearchResult {
            citation: format!("app@{SHA}:a\u{1b}.md#L1-L1"),
            kind: "section".into(),
            symbol: Some("Title\u{7}".into()),
            role: "docs",
            score: 1.0,
            license_class: "adapt".into(),
            indexed_at: "2026-10-20T06:00:00Z".into(),
            dirty: None,
            snippet: "# Title\u{1b}]52;c;ZWNobyBoaQ==\u{7}\tend".into(),
        };
        let out = human(&[result]);
        assert!(
            !out.chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        );
        assert!(out.contains("app@0123456789ab:a\\u{1b}.md"), "{out}");
        assert!(out.contains("Title\\u{7}"), "{out}");
        assert!(
            out.contains("# Title\\u{1b}]52;c;ZWNobyBoaQ==\\u{7}\tend"),
            "{out}"
        );
    }
}
