//! `telos context <INT-id | SCN-id>`: the bounded work pack of one intent
//! (spec §7.3, D10) -- the unit an agent is fed, never the whole spec.
//!
//! The pack is the intent's own canonical block plus everything an
//! implementer needs to act on it without walking the graph by hand: its
//! scenarios (with whether each is already proved), the notions its
//! statement and scenarios use, the constraints that apply to it (global
//! ones and any scoped to it), its own bindings, and its 1-hop
//! intent-to-intent neighbours. A scenario argument resolves to the intent
//! that owns it -- there is no scenario-shaped pack, only an intent-shaped
//! one (Annex C).
//!
//! # Which model the pack is read from (D10)
//!
//! `context` never requires an open change -- most calls are against the
//! coherent, sealed project, and the disk model answers those directly. But
//! an implementer also calls `context` on an intent an `add intent` staged a
//! moment ago, which the sealed spec has never heard of, or on one an open
//! change is mid-edit -- and the pack has to reflect *that* delta, journal
//! included, not the stale sealed one. So resolution tries the disk model
//! first and only falls back to a change's overlay when [`owner_of`] finds
//! one: the open change whose delta adds or edits the resolved intent, the
//! same ownership rule `telos bind` and `telos test` already apply one level
//! down (D5). When it does, the pack is built on that change's *post*
//! model -- ops replayed idempotently, the journal folded into bindings,
//! then the semantic pass -- exactly what `reconcile` itself builds (D2),
//! so a pack an implementer reads mid-change never disagrees with what
//! reconciling it would seal.
//!
//! Two things ride along with that choice:
//!
//! - `result.change` names the change the pack came from, `null` for the
//!   disk model -- so an agent knows whether what it just read is the
//!   sealed spec or one change's proposed future.
//! - The journal matters here in a way it does not for `show`: a `telos
//!   bind`/`telos test` green run is real evidence of progress the
//!   implementer just made, and folding it in (rather than reading the
//!   change's *ops* alone) is what makes `context`'s `bindings` section
//!   answer "what has this intent's implementation actually bound so far",
//!   not just "what the delta declares".

use std::collections::BTreeSet;

use serde_json::{Value, json};

use telos_core::emit::{emit_constraint, emit_intent, emit_notion};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::graph::{NodeRef, Relation};
use telos_core::ids::{
    ChangeId, ConstraintId, EntityRef, IntentId, NotionName, RepoPath, ScenarioId,
};
use telos_core::model::{Binding, Change, Intent, StagedOp, TelosModel};
use telos_core::overlay::{apply_ops_idempotent, fold_journal_bindings, parse_base};
use telos_core::semantic::build_model;

use crate::commands::{
    Ctx, Project, diagnostics_to_error, nearest_id, project, unknown, unparsable,
};
use crate::envelope::{CmdResult, Outcome};
use crate::projection::{applicable_constraints, implementations, proofs};

pub fn run(ctx: &Ctx, target: &str) -> CmdResult {
    let entity_ref: EntityRef = target.parse().map_err(|_| unparsable(target))?;
    if matches!(
        entity_ref,
        EntityRef::Notion(_) | EntityRef::Constraint(_) | EntityRef::Change(_)
    ) {
        return Err(not_applicable());
    }

    let project = project(ctx)?;
    let disk = project.ws.load_model().map_err(diagnostics_to_error)?;

    let (intent_id, owner) = resolve_intent(&project, &disk, &entity_ref)?;

    let (model, change) = match owner {
        Some(change) => (post_model(&project, change)?, Some(change.id)),
        None => (disk, None),
    };
    let intent = model
        .intents
        .get(&intent_id)
        .ok_or_else(|| unknown_intent(&project, &model, intent_id))?;

    let pack = build_pack(&model, intent, change);
    Ok(Outcome {
        result: to_json(&pack),
        human: to_human(&pack),
        next_actions: Vec::new(),
    })
}

/// «`context` applies to intents and scenarios» -- the frozen refusal for a
/// notion or a change argument (D10). Unlike `show`/`impact`, `context` has
/// exactly two shapes of target, so a third kind of well-formed reference is
/// not "not found", it is out of scope; there is nothing to suggest instead.
fn not_applicable() -> TelosError {
    TelosError::new(
        ErrorCode::TelosReferenceUnknown,
        "`context` applies to intents and scenarios",
    )
}

// --- resolving the argument to a pack's intent ------------------------------

