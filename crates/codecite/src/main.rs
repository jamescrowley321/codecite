//! The `codecite` command-line tool.

use clap::Parser;

/// Command-line arguments. Later milestones add subcommands here.
#[derive(Parser)]
#[command(version, about, arg_required_else_help = true)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
