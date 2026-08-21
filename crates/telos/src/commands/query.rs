//! `telos query <type>`: entities of one kind, filtered in AND and listed by
//! natural key -- the read side of the same relation graph `impact` walks
//! the other way.
//!
//! Each type is its own clap sub-subcommand declaring only the filters that
//! apply to it (`notion` has no `--status`, `scenario` has no `--kind`), so
//! an inapplicable flag is a clap usage error (exit 2) before any command
//! runs, rather than a filter a command silently ignores.

use clap::{Subcommand, ValueEnum};
use serde_json::{Value, json};

use telos_core::error::TelosError;
use telos_core::graph::{NodeRef, Relation};
use telos_core::ids::NotionName;
use telos_core::model::{ConstraintKind, Intent, IntentStatus, NotionKind, Statement, TelosModel};
use telos_core::suggest;
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, diagnostics_to_error, unknown};
use crate::envelope::{CmdResult, Outcome};

/// Which kind of entity `telos query` enumerates, together with the filters
/// that apply to it.
#[derive(Debug, Clone, Subcommand)]
pub enum QueryCommand {
    /// Query intents, optionally filtered.
    Intent {
        /// Only intents that use this notion.
        #[arg(long)]
        using: Option<String>,
        /// Only intents with this status.
        #[arg(long)]
        status: Option<StatusArg>,
        /// Only intents whose statement is driven by this event.
        #[arg(long = "triggered-by")]
        triggered_by: Option<String>,
    },
    /// Query scenarios, optionally filtered.
    Scenario {
        /// Only scenarios that use this notion.
        #[arg(long)]
        using: Option<String>,
    },
    /// Query notions, optionally filtered.
    Notion {
        /// Only notions of this kind.
        #[arg(long)]
        kind: Option<NotionKindArg>,
    },
    /// Query constraints, optionally filtered.
    Constraint {
        /// Only constraints of this kind.
        #[arg(long)]
        kind: Option<ConstraintKindArg>,
    },
}

/// `--status`'s value, one-to-one with [`telos_core::model::IntentStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum StatusArg {
    Draft,
    Active,
    Deprecated,
}

impl From<StatusArg> for IntentStatus {
    fn from(arg: StatusArg) -> Self {
        match arg {
            StatusArg::Draft => IntentStatus::Draft,
            StatusArg::Active => IntentStatus::Active,
            StatusArg::Deprecated => IntentStatus::Deprecated,
        }
    }
}

/// `query notion --kind`'s value, one-to-one with
/// [`telos_core::model::NotionKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum NotionKindArg {
    Actor,
    Entity,
    Value,
    Event,
    State,
}

impl From<NotionKindArg> for NotionKind {
    fn from(arg: NotionKindArg) -> Self {
        match arg {
            NotionKindArg::Actor => NotionKind::Actor,
            NotionKindArg::Entity => NotionKind::Entity,
            NotionKindArg::Value => NotionKind::Value,
            NotionKindArg::Event => NotionKind::Event,
            NotionKindArg::State => NotionKind::State,
        }
    }
}

/// `query constraint --kind`'s value, one-to-one with
/// [`telos_core::model::ConstraintKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum ConstraintKindArg {
    Stack,
    Architecture,
    Quality,
    Security,
    Convention,
}

impl From<ConstraintKindArg> for ConstraintKind {
    fn from(arg: ConstraintKindArg) -> Self {
        match arg {
            ConstraintKindArg::Stack => ConstraintKind::Stack,
            ConstraintKindArg::Architecture => ConstraintKind::Architecture,
            ConstraintKindArg::Quality => ConstraintKind::Quality,
            ConstraintKindArg::Security => ConstraintKind::Security,
            ConstraintKindArg::Convention => ConstraintKind::Convention,
        }
    }
}

pub fn run(ctx: &Ctx, query: &QueryCommand) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let model = ws.load_model().map_err(diagnostics_to_error)?;

    match query {
        QueryCommand::Intent {
            using,
            status,
            triggered_by,
        } => query_intent(&model, using.as_deref(), *status, triggered_by.as_deref()),
        QueryCommand::Scenario { using } => query_scenario(&model, using.as_deref()),
        QueryCommand::Notion { kind } => query_notion(&model, *kind),
        QueryCommand::Constraint { kind } => query_constraint(&model, *kind),
    }
}

