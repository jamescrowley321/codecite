//! `codecite refs` and `codecite graph`: the graph tools over a corpus
//! whose sources name SCIP indexes.

use std::io::Write as _;

use clap::{Args, Subcommand};
use codecite_core::{
    Direction, Graph, GraphQuery, MAX_CONFIDENCE, MAX_GRAPH_DEPTH, References, Store, symbol_name,
};

use super::{
    HUMAN_SNIPPET_CHARS, abbreviate_commit, cut, escape_controls, project_dirs, to_stdout,
    write_json,
};

/// What `codecite refs -h` says it prints.
pub(crate) const REFS_OUTPUT: &str = "\
Output: a header `N reference(s) to <id>`, with ` · <license> · indexed <at>` when
every reference shares them, then one line each:
  <citation> · in <enclosing> · <access> · <line>";

/// What `codecite refs --help` adds.
pub(crate) const REFS_HELP: &str = "Examples:
  codecite refs myrepo validate
  codecite refs myrepo Client.validate --json
  codecite refs myrepo 'rust-analyzer cargo app 0.1.0 client/impl#[Client]validate().'
Needs a SCIP index on the symbol's source (`scip` in the manifest).
Output: a header `N reference(s) to <id>`, with ` · <license> · indexed <at>` when
every reference shares them, then one line each:
  <citation> · in <enclosing> · <access> · <line>";

/// What `codecite graph -h` says it prints.
pub(crate) const GRAPH_OUTPUT: &str = "\
Output: a header `N caller(s) of <id> within <depth> call(s)` (or `callee(s)`), then
one line per symbol, nearest first:
  <distance> · <citation> · <kind> <name> · calls <via> at <call site> · <license>
(`called by <via>` for callees)";

/// What `codecite graph --help` adds.
pub(crate) const GRAPH_HELP: &str = "Examples:
  codecite graph callers myrepo check
  codecite graph callees myrepo Client.validate --depth 2 --json
Needs a SCIP index on the symbol's source (`scip` in the manifest).
Output: a header `N caller(s) of <id> within <depth> call(s)` (or `callee(s)`), then
one line per symbol, nearest first:
  <distance> · <citation> · <kind> <name> · calls <via> at <call site> · <license>
(`called by <via>` for callees)";

/// `codecite refs`'s arguments.
#[derive(Args)]
pub(crate) struct RefsArgs {
    /// The corpus to look in.
    corpus: String,
    /// The symbol: its name as written (`validate`), dotted with the names
    /// it is inside (`Client.validate`), or its SCIP ID.
    symbol: String,
    /// Leave out references below this confidence: 3 exact, 2 extracted,
    /// 1 inferred, 0 ambiguous.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=i64::from(MAX_CONFIDENCE)))]
    min_confidence: u8,
    /// Print the result as JSON, as the MCP `references` tool returns it.
    #[arg(long)]
    json: bool,
    /// Look in an index older than its manifest's `max_age_days`.
    #[arg(long)]
    allow_stale: bool,
}

/// `codecite graph`'s walks.
#[derive(Subcommand)]
pub(crate) enum GraphCommand {
    /// Print the symbols that call a symbol, out to `--depth` calls.
    #[command(after_long_help = "Examples:
  codecite graph callers myrepo check
  codecite graph callers myrepo Client.validate --depth 3 --json
Output: a header `N caller(s) of <id> within <depth> call(s)`, then one line per
symbol, nearest first:
  <distance> · <citation> · <kind> <name> · calls <via> at <call site> · <license>")]
    Callers(WalkArgs),
    /// Print the symbols a symbol calls, out to `--depth` calls.
    #[command(after_long_help = "Examples:
  codecite graph callees myrepo login
  codecite graph callees myrepo login --depth 2 --json
Output: a header `N callee(s) of <id> within <depth> call(s)`, then one line per
symbol, nearest first:
  <distance> · <citation> · <kind> <name> · called by <via> at <call site> · <license>")]
    Callees(WalkArgs),
}

/// A graph walk's arguments.
#[derive(Args)]
pub(crate) struct WalkArgs {
    /// The corpus to look in.
    corpus: String,
    /// The symbol: its name as written (`validate`), dotted with the names
    /// it is inside (`Client.validate`), or its SCIP ID.
    symbol: String,
    /// How many calls away to go: 1 for direct callers or callees, at most
    /// 10.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=i64::from(MAX_GRAPH_DEPTH)))]
    depth: u32,
    /// Follow only calls of at least this confidence: 3 exact, 2
    /// extracted, 1 inferred, 0 ambiguous.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=i64::from(MAX_CONFIDENCE)))]
    min_confidence: u8,
    /// Print the result as JSON, as the MCP `callers` and `callees` tools
    /// return it.
    #[arg(long)]
    json: bool,
    /// Look in an index older than its manifest's `max_age_days`.
    #[arg(long)]
    allow_stale: bool,
}

