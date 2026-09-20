//! Native plan CLI. Every mutation is an atomic, replayable core operation.

use clap::{Args, Subcommand};
use serde_json::{Value, json};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::plans::{actions, execution, ledger, model::*, store};
use telos_core::work::new_id;
use telos_core::workspace::Workspace;

use super::Ctx;
use crate::envelope::{CmdResult, Outcome};

#[derive(Debug, Clone, Args)]
pub struct PlanArgs {
    /// Reuse this identity to retry a mutation without repeating its effects.
    #[arg(skip)]
    pub request_id: Option<String>,
    /// Refuse an update if another writer has advanced the plan journal.
    #[arg(skip)]
    pub expected_version: Option<u64>,
    #[command(subcommand)]
    pub command: PlanCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum PlanCommand {
    /// Create a draft; invoke telos-brainstormer before defining its tasks.
    Open { title: String },
    /// List plans with verified progress.
    List,
    /// Show the full native record, including all revisions and events.
    Show { id: String },
    /// Append a definition revision from JSON on stdin.
    Edit { id: String },
    /// Display the exact contract and digest for human review.
    Diff { id: String },
    /// Approve exactly the reviewed revision; grants its task changes authority.
    Approve {
        id: String,
        #[arg(long)]
        expected_digest: String,
    },
    /// Reconstruct the next action from persisted state and current files.
    Resume { id: String },
    /// Save progress without changing the approved definition.
    Checkpoint {
        id: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        next_action: String,
    },
    /// Pause execution, retaining ownership of unfinished files.
    Pause {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Continue an approved paused plan.
    Continue { id: String },
    /// Cancel after resolving open changes; retains the entire history.
    Cancel {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Execute one approved validation command, scenario check, or review.
    Verify {
        id: String,
        #[arg(long)]
        task: Option<String>,
        name: String,
        #[arg(long)]
        review: Option<String>,
        #[arg(long)]
        retry_unknown: bool,
    },
    /// Finish after every task and final validation passes.
    Complete { id: String },
    /// Retain source-branch work records under an approved integration task.
    Integrate {
        id: String,
        #[arg(long)]
        source: String,
    },
    /// Prepare, execute and finish individual dependency-ordered tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TaskCommand {
    Prepare {
        plan: String,
        task: String,
    },
    /// Import a prepared change's exact delta into a new plan revision.
    Import {
        plan: String,
        task: String,
    },
    Start {
        plan: String,
        task: String,
    },
    Finish {
        plan: String,
        task: String,
    },
    Block {
        plan: String,
        task: String,
        reason: String,
    },
    Unblock {
        plan: String,
        task: String,
        reason: String,
    },
}

pub fn run(ctx: &Ctx, args: &PlanArgs, payload: Option<&str>) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let root = &ws.repo_root;
    let generated = new_id("REQ")?;
    let request = args.request_id.as_deref().unwrap_or(&generated);
    let version = args.expected_version;
    let value = match &args.command {
        PlanCommand::Open { title } => {
            ledger::verify(root)?;
            store::open(root, title, request)?
        }
        PlanCommand::List => {
            json!({"plans":store::list(root)?.iter().map(Plan::view).collect::<Result<Vec<_>,_>>()?})
        }
        PlanCommand::Show { id } => serde_json::to_value(store::read(root, id)?).expect("plan"),
        PlanCommand::Edit { id } => {
            let definition: Definition =
                serde_json::from_str(payload.unwrap_or("")).map_err(|e| {
                    TelosError::new(ErrorCode::TelosParseError, format!("plan definition: {e}"))
                })?;
            actions::revise(root, id, definition, request, version)?
        }
        PlanCommand::Diff { id } => {
            let plan = store::read(root, id)?;
            json!({"plan":plan.view()?,"definition":plan.revision(),"previous":plan.revisions.iter().rev().nth(1),"digest":plan.definition_digest()?})
        }
        PlanCommand::Approve {
            id,
            expected_digest,
        } => actions::approve(root, id, expected_digest, request, version)?,
        PlanCommand::Resume { id } => execution::resume(root, id)?,
        PlanCommand::Checkpoint {
            id,
            summary,
            next_action,
        } => actions::checkpoint(root, id, summary, next_action, request, version)?,
        PlanCommand::Pause { id, reason } => {
            actions::transition(root, id, None, EventKind::Paused, reason, request, version)?
        }
        PlanCommand::Continue { id } => {
            actions::transition(root, id, None, EventKind::Continued, "", request, version)?
        }
        PlanCommand::Cancel { id, reason } => actions::transition(
            root,
            id,
            None,
            EventKind::Cancelled,
            reason,
            request,
            version,
        )?,
        PlanCommand::Verify {
            id,
            task,
            name,
            review,
            retry_unknown,
        } => execution::verify(
            root,
            id,
            task.as_deref(),
            name,
            review.as_deref(),
            *retry_unknown,
            request,
            version,
        )?,
        PlanCommand::Complete { id } => execution::complete(root, id, request, version)?,
        PlanCommand::Integrate { id, source } => {
            ledger::integrate(root, id, source, request, version)?
        }
        PlanCommand::Task { command } => match command {
            TaskCommand::Prepare { plan, task } => {
                actions::prepare(root, plan, task, request, version)?
            }
            TaskCommand::Import { plan, task } => {
                actions::import_change(root, plan, task, request, version)?
            }
            TaskCommand::Start { plan, task } => {
                execution::start(root, plan, task, request, version)?
            }
            TaskCommand::Finish { plan, task } => {
                execution::finish(root, plan, task, request, version)?
            }
            TaskCommand::Block { plan, task, reason } => actions::transition(
                root,
                plan,
                Some(task),
                EventKind::TaskBlocked,
                reason,
                request,
                version,
            )?,
            TaskCommand::Unblock { plan, task, reason } => actions::transition(
                root,
                plan,
                Some(task),
                EventKind::TaskUnblocked,
                reason,
                request,
                version,
            )?,
        },
    };
    outcome(value)
}

pub fn history(ctx: &Ctx, target: Option<&str>) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let target = target.map(|t| {
        t.strip_prefix("CTX:")
            .or_else(|| t.strip_prefix("CAP:"))
            .or_else(|| t.strip_prefix("NOT:"))
            .unwrap_or(t)
    });
    let abandoned: Vec<_> = store::list(&ws.repo_root)?
        .into_iter()
        .flat_map(|p| {
            p.events.into_iter().filter_map(move |e| {
                (e.kind == EventKind::ChangeAbandoned
                    && target.is_none_or(|t| p.id == t || e.data["change"] == t))
                .then(|| json!({"plan":p.id,"event":e}))
            })
        })
        .collect();
    outcome(
        json!({"target":target,"history":ledger::history(&ws.repo_root,target)?,"abandoned":abandoned}),
    )
}

pub fn recover(ctx: &Ctx) -> CmdResult {
    let git = telos_core::git::GitRepo::discover(&ctx.cwd)?;
    outcome(json!({"recovered":telos_core::transaction::recover(git.root())?}))
}

pub fn outcome(value: Value) -> CmdResult {
    Ok(Outcome {
        human: serde_json::to_string_pretty(&value).expect("JSON"),
        result: value,
        next_actions: vec![],
    })
}
