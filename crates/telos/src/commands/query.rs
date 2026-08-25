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
use telos_core::ids::{NotionName, NotionRef};
use telos_core::model::{ConstraintKind, Intent, IntentStatus, NotionKind, Statement, TelosModel};
use telos_core::suggest;
use telos_core::workspace::Workspace;

use crate::commands::list::DomainFilter;
use crate::commands::{Ctx, diagnostics_to_error, unknown};
use crate::envelope::{CmdResult, Outcome};

/// Which kind of entity `telos query` enumerates, together with the filters
/// that apply to it.
#[derive(Debug, Clone, Subcommand)]
pub enum QueryCommand {
    /// Query intents, optionally filtered.
    Intent {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        capability: Option<String>,
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
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        capability: Option<String>,
        /// Only scenarios that use this notion.
        #[arg(long)]
        using: Option<String>,
    },
    /// Query notions, optionally filtered.
    Notion {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        capability: Option<String>,
        /// Only notions of this kind.
        #[arg(long)]
        kind: Option<NotionKindArg>,
    },
    /// Query constraints, optionally filtered.
    Constraint {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        capability: Option<String>,
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
            context,
            capability,
            using,
            status,
            triggered_by,
        } => query_intent(
            &model,
            &DomainFilter::parse(context.as_deref(), capability.as_deref())?,
            using.as_deref(),
            *status,
            triggered_by.as_deref(),
        ),
        QueryCommand::Scenario {
            context,
            capability,
            using,
        } => query_scenario(
            &model,
            &DomainFilter::parse(context.as_deref(), capability.as_deref())?,
            using.as_deref(),
        ),
        QueryCommand::Notion {
            context,
            capability,
            kind,
        } => query_notion(
            &model,
            &DomainFilter::parse(context.as_deref(), capability.as_deref())?,
            *kind,
        ),
        QueryCommand::Constraint {
            context,
            capability,
            kind,
        } => query_constraint(
            &model,
            &DomainFilter::parse(context.as_deref(), capability.as_deref())?,
            *kind,
        ),
    }
}

/// Resolves a `--using`/`--triggered-by` argument to the notion it names, or
/// the same “unknown notion ...” error (with the same edit-distance hint)
/// `show` reports for an unresolved notion argument.
fn resolve_notion_arg(
    model: &TelosModel,
    filter: &DomainFilter,
    arg: &str,
) -> Result<NotionRef, TelosError> {
    let raw = arg.strip_prefix("NOT:").unwrap_or(arg);
    let reference = if raw.contains('/') {
        raw.parse()?
    } else if let Some(context) = &filter.context {
        NotionRef::new(context.clone(), NotionName::new(raw)?)
    } else {
        return Err(TelosError::new(
            telos_core::error::ErrorCode::TelosParseError,
            format!("notion selector `{arg}` is not context-qualified"),
        )
        .hint("use `NOT:<context>/<Notion>` or add `--context <context>`"));
    };
    if model.domain_notions.contains_key(&reference) {
        return Ok(reference);
    }
    let known: Vec<String> = model
        .domain_notions
        .keys()
        .map(ToString::to_string)
        .collect();
    let hint = suggest::closest(&reference.to_string(), known.iter().map(String::as_str))
        .map(|candidate| format!("closest is `NOT:{candidate}`"));
    Err(unknown("notion", &reference, hint))
}

/// Whether `node` has an outgoing `uses` edge to `name`.
fn uses_notion(model: &TelosModel, node: &NodeRef, reference: &NotionRef) -> bool {
    let target = NodeRef::QualifiedNotion(reference.clone());
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
    filter: &DomainFilter,
    using: Option<&str>,
    status: Option<StatusArg>,
    triggered: Option<&str>,
) -> CmdResult {
    let using = using
        .map(|n| resolve_notion_arg(model, filter, n))
        .transpose()?;
    let triggered = triggered
        .map(|n| resolve_notion_arg(model, filter, n))
        .transpose()?;

    let mut items = Vec::new();
    let mut lines = Vec::new();
    for intent in model.intents.values() {
        let Some(owner) = model.intent_owners.get(&intent.id) else {
            continue;
        };
        if !filter.owner(owner) {
            continue;
        }
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
            && (event.context != owner.context || !triggered_by(intent, &event.notion))
        {
            continue;
        }
        items.push(json!({"id": intent.id, "owner": owner.to_string()}));
        lines.push(format!("{}  {}", intent.id, intent.title));
    }
    Ok(outcome(items, lines))
}

fn query_scenario(model: &TelosModel, filter: &DomainFilter, using: Option<&str>) -> CmdResult {
    let using = using
        .map(|n| resolve_notion_arg(model, filter, n))
        .transpose()?;

    let mut items = Vec::new();
    let mut lines = Vec::new();
    for id in model.scenario_owner.keys() {
        let Some((intent, scenario)) = model.scenario(*id) else {
            continue;
        };
        let Some(owner) = model.intent_owners.get(&intent.id) else {
            continue;
        };
        if !filter.owner(owner) {
            continue;
        }
        if let Some(name) = &using
            && !uses_notion(model, &NodeRef::Scenario(*id), name)
        {
            continue;
        }
        items.push(json!({"id": scenario.id, "owner": owner.to_string()}));
        lines.push(format!("{}  {}", scenario.id, scenario.title));
    }
    Ok(outcome(items, lines))
}

fn query_notion(
    model: &TelosModel,
    filter: &DomainFilter,
    kind: Option<NotionKindArg>,
) -> CmdResult {
    let mut items = Vec::new();
    let mut lines = Vec::new();
    for (reference, notion) in &model.domain_notions {
        let Some(owner) = model.notion_owners.get(reference) else {
            continue;
        };
        if !filter.owner(owner) {
            continue;
        }
        if let Some(kind) = kind
            && notion.kind != NotionKind::from(kind)
        {
            continue;
        }
        items.push(json!({"name": reference, "owner": owner.to_string()}));
        lines.push(reference.to_string());
    }
    Ok(outcome(items, lines))
}

fn query_constraint(
    model: &TelosModel,
    filter: &DomainFilter,
    kind: Option<ConstraintKindArg>,
) -> CmdResult {
    let mut items = Vec::new();
    let mut lines = Vec::new();
    for constraint in model.constraints.values() {
        let Some(owner) = model.constraint_owners.get(&constraint.id) else {
            continue;
        };
        let matches_owner = match owner {
            Some(owner) => filter.owner(owner),
            None => filter.context.is_none() && filter.capability.is_none(),
        };
        if !matches_owner {
            continue;
        }
        if let Some(kind) = kind
            && constraint.kind != ConstraintKind::from(kind)
        {
            continue;
        }
        let owner = owner
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "project".to_string());
        items.push(json!({"id": constraint.id, "owner": owner}));
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