/// Resolves a `--using`/`--triggered-by` argument to the notion it names, or
/// the same “unknown notion ...” error (with the same edit-distance hint)
/// `show` reports for an unresolved notion argument.
fn resolve_notion_arg(model: &TelosModel, arg: &str) -> Result<NotionName, TelosError> {
    let name = NotionName::new(arg)?;
    if model.notions.contains_key(&name) {
        return Ok(name);
    }
    let known: Vec<&str> = model.notions.keys().map(NotionName::as_str).collect();
    let hint =
        suggest::closest(name.as_str(), known.iter().copied()).map(|c| format!("closest is `{c}`"));
    Err(unknown("notion", &name, hint))
}

/// Whether `node` has an outgoing `uses` edge to `name`.
fn uses_notion(model: &TelosModel, node: &NodeRef, name: &NotionName) -> bool {
    let target = NodeRef::Notion(name.clone());
    model
        .graph
        .out_edges(node)
        .iter()
        .any(|(rel, to)| *rel == Relation::Uses && *to == target)
}

/// Whether `intent`'s statement is event-driven by `event`.
fn triggered_by(intent: &Intent, event: &NotionName) -> bool {
    matches!(&intent.statement, Statement::EventDriven { event: e, .. } if e.node == *event)
}

fn query_intent(
    model: &TelosModel,
    using: Option<&str>,
    status: Option<StatusArg>,
    triggered: Option<&str>,
) -> CmdResult {
    let using = using.map(|n| resolve_notion_arg(model, n)).transpose()?;
    let triggered = triggered
        .map(|n| resolve_notion_arg(model, n))
        .transpose()?;

    let mut items = Vec::new();
    let mut lines = Vec::new();
    for intent in model.intents.values() {
        if let Some(name) = &using
            && !uses_notion(model, &NodeRef::Intent(intent.id), name)
        {
            continue;
        }
        if let Some(status) = status
            && intent.status != IntentStatus::from(status)
        {
            continue;
        }
        if let Some(event) = &triggered
            && !triggered_by(intent, event)
        {
            continue;
        }
        items.push(json!(intent.id));
        lines.push(format!("{}  {}", intent.id, intent.title));
    }
    Ok(outcome(items, lines))
}

fn query_scenario(model: &TelosModel, using: Option<&str>) -> CmdResult {
    let using = using.map(|n| resolve_notion_arg(model, n)).transpose()?;

    let mut items = Vec::new();
    let mut lines = Vec::new();
    for id in model.scenario_owner.keys() {
        let Some((_, scenario)) = model.scenario(*id) else {
            continue;
        };
        if let Some(name) = &using
            && !uses_notion(model, &NodeRef::Scenario(*id), name)
        {
            continue;
        }
        items.push(json!(scenario.id));
        lines.push(format!("{}  {}", scenario.id, scenario.title));
    }
    Ok(outcome(items, lines))
}

fn query_notion(model: &TelosModel, kind: Option<NotionKindArg>) -> CmdResult {
    let mut items = Vec::new();
    let mut lines = Vec::new();
    for notion in model.notions.values() {
        if let Some(kind) = kind
            && notion.kind != NotionKind::from(kind)
        {
            continue;
        }
        items.push(json!(notion.name));
        lines.push(notion.name.to_string());
    }
    Ok(outcome(items, lines))
}

fn query_constraint(model: &TelosModel, kind: Option<ConstraintKindArg>) -> CmdResult {
    let mut items = Vec::new();
    let mut lines = Vec::new();
    for constraint in model.constraints.values() {
        if let Some(kind) = kind
            && constraint.kind != ConstraintKind::from(kind)
        {
            continue;
        }
        items.push(json!(constraint.id));
        lines.push(format!("{}  {}", constraint.id, constraint.title));
    }
    Ok(outcome(items, lines))
}

/// `{"items": [...]}` in JSON, one id/name per line in human mode.
fn outcome(items: Vec<Value>, lines: Vec<String>) -> Outcome {
    Outcome {
        result: json!({ "items": items }),
        human: lines.join("\n"),
        next_actions: Vec::new(),
    }
}
