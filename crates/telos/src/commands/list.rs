//! `telos list <type>`: enumerates one kind of entity across the whole
//! spec, in its natural key order -- the order its `BTreeMap` already keeps
//! it in: alphabetical for notion names, ascending for every numeric id.

use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Value, json};

use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::{CapabilityId, CapabilityRef, ContextId, Owner};
use telos_core::model::TelosModel;
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, diagnostics_to_error};
use crate::envelope::{CmdResult, Outcome};

/// Which kind of entity `telos list` enumerates -- `telos list <type>`'s
/// argument. Renders as its lowercase name (`notion`, `intent`, `scenario`,
/// `constraint`), which is also the type's spelling everywhere else in the
/// CLI, so `telos list widget` is a clap usage error rather than a command
/// that runs and answers with nothing.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum EntityType {
    Context,
    Capability,
    Notion,
    Intent,
    Scenario,
    Constraint,
}

pub fn run(
    ctx: &Ctx,
    kind: EntityType,
    context: Option<&str>,
    capability: Option<&str>,
) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let model = ws.load_model().map_err(diagnostics_to_error)?;
    let filter = DomainFilter::parse(context, capability)?;

    let (items, lines) = match kind {
        EntityType::Context => list_contexts(&model, &filter),
        EntityType::Capability => list_capabilities(&model, &filter),
        EntityType::Notion => list_notions(&model, &filter),
        EntityType::Intent => list_intents(&model, &filter),
        EntityType::Scenario => list_scenarios(&model, &filter),
        EntityType::Constraint => list_constraints(&model, &filter),
    };

    Ok(Outcome {
        result: json!({ "items": items }),
        human: lines.join("\n"),
        next_actions: Vec::new(),
    })
}

#[derive(Default)]
pub(crate) struct DomainFilter {
    pub(crate) context: Option<ContextId>,
    pub(crate) capability: Option<CapabilityId>,
}

impl DomainFilter {
    pub(crate) fn parse(
        context: Option<&str>,
        capability: Option<&str>,
    ) -> Result<Self, TelosError> {
        let mut context = context.map(ContextId::new).transpose()?;
        let capability = match capability {
            Some(raw) if raw.contains('/') => {
                let qualified: CapabilityRef = raw.parse()?;
                if let Some(expected) = &context
                    && expected != &qualified.context
                {
                    return Err(TelosError::new(
                        ErrorCode::TelosContextBoundaryViolation,
                        format!("capability `{qualified}` does not belong to context `{expected}`"),
                    ));
                }
                context = Some(qualified.context);
                Some(qualified.capability)
            }
            Some(raw) => {
                if context.is_none() {
                    return Err(TelosError::new(
                        ErrorCode::TelosParseError,
                        "a bare --capability requires --context",
                    )
                    .hint("use `--context <context> --capability <capability>` or a qualified capability"));
                }
                Some(CapabilityId::new(raw)?)
            }
            None => None,
        };
        Ok(Self {
            context,
            capability,
        })
    }

    pub(crate) fn owner(&self, owner: &Owner) -> bool {
        self.context
            .as_ref()
            .is_none_or(|context| context == &owner.context)
            && self
                .capability
                .as_ref()
                .is_none_or(|capability| owner.capability.as_ref() == Some(capability))
    }
}

/// The lowercase spelling a `kind`/`status` field serializes to, read back
/// off the same `Serialize` impl the JSON item uses -- so a human-mode line
/// and its JSON sibling can never spell an entity's kind two different
/// ways.
fn label(v: impl Serialize) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

fn list_contexts(model: &TelosModel, filter: &DomainFilter) -> (Vec<Value>, Vec<String>) {
    model
        .contexts
        .values()
        .filter(|context| {
            filter
                .context
                .as_ref()
                .is_none_or(|selected| selected == &context.id)
                && filter.capability.is_none()
        })
        .map(|context| {
            let item = json!({
                "id": context.id,
                "kind": context.kind,
                "title": context.title,
            });
            let line = format!("{} [{}] {}", context.id, label(context.kind), context.title);
            (item, line)
        })
        .unzip()
}

fn list_capabilities(model: &TelosModel, filter: &DomainFilter) -> (Vec<Value>, Vec<String>) {
    model
        .capabilities
        .iter()
        .filter(|(id, _)| {
            filter
                .context
                .as_ref()
                .is_none_or(|context| context == &id.context)
                && filter
                    .capability
                    .as_ref()
                    .is_none_or(|capability| capability == &id.capability)
        })
        .map(|(id, capability)| {
            let item = json!({
                "id": id,
                "owner": id.context,
                "title": capability.title,
            });
            let line = format!("{}: {}", id, capability.title);
            (item, line)
        })
        .unzip()
}

fn list_notions(model: &TelosModel, filter: &DomainFilter) -> (Vec<Value>, Vec<String>) {
    model
        .domain_notions
        .iter()
        .filter(|(reference, _)| {
            model
                .notion_owners
                .get(*reference)
                .is_some_and(|owner| filter.owner(owner))
        })
        .map(|(reference, notion)| {
            let owner = &model.notion_owners[reference];
            let item = json!({
                "name": reference,
                "owner": owner.to_string(),
                "kind": notion.kind,
                "def": notion.def,
            });
            let line = format!(
                "{} ({}) [{}]: {}",
                reference,
                label(notion.kind),
                owner,
                notion.def
            );
            (item, line)
        })
        .unzip()
}

fn list_intents(model: &TelosModel, filter: &DomainFilter) -> (Vec<Value>, Vec<String>) {
    model
        .intents
        .values()
        .filter_map(|intent| Some((intent, model.intent_owners.get(&intent.id)?)))
        .filter(|(_, owner)| filter.owner(owner))
        .map(|(intent, owner)| {
            let item = json!({
                "id": intent.id,
                "owner": owner.to_string(),
                "title": intent.title,
                "status": intent.status,
            });
            let line = format!(
                "{} [{}] [{}] {}",
                intent.id,
                label(intent.status),
                owner,
                intent.title
            );
            (item, line)
        })
        .unzip()
}

fn list_scenarios(model: &TelosModel, filter: &DomainFilter) -> (Vec<Value>, Vec<String>) {
    model
        .scenario_owner
        .keys()
        .filter_map(|id| model.scenario(*id))
        .filter_map(|(intent, scenario)| {
            Some((intent, scenario, model.intent_owners.get(&intent.id)?))
        })
        .filter(|(_, _, owner)| filter.owner(owner))
        .map(|(intent, scenario, owner)| {
            let item = json!({
                "id": scenario.id,
                "owner": owner.to_string(),
                "title": scenario.title,
                "intent": intent.id,
            });
            let line = format!(
                "{} [{}] [{}] {}",
                scenario.id, intent.id, owner, scenario.title
            );
            (item, line)
        })
        .unzip()
}

fn list_constraints(model: &TelosModel, filter: &DomainFilter) -> (Vec<Value>, Vec<String>) {
    model
        .constraints
        .values()
        .filter_map(|constraint| Some((constraint, model.constraint_owners.get(&constraint.id)?)))
        .filter(|(_, owner)| match owner {
            Some(owner) => filter.owner(owner),
            None => filter.context.is_none() && filter.capability.is_none(),
        })
        .map(|(constraint, owner)| {
            let owner = owner
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "project".to_string());
            let item = json!({
                "id": constraint.id,
                "owner": owner,
                "kind": constraint.kind,
                "title": constraint.title,
            });
            let line = format!(
                "{} [{}] [{}] {}",
                constraint.id,
                label(constraint.kind),
                owner,
                constraint.title
            );
            (item, line)
        })
        .unzip()
}
