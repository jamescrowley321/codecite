//! `codecite-bench`: runs labelled queries through `codecite search` and
//! reports Recall@1, Recall@10 and MRR for each query class, with every miss.

mod answer;
mod assist;
mod cost;
mod grep;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};
use clap::Parser;
use serde::Deserialize;

use answer::{Answer, Citation};

/// How many results each query asks for. Recall@10 and MRR count hits up
/// to this rank.
const DEPTH: u32 = 10;

/// Command-line arguments.
#[derive(Parser)]
#[command(about)]
struct Cli {
    /// The labelled queries.
    #[arg(long, default_value = "evals/retrieval/queries.toml")]
    queries: PathBuf,
    /// The `codecite` binary to measure [default: the one next to this binary].
    #[arg(long)]
    codecite: Option<PathBuf>,
    /// Only queries whose class starts with this, such as `spec`.
    #[arg(long)]
    class: Option<String>,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// Rank with `git grep` over this repository instead of running
    /// `codecite search`: the baseline the history benchmark compares with.
    #[arg(long, requires = "commit")]
    grep: Option<PathBuf>,
    /// The commit `--grep` searches.
    #[arg(long, requires = "grep")]
    commit: Option<String>,
    /// Rank with no search at all: the files the other queries' PRs changed
    /// most often. A query-blind floor that both engines must beat.
    #[arg(long, conflicts_with = "grep")]
    popular: bool,
    /// Measure what a query costs instead of how well it ranks: output
    /// bytes and time of `codecite search` and of `rg` over this checkout
    /// of the corpus's tree.
    #[arg(long, conflicts_with_all = ["grep", "popular"])]
    cost: Option<PathBuf>,
    /// The `rg` program `--cost` compares with.
    #[arg(long, default_value = "rg", requires = "cost")]
    rg: PathBuf,
    /// Measure `codecite assist` instead: its latency on a grep call made
    /// of each query's text, and of the query's first word as a symbol.
    #[arg(long, conflicts_with_all = ["grep", "popular", "cost"])]
    assist: bool,
}

/// The queries file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryFile {
    /// The corpus every query searches.
    corpus: String,
    /// The queries.
    query: Vec<Query>,
}

/// One labelled query.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    id: String,
    class: String,
    text: String,
    answers: Vec<String>,
    /// Where the query comes from. Not used by the runner.
    #[allow(dead_code)]
    source: Option<String>,
}

/// One query's outcome.
#[derive(serde::Serialize)]
struct Outcome {
    id: String,
    class: String,
    text: String,
    /// The rank of the first result that is an answer, from 1.
    rank: Option<usize>,
    /// The first result's citation.
    top: Option<String>,
}

/// The scores of a set of queries.
#[derive(serde::Serialize)]
struct Scores {
    queries: usize,
    recall_at_1: f64,
    recall_at_10: f64,
    mrr: f64,
}