/// The intent a pack is built around: `entity_ref` itself for an `INT-…`
/// argument, or the owning intent for a `SCN-…` one (the caller has already
/// filtered out the notion/change shapes).
///
/// "Known" spans both worlds an intent or scenario can live in, exactly as
/// `telos bind`/`telos test` already judge ownership one level down: the
/// disk model, and every open change's staged intents -- an intent or
/// scenario `add`/`edit intent` allocated a moment ago is precisely the one
/// `telos context` is about to be pointed at.
fn resolve_intent<'a>(
    project: &'a Project,
    disk: &TelosModel,
    entity_ref: &EntityRef,
) -> Result<(IntentId, Option<&'a Change>), TelosError> {
    match entity_ref {
        EntityRef::Intent(id) => {
            if let Some(change) = change_touching_intent(&project.parsed, *id) {
                if staged_intents(change).contains_key(id) {
                    return Ok((*id, Some(change)));
                }
                return Err(unknown_intent(project, disk, *id));
            }
            disk.intents
                .contains_key(id)
                .then_some((*id, None))
                .ok_or_else(|| unknown_intent(project, disk, *id))
        }
        EntityRef::Scenario(id) => {
            if let Some((intent, _)) = disk.scenario(*id) {
                if let Some(change) = change_touching_intent(&project.parsed, intent.id) {
                    if staged_intents(change)
                        .get(&intent.id)
                        .is_some_and(|staged| staged.scenarios.iter().any(|s| s.id == *id))
                    {
                        return Ok((intent.id, Some(change)));
                    }
                    return Err(unknown_scenario(project, disk, *id));
                }
                return Ok((intent.id, None));
            }
            for change in &project.parsed {
                if let Some(intent) = staged_intents(change)
                    .values()
                    .find(|intent| intent.scenarios.iter().any(|s| s.id == *id))
                {
                    return Ok((intent.id, Some(change)));
                }
            }
            Err(unknown_scenario(project, disk, *id))
        }
        EntityRef::Notion(_) | EntityRef::Constraint(_) | EntityRef::Change(_) => {
            Err(not_applicable())
        }
    }
}

/// The post-state of every intent one change stages, keyed by id: what
/// `add`/`edit intent` insert and `remove intent` withdraws, applied in
/// staged order -- the same construction `telos test`'s `staged_intents`
/// uses one level down, kept here as its own copy since the two commands
/// read it for different questions (which scenarios owe a witness there,
/// which intent a scenario belongs to here).
fn staged_intents(change: &Change) -> std::collections::BTreeMap<IntentId, Intent> {
    let mut intents = std::collections::BTreeMap::new();
    for op in &change.ops {
        match op {
            StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => {
                intents.insert(i.id, i.clone());
            }
            StagedOp::RemoveIntent(id) => {
                intents.remove(id);
            }
            _ => {}
        }
    }
    intents
}

/// Every intent id the disk model or any open change's overlay declares --
/// the nearest-id hint's search space for an unknown `INT-…` argument.
fn known_intent_ids(project: &Project, disk: &TelosModel) -> BTreeSet<IntentId> {
    let mut known: BTreeSet<IntentId> = disk.intents.keys().copied().collect();
    for change in &project.parsed {
        let staged = staged_intents(change);
        let touched = touched_intent_ids(change);
        known.retain(|id| !touched.contains(id));
        known.extend(staged.keys().copied());
    }
    known
}

/// Every scenario id the disk model or any open change's overlay declares.
fn known_scenario_ids(project: &Project, disk: &TelosModel) -> BTreeSet<ScenarioId> {
    let mut known: std::collections::BTreeMap<ScenarioId, IntentId> = disk
        .scenario_owner
        .iter()
        .map(|(scenario, intent)| (*scenario, *intent))
        .collect();
    for change in &project.parsed {
        let staged = staged_intents(change);
        let touched = touched_intent_ids(change);
        known.retain(|_, owner| !touched.contains(owner));
        for intent in staged.values() {
            known.extend(intent.scenarios.iter().map(|s| (s.id, intent.id)));
        }
    }
    known.into_keys().collect()
}

fn unknown_intent(project: &Project, disk: &TelosModel, id: IntentId) -> TelosError {
    let known = known_intent_ids(project, disk);
    let hint = nearest_id(id.0, known.iter().map(|i| i.0), |n| IntentId(n).to_string());
    unknown("intent", id, hint)
}

fn unknown_scenario(project: &Project, disk: &TelosModel, id: ScenarioId) -> TelosError {
    let known = known_scenario_ids(project, disk);
    let hint = nearest_id(id.0, known.iter().map(|s| s.0), |n| {
        ScenarioId(n).to_string()
    });
    unknown("scenario", id, hint)
}

// --- which model the pack is built on (D10) ---------------------------------

