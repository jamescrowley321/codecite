//! `codecite-bench`: runs labelled queries through `codecite search` and
//! reports Recall@1, Recall@10 and MRR for each query class, with every miss.

mod answer;
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
    let codecite = match cli.codecite {
        Some(path) => path,
        None => std::env::current_exe()?
            .with_file_name(format!("codecite{}", std::env::consts::EXE_SUFFIX)),
    };

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
        let citations = match (&cli.grep, &cli.commit) {
            (Some(repo), Some(commit)) => grep::grep_rank(repo, commit, &query.text, DEPTH),
            _ => search(&codecite, &file.corpus, &query.text),
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

    let engine = if cli.grep.is_some() {
        "grep"
    } else {
        "codecite"
    };
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
            Answer::parse(answer).with_context(|| format!("query `{}`", query.id))?;
        }
    }
    Ok(())
}

/// The citations `codecite search` returns for `text`, best first.
fn search(codecite: &Path, corpus: &str, text: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new(codecite)
        .args(["search", corpus, text, "--json", "--allow-stale", "--limit"])
        .arg(DEPTH.to_string())
        .output()
        .with_context(|| format!("running {}", codecite.display()))?;
    if !output.status.success() {
        bail!(
            "codecite search failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    citations(&output.stdout).context("parsing codecite search --json")
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
}
