//! The in-memory data model: one parsed `.tel` file (`TelFile`), and the
//! whole-spec aggregate built from all of them (`TelosModel`).
//!
//! `TelosModel` is partial here: `graph` (the relation graph), `sources`
//! (file -> entity map), and the `resolve()` method are added in Task 8,
//! once `graph.rs` exists. Only `notions`, `intents`, `constraints`,
//! `bindings`, `scenario_owner`, and `scenario()` are implemented now.

pub mod binding;
pub mod constraint;
pub mod expr;
pub mod intent;
pub mod notion;
pub mod scenario;

pub use binding::{Binding, TestRef};
pub use constraint::{Constraint, ConstraintKind, Rule, Scope};
pub use expr::{AttrRef, CmpOp, Expr, Literal, Operand};
pub use intent::{Action, Intent, IntentStatus, Statement};
pub use notion::{Attr, AttrType, Notion, NotionKind, Rel};
pub use scenario::{InstanceStep, Scenario};

use std::collections::BTreeMap;

use serde::Serialize;

use crate::ids::{ConstraintId, IntentId, NotionName, ScenarioId};

/// One parsed `.tel` file's content, before it is folded into a
/// `TelosModel` by the semantic pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TelFile {
    Notion(Notion),
    Intent(Intent),
    Constraint(Constraint),
    Bindings(Vec<Binding>),
}

/// The whole spec, aggregated from every `.tel` file in the workspace.
///
/// Partial for now -- see the module doc comment.
#[derive(Debug, Default)]
pub struct TelosModel {
    pub notions: BTreeMap<NotionName, Notion>,
    pub intents: BTreeMap<IntentId, Intent>,
    pub constraints: BTreeMap<ConstraintId, Constraint>,
    pub bindings: Vec<Binding>,
    /// The structural `verifies` relation: which intent owns which scenario.
    pub scenario_owner: BTreeMap<ScenarioId, IntentId>,
}

impl TelosModel {
    /// Looks up a scenario by id, together with the intent that owns it.
    pub fn scenario(&self, id: ScenarioId) -> Option<(&Intent, &Scenario)> {
        let intent_id = self.scenario_owner.get(&id)?;
        let intent = self.intents.get(intent_id)?;
        let scenario = intent.scenarios.iter().find(|s| s.id == id)?;
        Some((intent, scenario))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_scenario(id: u32) -> Scenario {
        Scenario {
            id: ScenarioId(id),
            title: "sample".to_string(),
            given: vec![],
            when: InstanceStep {
                notion: crate::span::Sp {
                    node: NotionName::new("Paid").unwrap(),
                    span: crate::span::Span::default(),
                },
                fields: vec![],
            },
            then: vec![],
        }
    }

    fn sample_intent(id: u32, scenarios: Vec<Scenario>) -> Intent {
        Intent {
            id: IntentId(id),
            title: "sample intent".to_string(),
            status: IntentStatus::Active,
            telos: "because".to_string(),
            statement: Statement::Ubiquitous {
                action: Action::Free("do it".to_string()),
            },
            refines: vec![],
            requires: vec![],
            excludes: vec![],
            scenarios,
        }
    }

    #[test]
    fn scenario_looks_up_the_owning_intent_and_the_scenario() {
        let scenario = sample_scenario(107);
        let intent = sample_intent(42, vec![scenario.clone()]);

        let mut model = TelosModel::default();
        model.intents.insert(IntentId(42), intent.clone());
        model.scenario_owner.insert(ScenarioId(107), IntentId(42));

        let (found_intent, found_scenario) = model.scenario(ScenarioId(107)).unwrap();
        assert_eq!(found_intent.id, IntentId(42));
        assert_eq!(found_scenario.id, ScenarioId(107));
        assert_eq!(*found_scenario, scenario);
    }

    #[test]
    fn scenario_returns_none_for_an_unknown_id() {
        let model = TelosModel::default();
        assert_eq!(model.scenario(ScenarioId(999)), None);
    }

    #[test]
    fn scenario_returns_none_when_owner_intent_is_missing() {
        // scenario_owner points at an intent that was never inserted --
        // must not panic, just report not found.
        let mut model = TelosModel::default();
        model.scenario_owner.insert(ScenarioId(1), IntentId(1));
        assert_eq!(model.scenario(ScenarioId(1)), None);
    }

    #[test]
    fn tel_file_wraps_each_parsed_kind() {
        let notion = Notion {
            name: NotionName::new("Invoice").unwrap(),
            kind: NotionKind::Entity,
            def: "A billing document.".to_string(),
            attrs: vec![],
            rels: vec![],
        };
        let file = TelFile::Notion(notion.clone());
        assert_eq!(file, TelFile::Notion(notion));
    }
}
