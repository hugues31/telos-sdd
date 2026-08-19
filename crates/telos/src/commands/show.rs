//! `telos show <id|Name>`: prints one entity's canonical block plus its
//! relations, resolving a typed id (`INT-0042`, `SCN-0107`, `CON-0003`) or a
//! bare notion name (`Invoice`) against the loaded spec.
//!
//! The canonical block is never reformatted here -- it is exactly what
//! [`telos_core::emit`] produces for the entity, reused byte for byte, so
//! `show` and the emitter can never disagree about what "canonical" means.
//! A scenario has no canonical block of its own (a scenario is nested
//! inside its owning intent's file, not a file in its own right), so
//! `show SCN-...` prints the *owning intent's* block, headed by a line that
//! names which scenario is being shown and which intent owns it.
//!
//! `show CHG-...` is the one argument resolved outside the spec model
//! entirely: a change lives in the change store, not in `spec_files`, so it
//! is looked up there and reports the empty relations of Annex E -- see
//! [`show_change`].

use serde_json::{Value, json};

use telos_core::changes::read_change;
use telos_core::emit::{emit_change, emit_constraint, emit_intent, emit_notion};
use telos_core::graph::NodeRef;
use telos_core::ids::{ChangeId, ConstraintId, EntityRef, IntentId, NotionName, ScenarioId};
use telos_core::model::TelosModel;
use telos_core::suggest;
use telos_core::workspace::Workspace;

use crate::commands::change::op_descriptors;
use crate::commands::{Ctx, diagnostics_to_error, nearest_id, unknown, unparsable};
use crate::envelope::{CmdResult, Outcome};

pub fn run(ctx: &Ctx, target: &str) -> CmdResult {
    let entity_ref: EntityRef = target.parse().map_err(|_| unparsable(target))?;

    let ws = Workspace::discover(&ctx.cwd)?;

    // A change is not part of the spec model -- `telos/changes/` is excluded
    // from `spec_files` -- so it is resolved against the change store, and
    // resolved *before* the model is loaded: showing a change must not
    // depend on whether the spec around it happens to parse.
    if let EntityRef::Change(id) = entity_ref {
        return show_change(&ws, id);
    }

    let model = ws.load_model().map_err(diagnostics_to_error)?;

    match entity_ref {
        EntityRef::Notion(name) => show_notion(&model, &name),
        EntityRef::Intent(id) => show_intent(&model, id),
        EntityRef::Scenario(id) => show_scenario(&model, id),
        EntityRef::Constraint(id) => show_constraint(&model, id),
        // Handled above, before the model was ever loaded.
        EntityRef::Change(id) => show_change(&ws, id),
    }
}

/// `show CHG-0001`: the change's own fields, its ops as descriptors, and
/// its canonical text.
///
/// `canonical` is [`emit_change`]'s output rather than the file's bytes on
/// disk -- the same policy every other `show` follows (the emitter defines
/// what canonical means, and D1's round-trip makes the two identical for
/// any file the tool wrote). `relations` is the empty pair Annex E freezes:
/// a change is a transaction record, not a node of the spec graph, so it
/// has no edge to report -- and the key is still present, so a consumer
/// reads `show` the same way whatever it was pointed at.
fn show_change(ws: &Workspace, id: ChangeId) -> CmdResult {
    let change = read_change(ws, id)?;
    let canonical = emit_change(&change);

    Ok(Outcome {
        result: json!({
            "entity": {
                "id": change.id,
                "status": change.status.as_str(),
                "motivation": change.motivation,
                "ops": op_descriptors(&change),
            },
            "canonical": canonical,
            "relations": { "out": [], "in": [] },
        }),
        human: format!("{canonical}\nrelations:"),
        next_actions: Vec::new(),
    })
}

