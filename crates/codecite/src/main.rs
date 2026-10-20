//! The `codecite` command-line tool.

use clap::Parser;

/// Command-line arguments. Later milestones add subcommands here.
#[derive(Parser)]
#[command(version, about, arg_required_else_help = true)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Context as _;

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
}
