//! Argument parsing and dispatch: parse, run exactly one command, render its
//! result once, print, exit.
//!
//! M1's surface is `version` and `init`. Commands are added here as their own
//! tasks land -- an unimplemented command is absent from this enum, never a
//! stub, so `telos <not-yet-a-command>` is a clap usage error (exit 2) rather
//! than a command that answers with something meaningless.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use telos_core::error::{ErrorCode, TelosError};

use crate::commands::{self, Ctx};
use crate::envelope::CmdResult;
use crate::render::render;

/// telos: specification-driven development, sealed to the byte.
#[derive(Parser)]
#[command(name = "telos", version, about, long_about = None)]
struct Cli {
    /// Answer with the JSON envelope instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the telos version.
    Version,
    /// Create `telos/` in this git repository and seal it.
    Init,
}

impl Command {
    /// The name this command answers under in the envelope's `command` key.
    fn name(&self) -> &'static str {
        match self {
            Command::Version => "version",
            Command::Init => "init",
        }
    }
}

/// Parses the command line, runs the requested command, and prints its
/// rendered answer.
///
/// Stream choice: JSON goes to stdout whatever happened, so a consumer reads
/// one stream and parses one envelope. Human-mode output splits the usual
/// way -- success on stdout, errors on stderr -- so a shell pipeline is
/// never fed an error message as if it were data.
pub fn run() -> ExitCode {
    let cli = Cli::parse();

    let name = cli.command.name();
    let res = execute(&cli.command);
    let failed = res.is_err();

    let (text, code) = render(name, res, cli.json);
    if cli.json || !failed {
        println!("{text}");
    } else {
        eprintln!("{text}");
    }
    code
}

fn execute(command: &Command) -> CmdResult {
    match command {
        Command::Version => commands::version(),
        Command::Init => commands::init::run(&ctx()?),
    }
}

/// Builds the context commands run in. The current directory is where every
/// discovery (the git repository, the workspace) starts from.
fn ctx() -> Result<Ctx, TelosError> {
    let cwd = std::env::current_dir().map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to read the current directory: {e}"),
        )
    })?;
    Ok(Ctx { cwd })
}