/// The change that controls an intent's effective existence. A final
/// `RemoveIntent` masks the disk copy just as an `AddIntent` or `EditIntent`
/// replaces it, so resolution must select this change before it falls back to
/// the sealed model.
fn change_touching_intent(changes: &[Change], intent: IntentId) -> Option<&Change> {
    changes
        .iter()
        .find(|change| touched_intent_ids(change).contains(&intent))
}

/// Every intent a change's final overlay may replace or remove. Removing
/// these ids from the sealed suggestion candidates before adding the final
/// staged intents is what prevents an already-removed id from suggesting
/// itself back to the caller.
fn touched_intent_ids(change: &Change) -> BTreeSet<IntentId> {
    change
        .ops
        .iter()
        .filter_map(|op| match op {
            StagedOp::AddIntent(intent) | StagedOp::EditIntent(intent) => Some(intent.id),
            StagedOp::RemoveIntent(intent) => Some(*intent),
            _ => None,
        })
        .collect()
}

/// The change's post model: its ops replayed idempotently over the sealed
/// base, its journal folded into bindings, then the semantic pass -- the
/// exact construction `reconcile` itself builds (D2), so a pack read
/// mid-change never disagrees with what reconciling it would seal.
fn post_model(project: &Project, change: &Change) -> Result<TelosModel, TelosError> {
    let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;
    let folded = fold_journal_bindings(apply_ops_idempotent(base, &change.ops), change);
    build_model(folded).map_err(diagnostics_to_error)
}

// --- assembling the pack -----------------------------------------------------

struct ScenarioEntry {
    id: ScenarioId,
    title: String,
    proved: bool,
}

struct NotionEntry {
    name: NotionName,
    canonical: String,
}

struct ConstraintEntry {
    id: ConstraintId,
    scope: &'static str,
    canonical: String,
}

struct ProvesEntry {
    scenario: ScenarioId,
    test: String,
}

struct NeighborEntry {
    id: IntentId,
    title: String,
    rel: Relation,
    direction: &'static str,
}

/// The whole pack of Annex C, already sorted and filtered per its rules --
/// `to_json` and `to_human` each render the same data, once computed here.
struct Pack {
    id: IntentId,
    change: Option<ChangeId>,
    canonical: String,
    scenarios: Vec<ScenarioEntry>,
    notions: Vec<NotionEntry>,
    constraints: Vec<ConstraintEntry>,
    implements: Vec<RepoPath>,
    proves: Vec<ProvesEntry>,
    neighbors: Vec<NeighborEntry>,
}

fn build_pack(model: &TelosModel, intent: &Intent, change: Option<ChangeId>) -> Pack {
    Pack {
        id: intent.id,
        change,
        canonical: emit_intent(intent),
        scenarios: scenario_entries(model, intent),
        notions: notion_entries(model, intent),
        constraints: applicable_constraints(model, intent.id)
            .into_iter()
            .map(|entry| ConstraintEntry {
                id: entry.constraint.id,
                scope: entry.scope,
                canonical: emit_constraint(entry.constraint),
            })
            .collect(),
        implements: implementations(model, intent.id),
        proves: proofs(model, intent)
            .into_iter()
            .map(|entry| ProvesEntry {
                scenario: entry.scenario,
                test: entry.test,
            })
            .collect(),
        neighbors: neighbor_entries(model, intent.id),
    }
}

/// Scenarios in the intent's own order (Annex C: not sorted, since an
/// `Intent`'s own `scenarios` already are, by id). `proved` is whether the
/// pack's own model -- journal folded, for an overlay pack -- holds a
/// `Proves` binding for it.
fn scenario_entries(model: &TelosModel, intent: &Intent) -> Vec<ScenarioEntry> {
    intent
        .scenarios
        .iter()
        .map(|s| ScenarioEntry {
            id: s.id,
            title: s.title.clone(),
            proved: model
                .bindings
                .iter()
                .any(|b| matches!(b, Binding::Proves { scenario, .. } if scenario.node == s.id)),
        })
        .collect()
}

/// The notions the intent's statement or any of its scenarios use, sorted
/// by name (Annex C) -- read straight off the graph's derived `uses` edges
/// rather than re-walking the statement/scenario notion-collectors
/// `overlay.rs`'s referrer scan does: the semantic pass already built
/// exactly this set, once, for both the intent's own node and each
/// scenario's.
fn notion_entries(model: &TelosModel, intent: &Intent) -> Vec<NotionEntry> {
    let mut names: BTreeSet<NotionName> = BTreeSet::new();
    collect_uses(model, &NodeRef::Intent(intent.id), &mut names);
    for scenario in &intent.scenarios {
        collect_uses(model, &NodeRef::Scenario(scenario.id), &mut names);
    }
    names
        .into_iter()
        .map(|name| {
            let notion = model
                .notions
                .get(&name)
                .expect("a `uses` edge's target always resolves in a built model");
            NotionEntry {
                canonical: emit_notion(notion),
                name,
            }
        })
        .collect()
}

