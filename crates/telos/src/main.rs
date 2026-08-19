use clap::Parser;

/// telos command-line interface.
#[derive(Parser)]
#[command(version)]
struct Cli;

fn main() {
    Cli::parse();
}
