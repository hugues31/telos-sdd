//! Argument parsing and dispatch: parse, run exactly one command, render its
//! result once, print, exit.
//!
//! An unimplemented command is absent from this enum, never a
//! stub, so `telos <not-yet-a-command>` is a clap usage error (exit 2) rather
//! than a command that answers with something meaningless.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use telos_core::error::{ErrorCode, TelosError};

use crate::ci::CiProvider;
use crate::commands::{
    self, Ctx, agents::AgentHost, change::ChangeCommand, list::EntityType, mutate::EntityKind,
    query::QueryCommand, rebuild::RebuildCommand,
};
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
    Init {
        /// Install integrations for these comma-delimited agent hosts.
        #[arg(long, value_delimiter = ',', value_parser = parse_agent_host)]
        agents: Vec<AgentHost>,
        /// Install a sealed-state CI workflow.
        #[arg(long, value_enum)]
        ci: Option<CiProvider>,
    },
    /// Internal entry point invoked by generated synchronous host hooks.
    #[command(hide = true)]
    AgentGuard {
        #[arg(long, value_parser = parse_agent_host)]
        host: AgentHost,
    },
    /// Print the project's canonical configuration, or stage a complete edit.
    Config {
        /// The change to stage into (`CHG-0001`).
        #[arg(long)]
        change: Option<String>,
    },
    /// Print the bounded-context map, or stage its complete replacement.
    Map {
        /// The change to stage into (`CHG-0001`). The map DSL is read on stdin.
        #[arg(long)]
        change: Option<String>,
    },
    /// Report the project's state against its seal and its spec coverage.
    Status,
    /// View Telos documentation locally or export a sealed static snapshot.
    View {
        /// Loopback server port.
        #[arg(long, default_value_t = 3000, conflicts_with = "export")]
        port: u16,
        /// Write a self-contained static site to this new directory.
        #[arg(long, value_name = "DIR")]
        export: Option<PathBuf>,
        /// Open the generated view in the default web browser.
        #[arg(long)]
        open: bool,
    },
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
        /// Restrict results to one bounded context.
        #[arg(long)]
        context: Option<String>,
        /// Restrict results to one capability (qualified, or paired with --context).
        #[arg(long)]
        capability: Option<String>,
    },
    /// Answer with entities of one kind, filtered.
    Query {
        #[command(subcommand)]
        query: QueryCommand,
    },
    /// Report everything a change to one entity would ripple into.
    Impact {
        /// A typed id (`INT-0042`, `SCN-0107`, `CON-0003`) or a bare notion
        /// name (`Invoice`).
        target: String,
    },
    /// Print the bounded work pack of one intent: its scenarios, notions,
    /// applicable constraints, bindings and 1-hop neighbours -- the unit of
    /// agent context.
    Pack {
        /// An intent id (`INT-0042`) or a scenario id (`SCN-0107`), which
        /// resolves to the intent that owns it.
        target: String,
    },
    /// Plan a reconstruction or measure its real scenario progress.
    Rebuild {
        #[command(subcommand)]
        rebuild: RebuildCommand,
    },
    /// Open, list, diff, approve, reconcile and abandon changes.
    Change {
        #[command(subcommand)]
        change: ChangeCommand,
    },
    /// Stage the creation of an entity into an open change (payload on stdin).
    Add {
        /// What kind of entity to add.
        kind: EntityKind,
        /// The change to stage into (`CHG-0001`).
        #[arg(long)]
        change: String,
    },
    /// Stage a modification of an entity into an open change (payload on stdin).
    Edit {
        /// What kind of entity to edit.
        kind: EntityKind,
        /// The entity's natural key (`Invoice`, `INT-0042`, `CON-0003`).
        key: String,
        /// The change to stage into (`CHG-0001`).
        #[arg(long)]
        change: String,
    },
    /// Move an owned entity to another domain owner.
    Move {
        /// A typed entity selector (`INT-0042`, `CON-0003`, `NOT:billing/Invoice`).
        target: String,
        /// Destination owner (`context`, `context/capability`, or `project`).
        #[arg(long)]
        to: String,
        /// The change to stage into (`CHG-0001`).
        #[arg(long)]
        change: String,
    },
    /// Capture the project's drift as staged operations of a change.
    Adopt {
        /// Append the operations to this change (`CHG-0001`) instead of
        /// opening a new one.
        #[arg(long, value_name = "CHG-NNNN")]
        into: Option<String>,
        /// Require the exact drift token displayed by `telos status`.
        #[arg(long, value_name = "SHA256")]
        expected_state: Option<String>,
    },
    /// Restore every drifted path to the state the seal records.
    Revert {
        /// Require the exact drift token displayed by `telos status`.
        #[arg(long, value_name = "SHA256")]
        expected_state: Option<String>,
    },
    /// Run a scenario's test and seal the verdict as a witness in the change
    /// that owns it.
    Test {
        /// The scenario to witness (`SCN-0108`).
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        scenario: Option<String>,
        /// Witness every scenario the open, approved changes owe one for.
        /// Takes no scenario id.
        #[arg(long)]
        all: bool,
        /// Run this file's tests instead of discovering one by the
        /// `scn_NNNN` naming convention.
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
    },
    /// Record that a code file implements an intent, journalled into the
    /// change that owns it.
    Bind {
        /// The code file that implements the intent (`src/billing/invoice.rs`).
        path: String,
        /// The intent it implements (`INT-0042`).
        intent: String,
    },
    /// Stage the deletion of an entity into an open change.
    Remove {
        /// What kind of entity to remove.
        kind: EntityKind,
        /// The entity's natural key (`Invoice`, `INT-0042`, `CON-0003`).
        key: String,
        /// The change to stage into (`CHG-0001`).
        #[arg(long)]
        change: String,
    },
}