fn collect_uses(model: &TelosModel, node: &NodeRef, out: &mut BTreeSet<NotionName>) {
    for (rel, to) in model.graph.out_edges(node) {
        if *rel == Relation::Uses
            && let NodeRef::Notion(name) = to
        {
            out.insert(name.clone());
        }
    }
}

/// The intent's 1-hop intent-to-intent neighbours over `refines`/`requires`/
/// `excludes`, both directions, sorted by `(relation, id)` (Annex C).
///
/// `constrains`, `uses`, `implements` and `proves` edges also touch this
/// node in the graph, which is exactly why this filters by relation rather
/// than simply taking every edge: a neighbour is another *intent*, and only
/// three of the eight relations ever connect two of them.
fn neighbor_entries(model: &TelosModel, intent_id: IntentId) -> Vec<NeighborEntry> {
    let node = NodeRef::Intent(intent_id);
    let mut entries: Vec<NeighborEntry> = Vec::new();
    for (rel, to) in model.graph.out_edges(&node) {
        if is_intent_relation(*rel)
            && let NodeRef::Intent(id) = to
        {
            entries.push(neighbor(model, *id, *rel, "out"));
        }
    }
    for (rel, from) in model.graph.in_edges(&node) {
        if is_intent_relation(*rel)
            && let NodeRef::Intent(id) = from
        {
            entries.push(neighbor(model, *id, *rel, "in"));
        }
    }
    entries.sort_by(|a, b| (a.rel, a.id, a.direction).cmp(&(b.rel, b.id, b.direction)));
    entries
}

fn is_intent_relation(rel: Relation) -> bool {
    matches!(
        rel,
        Relation::Refines | Relation::Requires | Relation::Excludes
    )
}

fn neighbor(
    model: &TelosModel,
    id: IntentId,
    rel: Relation,
    direction: &'static str,
) -> NeighborEntry {
    let title = model
        .intents
        .get(&id)
        .map(|i| i.title.clone())
        .unwrap_or_default();
    NeighborEntry {
        id,
        title,
        rel,
        direction,
    }
}

// --- rendering ---------------------------------------------------------------

fn to_json(pack: &Pack) -> Value {
    json!({
        "id": pack.id,
        "change": pack.change,
        "canonical": pack.canonical,
        "scenarios": pack.scenarios.iter().map(|s| json!({
            "id": s.id, "title": s.title, "proved": s.proved,
        })).collect::<Vec<_>>(),
        "notions": pack.notions.iter().map(|n| json!({
            "name": n.name, "canonical": n.canonical,
        })).collect::<Vec<_>>(),
        "constraints": pack.constraints.iter().map(|c| json!({
            "id": c.id, "scope": c.scope, "canonical": c.canonical,
        })).collect::<Vec<_>>(),
        "bindings": {
            "implements": pack.implements,
            "proves": pack.proves.iter().map(|p| json!({
                "scenario": p.scenario, "test": p.test,
            })).collect::<Vec<_>>(),
        },
        "neighbors": pack.neighbors.iter().map(|n| json!({
            "id": n.id, "title": n.title, "rel": n.rel.as_str(), "direction": n.direction,
        })).collect::<Vec<_>>(),
    })
}

/// Readable sections, terse, reusing the same canonical blocks `show`
/// prints byte for byte: the intent, which change (if any) the pack came
/// from, then one heading per Annex C list.
fn to_human(pack: &Pack) -> String {
    let mut lines = vec![pack.canonical.clone()];
    lines.push(match pack.change {
        Some(id) => format!("change: {id}"),
        None => "change: none".to_string(),
    });

    lines.push("scenarios:".to_string());
    for s in &pack.scenarios {
        let verdict = if s.proved { "proved" } else { "not proved" };
        lines.push(format!("  {} {} ({verdict})", s.id, s.title));
    }

    lines.push("notions:".to_string());
    for n in &pack.notions {
        lines.push(format!("  {}", n.name));
    }

    lines.push("constraints:".to_string());
    for c in &pack.constraints {
        lines.push(format!("  {} ({})", c.id, c.scope));
    }

    lines.push("bindings:".to_string());
    for path in &pack.implements {
        lines.push(format!("  implements {path}"));
    }
    for p in &pack.proves {
        lines.push(format!("  proves {} -> {}", p.scenario, p.test));
    }

    lines.push("neighbors:".to_string());
    for n in &pack.neighbors {
        let arrow = if n.direction == "out" { "->" } else { "<-" };
        lines.push(format!("  {arrow} {} {}  {}", n.rel, n.id, n.title));
    }

    lines.join("\n")
}
