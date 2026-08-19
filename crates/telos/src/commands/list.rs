//! `telos list <type>`: enumerates one kind of entity across the whole
//! spec, in its natural key order -- the order its `BTreeMap` already keeps
//! it in: alphabetical for notion names, ascending for every numeric id.

use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Value, json};

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
    Notion,
    Intent,
    Scenario,
    Constraint,
}

pub fn run(ctx: &Ctx, kind: EntityType) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let model = ws.load_model().map_err(diagnostics_to_error)?;

    let (items, lines) = match kind {
        EntityType::Notion => list_notions(&model),
        EntityType::Intent => list_intents(&model),
        EntityType::Scenario => list_scenarios(&model),
        EntityType::Constraint => list_constraints(&model),
    };

    Ok(Outcome {
        result: json!({ "items": items }),
        human: lines.join("\n"),
        next_actions: Vec::new(),
    })
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

fn list_notions(model: &TelosModel) -> (Vec<Value>, Vec<String>) {
    model
        .notions
        .values()
        .map(|n| {
            let item = json!({ "name": n.name, "kind": n.kind, "def": n.def });
            let line = format!("{} ({}): {}", n.name, label(n.kind), n.def);
            (item, line)
        })
        .unzip()
}

fn list_intents(model: &TelosModel) -> (Vec<Value>, Vec<String>) {
    model
        .intents
        .values()
        .map(|i| {
            let item = json!({ "id": i.id, "title": i.title, "status": i.status });
            let line = format!("{} [{}] {}", i.id, label(i.status), i.title);
            (item, line)
        })
        .unzip()
}

fn list_scenarios(model: &TelosModel) -> (Vec<Value>, Vec<String>) {
    model
        .scenario_owner
        .keys()
        .filter_map(|id| model.scenario(*id))
        .map(|(intent, scenario)| {
            let item = json!({ "id": scenario.id, "title": scenario.title, "intent": intent.id });
            let line = format!("{} [{}] {}", scenario.id, intent.id, scenario.title);
            (item, line)
        })
        .unzip()
}

fn list_constraints(model: &TelosModel) -> (Vec<Value>, Vec<String>) {
    model
        .constraints
        .values()
        .map(|c| {
            let item = json!({ "id": c.id, "kind": c.kind, "title": c.title });
            let line = format!("{} [{}] {}", c.id, label(c.kind), c.title);
            (item, line)
        })
        .unzip()
}