/// `codecite refs`: opens the corpus read-only and prints every line that
/// references the symbol, as JSON or in a short human form.
pub(crate) fn refs(args: &RefsArgs) -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    let store = Store::open_corpus(dirs.data_dir(), &args.corpus)?;
    let found = store.references(&args.symbol, args.min_confidence, args.allow_stale)?;
    to_stdout(|out| {
        if args.json {
            write_json(out, serde_json::to_value(&found)?)?;
        } else {
            out.write_all(human_references(&found).as_bytes())?;
        }
        out.flush()
    })
}

/// `codecite graph callers|callees`: opens the corpus read-only and prints
/// the symbols the walk reaches, as JSON or in a short human form.
pub(crate) fn walk(command: &GraphCommand) -> anyhow::Result<()> {
    let (args, direction) = match command {
        GraphCommand::Callers(args) => (args, Direction::Callers),
        GraphCommand::Callees(args) => (args, Direction::Callees),
    };
    let dirs = project_dirs()?;
    let store = Store::open_corpus(dirs.data_dir(), &args.corpus)?;
    let graph = store.graph(&GraphQuery {
        symbol: &args.symbol,
        direction,
        depth: args.depth,
        min_confidence: args.min_confidence,
        allow_stale: args.allow_stale,
    })?;
    to_stdout(|out| {
        if args.json {
            write_json(out, serde_json::to_value(&graph)?)?;
        } else {
            out.write_all(human_graph(&graph).as_bytes())?;
        }
        out.flush()
    })
}

/// A symbol ID's short name for the human forms: its last descriptor's.
fn short(id: &str) -> String {
    symbol_name(id).unwrap_or_else(|| id.to_owned())
}

/// The short human form of references: a header with how many there are
/// and, when every reference shares them, the license class and
/// `indexed_at`; then per reference one line: citation, the definition it
/// is in, what it does, any `dirty` mark, the license and `indexed_at` if
/// not shared, and the line itself.
fn human_references(found: &References) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let n = found.total;
    let shared = found.results.first().filter(|first| {
        found
            .results
            .iter()
            .all(|r| r.license_class == first.license_class && r.indexed_at == first.indexed_at)
    });
    // `write!` to a `String` can't fail.
    let _ = write!(
        out,
        "{n} reference{} to {}",
        if n == 1 { "" } else { "s" },
        escape_controls(&found.id)
    );
    if found.results.len() < n {
        let _ = write!(out, ", the first {}", found.results.len());
    }
    if let Some(first) = shared {
        let _ = write!(
            out,
            " · {} · indexed {}",
            escape_controls(&first.license_class),
            escape_controls(&first.indexed_at)
        );
    }
    out.push('\n');
    for r in &found.results {
        out.push_str(&escape_controls(&abbreviate_commit(&r.citation)));
        if let Some(enclosing) = &r.enclosing {
            let _ = write!(out, " · in {}", escape_controls(&short(enclosing)));
        }
        if !r.access.is_empty() {
            let _ = write!(out, " · {}", r.access.join(","));
        }
        if r.dirty {
            out.push_str(" · dirty");
        }
        if shared.is_none() {
            let _ = write!(
                out,
                " · {} · indexed {}",
                escape_controls(&r.license_class),
                escape_controls(&r.indexed_at)
            );
        }
        let _ = write!(
            out,
            " · {}",
            escape_controls(&cut(r.text.trim(), HUMAN_SNIPPET_CHARS))
        );
        out.push('\n');
    }
    out
}

/// The short human form of a graph walk: a header with how many symbols it
/// reached, then per symbol one line: its distance, its definition's
/// citation (or its ID when the corpus doesn't define it), its kind and
/// name, the symbol one step nearer and the line of that call, and its
/// license class when the corpus defines it.
fn human_graph(graph: &Graph) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let n = graph.total;
    let (noun, link) = match graph.direction {
        Direction::Callers => ("caller", "calls"),
        Direction::Callees => ("callee", "called by"),
    };
    let _ = write!(
        out,
        "{n} {noun}{} of {} within {} call{}",
        if n == 1 { "" } else { "s" },
        escape_controls(&graph.id),
        graph.depth,
        if graph.depth == 1 { "" } else { "s" }
    );
    if graph.results.len() < n {
        let _ = write!(out, ", the first {}", graph.results.len());
    }
    out.push('\n');
    for node in &graph.results {
        let place = node
            .citation
            .as_deref()
            .map_or_else(|| node.id.clone(), |c| abbreviate_commit(c).into_owned());
        let _ = write!(out, "{} · {} · ", node.distance, escape_controls(&place));
        if let Some(kind) = &node.kind {
            let _ = write!(out, "{} ", escape_controls(kind));
        }
        let _ = write!(
            out,
            "{} · {link} {}",
            escape_controls(&node.name),
            escape_controls(&short(&node.via))
        );
        if let Some(site) = &node.call_site {
            let _ = write!(out, " at {}", escape_controls(&abbreviate_commit(site)));
        }
        if let Some(class) = &node.license_class {
            let _ = write!(out, " · {}", escape_controls(class));
        }
        if let Some(source) = &node.defined_in {
            let _ = write!(out, " · defined in {}", escape_controls(source));
        }
        out.push('\n');
    }
    out
}
