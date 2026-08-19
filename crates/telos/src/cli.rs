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

use crate::commands::{self, Ctx, list::EntityType};
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
    /// Report the project's state against its seal and its spec coverage.
    Status,
    /// Parse the spec and check its integrity.
    Check {
        /// Also require the project to be sealed and unmodified.
        #[arg(long)]
        sealed: bool,
    },
    /// Print one entity's canonical block and its relations.
    Show {
        /// A typed id (`INT-0042`, `SCN-0107`, `CON-0003`) or a bare notion
        /// name (`Invoice`).
        target: String,
    },
    /// List every entity of one kind, sorted by its natural key.
    List {
        /// Which kind of entity to list.
        kind: EntityType,
    },
}

impl Command {
    /// The name this command answers under in the envelope's `command` key.
    fn name(&self) -> &'static str {
        match self {
            Command::Version => "version",
            Command::Init => "init",
            Command::Status => "status",
            Command::Check { .. } => "check",
            Command::Show { .. } => "show",
            Command::List { .. } => "list",
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
        Command::Status => commands::status::run(&ctx()?),
        Command::Check { sealed } => commands::check::run(&ctx()?, *sealed),
        Command::Show { target } => commands::show::run(&ctx()?, target),
        Command::List { kind } => commands::list::run(&ctx()?, *kind),
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
