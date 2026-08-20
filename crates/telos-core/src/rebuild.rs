//! Deterministic prerequisite-first reconstruction planning.

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::IntentId;
use crate::model::TelosModel;

/// One intent in a reconstruction plan, with its direct prerequisites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildStep {
    pub intent: IntentId,
    pub requires: Vec<IntentId>,
}

/// Returns every intent in prerequisite-first order.
///
/// A [`TelosModel`] has already passed semantic validation, so all
/// prerequisites resolve and `requires` contains no cycle.
pub fn plan(model: &TelosModel) -> Vec<RebuildStep> {
    let mut prerequisite_counts = BTreeMap::new();
    let mut dependents: BTreeMap<IntentId, BTreeSet<IntentId>> = BTreeMap::new();
    let mut direct_requires = BTreeMap::new();

    for intent in model.intents.values() {
        let requires: BTreeSet<IntentId> = intent.requires.iter().map(|id| id.node).collect();
        prerequisite_counts.insert(intent.id, requires.len());
        direct_requires.insert(intent.id, requires.iter().copied().collect::<Vec<_>>());
        for prerequisite in requires {
            dependents
                .entry(prerequisite)
                .or_default()
                .insert(intent.id);
        }
    }

    let mut ready: BTreeSet<IntentId> = prerequisite_counts
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    let mut steps = Vec::with_capacity(model.intents.len());

    while let Some(id) = ready.pop_first() {
        steps.push(RebuildStep {
            intent: id,
            requires: direct_requires.remove(&id).unwrap_or_default(),
        });

        for dependent in dependents.get(&id).into_iter().flatten() {
            let count = prerequisite_counts
                .get_mut(dependent)
                .expect("a validated prerequisite names a known intent");
            *count -= 1;
            if *count == 0 {
                ready.insert(*dependent);
            }
        }
    }

    debug_assert_eq!(steps.len(), model.intents.len());
    steps
}