impl Command {
    /// The name this command answers under in the envelope's `command` key.
    fn name(&self) -> &'static str {
        match self {
            Command::Version => "version",
            Command::Init { .. } => "init",
            Command::AgentGuard { .. } => "agent-guard",
            Command::Config { .. } => "config",
            Command::Map { .. } => "map",
            Command::Status => "status",
            Command::View { .. } => "view",
            Command::Check { .. } => "check",
            Command::Show { .. } => "show",
            Command::List { .. } => "list",
            Command::Query { .. } => "query",
            Command::Impact { .. } => "impact",
            Command::Pack { .. } => "pack",
            Command::Rebuild { .. } => "rebuild",
            // One `command` for all three verbs: the envelope names the
            // command a caller invoked, and `telos change …` is one command
            // with subcommands, the same way `telos query …` is.
            Command::Change { .. } => "change",
            // The staging verbs are three commands, not one with an
            // argument, so each names itself.
            Command::Add { .. } => "add",
            Command::Edit { .. } => "edit",
            Command::Move { .. } => "move",
            Command::Remove { .. } => "remove",
            // The two exits from drift, each its own command.
            Command::Adopt { .. } => "adopt",
            Command::Revert { .. } => "revert",
            Command::Test { .. } => "test",
            Command::Bind { .. } => "bind",
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

    if let Command::AgentGuard { host } = &cli.command {
        return commands::agents::guard::run(*host);
    }
    if let Command::View {
        port,
        export: None,
        open,
    } = &cli.command
    {
        let context = match ctx() {
            Ok(context) => context,
            Err(error) => return commands::view::render_startup_error(error, cli.json),
        };
        return commands::view::serve(&context, *port, *open, cli.json);
    }

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
        Command::Init { agents, ci } => commands::init::run(&ctx()?, agents, *ci),
        Command::AgentGuard { .. } => unreachable!("agent guard returned before dispatch"),
        Command::Config { change } => {
            let payload = change.as_ref().map(|_| stdin_payload()).transpose()?;
            commands::config::run(&ctx()?, change.as_deref(), payload.as_deref())
        }
        Command::Map { change } => {
            let payload = change.as_ref().map(|_| stdin_payload()).transpose()?;
            commands::map::run(&ctx()?, change.as_deref(), payload.as_deref())
        }
        Command::Status => commands::status::run(&ctx()?),
        Command::View {
            export: Some(destination),
            open,
            ..
        } => {
            let destination = destination.to_str().ok_or_else(|| {
                TelosError::new(
                    ErrorCode::TelosParseError,
                    "export destination must be valid UTF-8",
                )
            })?;
            commands::view::export(&ctx()?, destination, *open)
        }
        Command::View { export: None, .. } => {
            unreachable!("live view returned before ordinary dispatch")
        }
        Command::Check { sealed } => commands::check::run(&ctx()?, *sealed),
        Command::Show { target } => commands::show::run(&ctx()?, target),
        Command::List {
            kind,
            context,
            capability,
        } => commands::list::run(&ctx()?, *kind, context.as_deref(), capability.as_deref()),
        Command::Query { query } => commands::query::run(&ctx()?, query),
        Command::Impact { target } => commands::impact::run(&ctx()?, target),
        Command::Pack { target } => commands::context::run(&ctx()?, target),
        Command::Rebuild { rebuild } => commands::rebuild::run(&ctx()?, rebuild),
        Command::Change { change } => commands::change::run(&ctx()?, change),
        Command::Add { kind, change } => {
            commands::mutate::add(&ctx()?, *kind, change, &stdin_payload()?)
        }
        Command::Edit { kind, key, change } => {
            commands::mutate::edit(&ctx()?, *kind, key, change, &stdin_payload()?)
        }
        Command::Move { target, to, change } => {
            commands::mutate::move_entity(&ctx()?, target, to, change)
        }
        Command::Remove { kind, key, change } => {
            commands::mutate::remove(&ctx()?, *kind, key, change)
        }
        Command::Adopt {
            into,
            expected_state,
        } => commands::adopt::run(&ctx()?, into.as_deref(), expected_state.as_deref()),
        Command::Revert { expected_state } => {
            commands::revert::run(&ctx()?, expected_state.as_deref())
        }
        Command::Test {
            scenario,
            all,
            file,
        } => commands::test::run(&ctx()?, scenario.as_deref(), *all, file.as_deref()),
        Command::Bind { path, intent } => commands::bind::run(&ctx()?, path, intent),
    }
}

/// Reads the whole of stdin, for the two commands that take a JSON payload
/// there.
///
/// Read here rather than inside the command so that the command layer stays
/// a pure function of its arguments -- the same reason `ctx()` reads the
/// current directory here. An empty read is not an error at this level: what
/// "nothing usable arrived" means is the payload parser's judgement
/// (`commands::mutate`), which reports it under one frozen message whatever
/// the cause.
fn stdin_payload() -> Result<String, TelosError> {
    let mut payload = String::new();
    std::io::stdin().read_to_string(&mut payload).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosParseError,
            format!("payload: failed to read stdin: {e}"),
        )
    })?;
    Ok(payload)
}

fn parse_agent_host(raw: &str) -> Result<AgentHost, String> {
    match raw {
        "claude" => Ok(AgentHost::Claude),
        "codex" => Ok(AgentHost::Codex),
        _ => Err(format!(
            "invalid agent host `{raw}`; expected `claude` or `codex`"
        )),
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
