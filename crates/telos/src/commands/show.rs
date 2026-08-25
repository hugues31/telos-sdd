//! `telos show <id|Name>`: prints one entity's canonical block plus its
//! relations, resolving a typed id (`INT-0042`, `SCN-0107`, `CON-0003`) or a
//! bare notion name (`Invoice`) against the loaded spec.
//!
//! A *spec* entity's canonical block is never reformatted here -- it is
//! exactly what [`telos_core::emit`] produces for it, reused byte for byte,
//! so `show` and the emitter can never disagree about what "canonical"
//! means for a sealed file (a change is the exception, see below).
//! A scenario has no canonical block of its own (a scenario is nested
//! inside its owning intent's file, not a file in its own right), so
//! `show SCN-...` prints the *owning intent's* block, headed by a line that
//! names which scenario is being shown and which intent owns it.
//!
//! `show CHG-...` is the one argument resolved outside the spec model
//! entirely: a change lives in the change store, not in `spec_files`, so it
//! is looked up there, reports the empty relations of the result schema, and -- alone
//! among the entities -- prints the bytes of its file rather than a
//! re-emission of them, because a change file is not sealed and may legally
//! differ from what the emitter would write. See [`show_change`].

use std::fs;

use serde_json::{Value, json};

use telos_core::changes::read_change;
use telos_core::emit::{
    emit_capability, emit_constraint, emit_context, emit_intent, emit_notion,
    emit_owned_constraint, emit_owned_intent, emit_owned_notion,
};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::graph::NodeRef;
use telos_core::ids::{
    CapabilityRef, ChangeId, ConstraintId, ContextId, EntityRef, IntentId, NotionRef, ScenarioId,
};
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
        EntityRef::Context(id) => show_context(&model, &id),
        EntityRef::Capability(id) => show_capability(&model, &id),
        EntityRef::Notion(name) => show_notion(&model, &name),
        EntityRef::Intent(id) => show_intent(&model, id),
        EntityRef::Scenario(id) => show_scenario(&model, id),
        EntityRef::Constraint(id) => show_constraint(&model, id),
        // Handled above, before the model was ever loaded.
        EntityRef::Change(id) => show_change(&ws, id),
    }
}

fn show_context(model: &TelosModel, id: &ContextId) -> CmdResult {
    let Some(context) = model.contexts.get(id) else {
        return Err(unknown("context", id, None));
    };
    Ok(entity_outcome(
        json!(context),
        emit_context(context),
        model,
        &NodeRef::Context(id.clone()),
    ))
}

fn show_capability(model: &TelosModel, id: &CapabilityRef) -> CmdResult {
    let Some(capability) = model.capabilities.get(id) else {
        return Err(unknown("capability", id, None));
    };
    Ok(entity_outcome(
        json!(capability),
        emit_capability(capability),
        model,
        &NodeRef::Capability(id.clone()),
    ))
}

/// `show CHG-0001`: the change's own fields, its ops as descriptors, and
/// the text of its file.
///
/// `canonical` is the file's actual bytes (the result schema: "the file text"),
/// *not* a re-emission of the parsed change -- and that is a deliberate
/// departure from what `show` does for a notion, an intent or a constraint.
/// The reason is that a change file is not covered by the seal
/// (`telos/changes/` is excluded from [`Workspace::spec_files`]), so a
/// hand-edited but still parseable change file is legal state rather than
/// drift, and inspecting a transaction record has to show what is really on
/// disk. For any file telos itself wrote the two are identical anyway, by
/// the change journal's round-trip invariant; the distinction appears only
/// when it matters.
///
/// The parse is still what produces `entity`: [`read_change`] is what turns
/// an unknown id into the store's “unknown change `CHG-9999`” and a
/// corrupted one into a parse error, so nothing here reports fields off a
/// file it did not first validate.
///
/// `relations` is the empty pair the result schema freezes: a change is a transaction
/// record, not a node of the spec graph, so it has no edge to report -- and
/// the key is still present, so a consumer reads `show` the same way
/// whatever it was pointed at.
fn show_change(ws: &Workspace, id: ChangeId) -> CmdResult {
    let change = read_change(ws, id)?;
    let canonical = read_change_file(ws, id)?;

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

/// The raw text of `telos/changes/<id>.tel`.
///
/// Read *after* [`read_change`] has already succeeded, so the only failure
/// left here is an I/O one racing the first read (the file deleted in
/// between, a permission revoked) -- never "unknown change", which
/// [`read_change`] owns and reports with its nearest-id hint.
fn read_change_file(ws: &Workspace, id: ChangeId) -> Result<String, TelosError> {
    let path = ws.telos_dir.join("changes").join(format!("{id}.tel"));
    fs::read_to_string(&path).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to read {}: {e}", path.display()),
        )
    })
}

fn show_notion(model: &TelosModel, name: &NotionRef) -> CmdResult {
    let Some(notion) = model.domain_notions.get(name) else {
        let target = name.to_string();
        let known: Vec<String> = model
            .domain_notions
            .keys()
            .map(ToString::to_string)
            .collect();
        let hint = suggest::closest(&target, known.iter().map(String::as_str))
            .map(|candidate| format!("closest is `{candidate}`"));
        return Err(unknown("notion", name, hint));
    };

    let node = NodeRef::QualifiedNotion(name.clone());
    let canonical = model.notion_owners.get(name).map_or_else(
        || emit_notion(notion),
        |owner| emit_owned_notion(owner, notion),
    );
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
    let canonical = model.intent_owners.get(&id).map_or_else(
        || emit_intent(intent),
        |owner| emit_owned_intent(owner, intent),
    );
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
    let canonical = model.constraint_owners.get(&id).map_or_else(
        || emit_constraint(constraint),
        |owner| emit_owned_constraint(owner.as_ref(), constraint),
    );
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