impl Scores {
    fn of<'a>(outcomes: impl Iterator<Item = &'a Outcome>) -> Scores {
        let ranks: Vec<Option<usize>> = outcomes.map(|o| o.rank).collect();
        let n = ranks.len().max(1) as f64;
        let share =
            |k: usize| ranks.iter().filter(|r| r.is_some_and(|r| r <= k)).count() as f64 / n;
        Scores {
            queries: ranks.len(),
            recall_at_1: share(1),
            recall_at_10: share(DEPTH as usize),
            mrr: ranks
                .iter()
                .flatten()
                .fold(0.0, |sum, &r| sum + 1.0 / r as f64)
                / n,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let file: QueryFile = toml::from_str(
        &std::fs::read_to_string(&cli.queries)
            .with_context(|| format!("reading {}", cli.queries.display()))?,
    )
    .with_context(|| format!("parsing {}", cli.queries.display()))?;
    check(&file)?;
    let codecite = match cli.codecite.clone() {
        Some(path) => path,
        None => std::env::current_exe()?
            .with_file_name(format!("codecite{}", std::env::consts::EXE_SUFFIX)),
    };

    let queries: Vec<&Query> = file
        .query
        .iter()
        .filter(|q| {
            cli.class
                .as_ref()
                .is_none_or(|c| q.class.starts_with(c.as_str()))
        })
        .collect();
    if let Some(checkout) = &cli.cost {
        let report = cost::cost(&queries, &file.corpus, &codecite, &cli.rg, checkout)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if cli.assist {
        let report = assist::assist(&queries, &file.corpus, &codecite)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let mut outcomes = Vec::new();
    for query in &file.query {
        if cli
            .class
            .as_ref()
            .is_some_and(|c| !query.class.starts_with(c.as_str()))
        {
            continue;
        }
        let answers = query
            .answers
            .iter()
            .map(|a| Answer::parse(a))
            .collect::<anyhow::Result<Vec<_>>>()?;
        // Several chunks of one file would fill a file-level top 10 with
        // fewer files than grep's, so a file-labelled query ranks distinct
        // files: each file's first result, from a deeper search.
        let by_file = answers.iter().all(|a| matches!(a, Answer::File { .. }));
        let citations = match (&cli.grep, &cli.commit) {
            _ if cli.popular => Ok(popular_rank(&file.query, query, DEPTH)),
            (Some(repo), Some(commit)) => grep::grep_rank(repo, commit, &query.text, DEPTH),
            _ if by_file => search(&codecite, &file.corpus, &query.text, FILE_SEARCH_DEPTH)
                .map(|c| first_per_file(c, DEPTH)),
            _ => search(&codecite, &file.corpus, &query.text, DEPTH),
        }
        .with_context(|| format!("query `{}`", query.id))?;
        let rank = citations
            .iter()
            .position(|c| {
                let c = Citation::parse(c);
                answers.iter().any(|a| a.matches(&c))
            })
            .map(|i| i + 1);
        outcomes.push(Outcome {
            id: query.id.clone(),
            class: query.class.clone(),
            text: query.text.clone(),
            rank,
            top: citations.into_iter().next(),
        });
    }

    let mut by_class: BTreeMap<&str, Vec<&Outcome>> = BTreeMap::new();
    for outcome in &outcomes {
        by_class.entry(&outcome.class).or_default().push(outcome);
    }
    let classes: BTreeMap<&str, Scores> = by_class
        .iter()
        .map(|(class, os)| (*class, Scores::of(os.iter().copied())))
        .collect();
    let groups: BTreeMap<&str, Scores> = ["spec", "code", "history"]
        .into_iter()
        .map(|g| {
            (
                g,
                Scores::of(outcomes.iter().filter(|o| o.class.starts_with(g))),
            )
        })
        .filter(|(_, s)| s.queries > 0)
        .collect();
    let all = Scores::of(outcomes.iter());

    let engine = engine(&cli);
    if cli.json {
        let report = serde_json::json!({
            "engine": engine, "classes": classes, "groups": groups, "all": all, "outcomes": outcomes,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("Engine: {engine}\n");
    println!("| Class | Queries | Recall@1 | Recall@10 | MRR |");
    println!("| --- | ---: | ---: | ---: | ---: |");
    let row = |name: &str, s: &Scores| {
        println!(
            "| {name} | {} | {:.2} | {:.2} | {:.2} |",
            s.queries, s.recall_at_1, s.recall_at_10, s.mrr
        );
    };
    for (class, scores) in &classes {
        row(class, scores);
    }
    for (group, scores) in &groups {
        row(&format!("**{group}**"), scores);
    }
    row("**all**", &all);
    let misses: Vec<&Outcome> = outcomes
        .iter()
        .filter(|o| o.rank.is_none_or(|r| r > 1))
        .collect();
    if !misses.is_empty() {
        println!("\nNot first (rank, or - when outside the top {DEPTH}):\n");
        for o in misses {
            let rank = o.rank.map_or("-".to_owned(), |r| r.to_string());
            let top = o.top.as_deref().unwrap_or("no results");
            println!("- {rank:>2}  {}  \"{}\"  top: {top}", o.id, o.text);
        }
    }
    Ok(())
}

/// The engine that ranked a run: `codecite`, the `git grep` baseline or the
/// query-blind popular-files floor.
fn engine(cli: &Cli) -> &'static str {
    if cli.cost.is_some() {
        "cost"
    } else if cli.assist {
        "assist"
    } else if cli.popular {
        "popular"
    } else if cli.grep.is_some() {
        "grep"
    } else {
        "codecite"
    }
}

/// Rejects a queries file with a duplicate id, an empty field or a bad
/// label, before any search runs.
fn check(file: &QueryFile) -> anyhow::Result<()> {
    let mut ids = HashSet::new();
    for query in &file.query {
        if !ids.insert(query.id.as_str()) {
            bail!("query id `{}` is used twice", query.id);
        }
        if query.text.trim().is_empty() || query.class.is_empty() || query.answers.is_empty() {
            bail!("query `{}` needs a class, text and answers", query.id);
        }
        for answer in &query.answers {
            let parsed = Answer::parse(answer).with_context(|| format!("query `{}`", query.id))?;
            // Only mined history queries label whole files, so elsewhere a
            // bare path is a spec or code label that lost its fragment.
            if matches!(parsed, Answer::File { .. }) && !query.class.starts_with("history") {
                bail!(
                    "query `{}`: answer `{answer}` is a bare file path, which only \
                     history queries take; add `#L<line>` or `#section-<number>`",
                    query.id
                );
            }
        }
    }
    Ok(())
}

/// How many results a file-labelled query asks for, so that
/// [`first_per_file`] still has [`DEPTH`] distinct files to keep.
const FILE_SEARCH_DEPTH: u32 = 50;

/// Each code file's first citation, and every other citation, in order,
/// at most `depth`.
fn first_per_file(citations: Vec<String>, depth: u32) -> Vec<String> {
    let mut seen = HashSet::new();
    citations
        .into_iter()
        .filter(|c| match Citation::parse(c) {
            Citation::Code { path, .. } => seen.insert(path),
            _ => true,
        })
        .take(depth as usize)
        .collect()
}

/// The files the other PRs' queries name as answers, most PRs first, then by
/// path, at most `depth`, cited `popular@0:<path>#L1`. A PR is a query's
/// `source`, so its title and issue queries count once, and never for each
/// other.
fn popular_rank(queries: &[Query], query: &Query, depth: u32) -> Vec<String> {
    let mut prs: BTreeMap<&str, HashSet<&str>> = BTreeMap::new();
    for other in queries {
        let pr = other.source.as_deref().unwrap_or(&other.id);
        if pr == query.source.as_deref().unwrap_or(&query.id) {
            continue;
        }
        for answer in &other.answers {
            prs.entry(answer.as_str()).or_default().insert(pr);
        }
    }
    let mut ranked: Vec<(&str, usize)> = prs.into_iter().map(|(p, s)| (p, s.len())).collect();
    // The map is in path order, and the sort is stable.
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    ranked
        .into_iter()
        .take(depth as usize)
        .map(|(path, _)| format!("popular@0:{path}#L1"))
        .collect()
}

/// The citations `codecite search` returns for `text`, best first, at most
/// `limit`. A query with no words to search for has no results, as grep's
/// would, rather than failing the run.
fn search(codecite: &Path, corpus: &str, text: &str, limit: u32) -> anyhow::Result<Vec<String>> {
    let output = Command::new(codecite)
        .args(search_args(corpus, text, limit))
        .output()
        .with_context(|| format!("running {}", codecite.display()))?;
    if !output.status.success() && has_no_words(&output.stderr) {
        return Ok(Vec::new());
    }
    if !output.status.success() {
        bail!(
            "codecite search failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    citations(&output.stdout).context("parsing codecite search --json")
}

/// `codecite search`'s arguments. A mined query can start with `--`, so it
/// follows `--` and is never read as a flag.
fn search_args(corpus: &str, text: &str, limit: u32) -> Vec<String> {
    let limit = limit.to_string();
    [
        "search",
        "--json",
        "--allow-stale",
        "--limit",
        &limit,
        "--",
        corpus,
        text,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Whether `codecite search` failed because the query has no words. The
/// message is codecite's own, pinned by `crates/codecite/tests/search.rs`.
fn has_no_words(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("no words to search for")
}

/// The citations in what `codecite search --json` prints,
/// `{"results": [...]}`, best first.
fn citations(stdout: &[u8]) -> serde_json::Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Printed {
        results: Vec<Result>,
    }
    #[derive(Deserialize)]
    struct Result {
        citation: String,
    }
    let printed: Printed = serde_json::from_slice(stdout)?;
    Ok(printed.results.into_iter().map(|r| r.citation).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citations_come_from_the_results_object() {
        let printed = br#"{"results":[{"citation":"a@0123456:x.rs#L1","score":2.0},{"citation":"rfc9449@sha256:1a2b3c4d5e6f#section-4.3"}]}"#;
        assert_eq!(
            citations(printed).unwrap(),
            [
                "a@0123456:x.rs#L1",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3"
            ]
        );
        assert!(citations(br#"{"results":[]}"#).unwrap().is_empty());
        // The bare array `search --json` printed before is refused, not
        // read as no results.
        assert!(citations(br#"[{"citation":"a@0123456:x.rs#L1"}]"#).is_err());
    }

    #[test]
    fn file_level_ranking_keeps_each_files_first_result() {
        let cited = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let ranked = cited(&[
            "r@0123abcd:a.py#L1-L9",
            "r@0123abcd:a.py#L20-L30",
            "r@0123abcd:b.py#L1",
            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
            "r@0123abcd:c.py#L3",
        ]);
        assert_eq!(
            first_per_file(ranked, 3),
            cited(&[
                "r@0123abcd:a.py#L1-L9",
                "r@0123abcd:b.py#L1",
                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
            ])
        );
    }

    #[test]
    fn query_text_that_looks_like_a_flag_is_searched_as_text() {
        let args = search_args("c", "--path=src fix the parser", 10);
        let dashes = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(&args[dashes + 1..], ["c", "--path=src fix the parser"]);
    }

    #[test]
    fn only_a_query_with_no_words_counts_as_no_results() {
        assert!(has_no_words(
            b"error: the query has no words to search for\n"
        ));
        assert!(!has_no_words(b"error: no corpus named c\n"));
    }

    #[test]
    fn each_engine_names_itself_in_the_report() {
        let engine_of = |args: &[&str]| {
            let cli =
                Cli::parse_from(std::iter::once("codecite-bench").chain(args.iter().copied()));
            engine(&cli)
        };
        assert_eq!(engine_of(&[]), "codecite");
        assert_eq!(engine_of(&["--grep", "r", "--commit", "abc1234"]), "grep");
        assert_eq!(engine_of(&["--popular"]), "popular");
        assert_eq!(engine_of(&["--cost", "/c"]), "cost");
        assert_eq!(engine_of(&["--assist"]), "assist");
    }

    #[test]
    fn rg_is_only_taken_with_cost() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("codecite-bench").chain(args.iter().copied()))
        };
        assert!(parse(&["--rg", "/bin/rg"]).is_err());
        let cli = parse(&["--cost", "/c", "--rg", "/bin/rg"]).unwrap();
        assert_eq!(cli.rg, Path::new("/bin/rg"));
        assert_eq!(parse(&["--cost", "/c"]).unwrap().rg, Path::new("rg"));
    }

    #[test]
    fn only_history_queries_take_a_bare_file_answer() {
        let file = |class: &str, answer: &str| QueryFile {
            corpus: "c".into(),
            query: vec![Query {
                class: class.into(),
                ..query("q", "s", &[answer])
            }],
        };
        assert!(check(&file("history-title", "src/a.py")).is_ok());
        // A spec label that lost its `#section-<n>` is still an error.
        let err = check(&file("spec-keyword", "rfc9449.txt")).unwrap_err();
        assert!(format!("{err:#}").contains("history"), "{err:#}");
        assert!(check(&file("code-symbol", "src/a.py#L3")).is_ok());
    }

    fn query(id: &str, source: &str, answers: &[&str]) -> Query {
        Query {
            id: id.into(),
            class: "history-title".into(),
            text: "t".into(),
            answers: answers.iter().map(|a| a.to_string()).collect(),
            source: Some(source.into()),
        }
    }

    #[test]
    fn the_popular_baseline_ranks_files_by_how_many_other_prs_changed_them() {
        let queries = vec![
            query("q1-title", "pr1", &["docs/design.md", "src/a.rs"]),
            query("q1-issue", "pr1", &["docs/design.md", "src/a.rs"]),
            query("q2-title", "pr2", &["docs/design.md", "src/b.rs"]),
            query("q3-title", "pr3", &["docs/design.md", "src/b.rs"]),
        ];
        // For pr1: design.md is in pr2 and pr3, b.rs in both; a.rs only in
        // pr1 itself, so it is left out.
        assert_eq!(
            popular_rank(&queries, &queries[0], 10),
            ["popular@0:docs/design.md#L1", "popular@0:src/b.rs#L1"]
        );
        assert_eq!(
            popular_rank(&queries, &queries[2], 1),
            ["popular@0:docs/design.md#L1"]
        );
    }
}
