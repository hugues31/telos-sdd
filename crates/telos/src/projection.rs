//! Deterministic, bounded projections shared by CLI context and web views.

use std::collections::BTreeSet;

use telos_core::ids::{IntentId, RepoPath, ScenarioId};
use telos_core::model::{Binding, Constraint, Intent, Scope, TelosModel};

pub(crate) struct ApplicableConstraint<'a> {
    pub(crate) constraint: &'a Constraint,
    pub(crate) scope: &'static str,
}

pub(crate) struct Proof {
    pub(crate) scenario: ScenarioId,
    pub(crate) test: String,
}

/// Global constraints and constraints scoped to `intent_id`, in id order.
pub(crate) fn applicable_constraints(
    model: &TelosModel,
    intent_id: IntentId,
) -> Vec<ApplicableConstraint<'_>> {
    model
        .constraints
        .values()
        .filter_map(|constraint| {
            let scope = match &constraint.scope {
                Scope::Global => "global",
                Scope::Intents(ids) if ids.iter().any(|id| id.node == intent_id) => "scoped",
                Scope::Intents(_) => return None,
            };
            Some(ApplicableConstraint { constraint, scope })
        })
        .collect()
}

/// Source paths implementing one intent, in lexical path order.
pub(crate) fn implementations(model: &TelosModel, intent_id: IntentId) -> Vec<RepoPath> {
    let mut paths: Vec<RepoPath> = model
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Implements { path, intent } if intent.node == intent_id => Some(path.clone()),
            _ => None,
        })
        .collect();
    paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    paths
}

/// Test locators proving scenarios owned by `intent`, sorted by scenario then test.
pub(crate) fn proofs(model: &TelosModel, intent: &Intent) -> Vec<Proof> {
    let scenario_ids: BTreeSet<ScenarioId> = intent
        .scenarios
        .iter()
        .map(|scenario| scenario.id)
        .collect();
    let mut entries: Vec<Proof> = model
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Proves { test, scenario } if scenario_ids.contains(&scenario.node) => {
                Some(Proof {
                    scenario: scenario.node,
                    test: test.to_string(),
                })
            }
            _ => None,
        })
        .collect();
    entries.sort_by(|a, b| (a.scenario, &a.test).cmp(&(b.scenario, &b.test)));
    entries
}
