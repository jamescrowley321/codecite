//! The `codecite` command-line tool.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand};
use codecite_core::Store;
use codecite_index::{Manifest, WalkOptions, index_corpus};

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
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Index { corpus, manifest } => index(&corpus, manifest),
    }
}

/// `codecite index`: reads the manifest, indexes it into the corpus file in
/// the user's data directory, and prints a summary.
fn index(corpus: &str, manifest: Option<PathBuf>) -> anyhow::Result<()> {
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

    let path = corpus_path(corpus)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut store = Store::open_read_write(&path)
        .with_context(|| format!("opening corpus file {}", path.display()))?;
    let summary = index_corpus(&mut store, &manifest, &WalkOptions::default())
        .with_context(|| format!("indexing corpus `{corpus}`"))?;
    println!("indexed corpus `{corpus}` into {}", path.display());
    println!("{summary}");
    Ok(())
}

/// A corpus name becomes a file name, so it may hold only ASCII letters,
/// digits, `-`, `_` and `.`, and may not start with `.`.
fn check_corpus_name(corpus: &str) -> anyhow::Result<()> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if corpus.is_empty() || corpus.starts_with('.') || !corpus.chars().all(allowed) {
        bail!(
            "invalid corpus name {corpus:?}: use ASCII letters, digits, `-`, `_` and `.`, not starting with `.`"
        );
    }
    Ok(())
}

/// Where the corpus file lives: `<data dir>/codecite/<corpus>.db`, e.g.
/// `~/.local/share/codecite/identity.db` on Linux. Never in the repository.
fn corpus_path(corpus: &str) -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "codecite")
        .context("no home directory to keep corpus files in")?;
    Ok(dirs.data_dir().join(format!("{corpus}.db")))
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
    fn corpus_names_are_plain_file_names() {
        for good in ["identity", "oidc-impls", "rfc_9449", "v1.2"] {
            assert!(check_corpus_name(good).is_ok(), "{good}");
        }
        for bad in ["", ".hidden", "..", "../x", "a/b", "a\\b", "名前", "a b"] {
            assert!(check_corpus_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn index_requires_a_corpus() {
        assert!(Cli::try_parse_from(["codecite", "index"]).is_err());
        let cli =
            Cli::try_parse_from(["codecite", "index", "identity", "--manifest", "m.toml"]).unwrap();
        let Command::Index { corpus, manifest } = cli.command;
        assert_eq!(corpus, "identity");
        assert_eq!(manifest, Some(PathBuf::from("m.toml")));
    }
}