fn show_notion(model: &TelosModel, name: &NotionName) -> CmdResult {
    let Some(notion) = model.notions.get(name) else {
        let known: Vec<&str> = model.notions.keys().map(NotionName::as_str).collect();
        let hint = suggest::closest(name.as_str(), known.iter().copied())
            .map(|c| format!("closest is `{c}`"));
        return Err(unknown("notion", name, hint));
    };

    let node = NodeRef::Notion(name.clone());
    let canonical = emit_notion(notion);
    Ok(entity_outcome(json!(notion), canonical, model, &node))
}

fn show_intent(model: &TelosModel, id: IntentId) -> CmdResult {
    let Some(intent) = model.intents.get(&id) else {
        let hint = nearest_id(id.0, model.intents.keys().map(|i| i.0), |n| {
            IntentId(n).to_string()
        });
        return Err(unknown("intent", id, hint));
    };

    let node = NodeRef::Intent(id);
    let canonical = emit_intent(intent);
    Ok(entity_outcome(json!(intent), canonical, model, &node))
}

fn show_constraint(model: &TelosModel, id: ConstraintId) -> CmdResult {
    let Some(constraint) = model.constraints.get(&id) else {
        let hint = nearest_id(id.0, model.constraints.keys().map(|c| c.0), |n| {
            ConstraintId(n).to_string()
        });
        return Err(unknown("constraint", id, hint));
    };

    let node = NodeRef::Constraint(id);
    let canonical = emit_constraint(constraint);
    Ok(entity_outcome(json!(constraint), canonical, model, &node))
}

/// Builds the answer for a notion, intent or constraint: their shapes are
/// identical (the entity's own canonical block, headed by nothing, followed
/// by the relations of its own node), unlike a scenario's (see
/// [`show_scenario`]).
fn entity_outcome(entity: Value, canonical: String, model: &TelosModel, node: &NodeRef) -> Outcome {
    let human = format!("{canonical}\n{}", relations_human(model, node));
    let result = json!({
        "entity": entity,
        "canonical": canonical,
        "relations": relations_json(model, node),
    });
    Outcome {
        result,
        human,
        next_actions: Vec::new(),
    }
}

fn show_scenario(model: &TelosModel, id: ScenarioId) -> CmdResult {
    let Some((intent, scenario)) = model.scenario(id) else {
        let hint = nearest_id(id.0, model.scenario_owner.keys().map(|s| s.0), |n| {
            ScenarioId(n).to_string()
        });
        return Err(unknown("scenario", id, hint));
    };

    // The scenario's own node -- relations hang off the scenario, not off
    // the owning intent whose block is reused to print it.
    let node = NodeRef::Scenario(id);
    let canonical = emit_intent(intent);
    let header = format!("scenario {id} belongs to intent {}:", intent.id);
    let human = format!("{header}\n{canonical}\n{}", relations_human(model, &node));
    let result = json!({
        "entity": scenario,
        "canonical": canonical,
        "relations": relations_json(model, &node),
    });
    Ok(Outcome {
        result,
        human,
        next_actions: Vec::new(),
    })
}

/// `relations:` then one `  -> <rel> <target>` line per out edge and one
/// `  <- <rel> <source>` line per in edge, both in graph order (sorted by
/// `(relation, node)`, per [`telos_core::graph::Graph`]).
fn relations_human(model: &TelosModel, node: &NodeRef) -> String {
    let mut lines = vec!["relations:".to_string()];
    for (rel, to) in model.graph.out_edges(node) {
        lines.push(format!("  -> {rel} {to}"));
    }
    for (rel, from) in model.graph.in_edges(node) {
        lines.push(format!("  <- {rel} {from}"));
    }
    lines.join("\n")
}

/// `{"out": [{"rel": ..., "to": ...}], "in": [{"rel": ..., "from": ...}]}`.
fn relations_json(model: &TelosModel, node: &NodeRef) -> Value {
    let out: Vec<Value> = model
        .graph
        .out_edges(node)
        .iter()
        .map(|(rel, to)| json!({ "rel": rel, "to": to }))
        .collect();
    let inn: Vec<Value> = model
        .graph
        .in_edges(node)
        .iter()
        .map(|(rel, from)| json!({ "rel": rel, "from": from }))
        .collect();
    json!({ "out": out, "in": inn })
}
