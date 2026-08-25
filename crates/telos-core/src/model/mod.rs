//! The in-memory data model: one parsed `.tel` file (`TelFile`), and the
//! whole-spec aggregate built from all of them (`TelosModel`).
//!
//! A `TelosModel` is only ever produced by [`crate::semantic::build_model`],
//! which is what fills `scenario_owner`, `graph` and `sources` and what
//! guarantees that every reference inside it resolves.
//!
//! [`change::Change`] lives here too but is deliberately *not* a `TelFile`
//! variant: a change file is its own kind, parsed and stored by its own
//! store, and is never folded into a `TelosModel` -- it describes a pending
//! mutation *of* the spec, not a part of it.

pub mod binding;
pub mod change;
pub mod constraint;
pub mod domain;
pub mod expr;
pub mod intent;
pub mod notion;
pub mod scenario;

pub use binding::{Binding, TestRef};
pub use change::{
    Change, ChangeStatus, JournalEntry, StagedOp, TestRun, Witness, constraint_path, intent_path,
    notion_path,
};
pub use constraint::{Constraint, ConstraintKind, Rule, Scope};
pub use domain::{Capability, Context, ContextDependency, ContextKind, ContextMap, NotionMapping};
pub use expr::{AttrRef, CmpOp, Expr, Literal, Operand};
pub use intent::{Action, Intent, IntentStatus, Statement};
pub use notion::{Attr, AttrType, Notion, NotionKind, Rel};
pub use scenario::{InstanceStep, Scenario};

use std::collections::BTreeMap;

use serde::Serialize;

use crate::graph::{Graph, NodeRef};
use crate::ids::{
    CapabilityRef, ConstraintId, ContextId, EntityRef, IntentId, NotionName, NotionRef, Owner,
    RepoPath, ScenarioId,
};

/// One parsed `.tel` file's content, before it is folded into a
/// `TelosModel` by the semantic pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TelFile {
    Context(Context),
    Capability(Capability),
    OwnedNotion {
        owner: Owner,
        notion: Notion,
    },
    OwnedIntent {
        owner: Owner,
        intent: Intent,
    },
    OwnedConstraint {
        owner: Option<Owner>,
        constraint: Constraint,
    },
    ContextBindings {
        context: ContextId,
        bindings: Vec<Binding>,
    },
    ContextMap(ContextMap),
    /// Internal, unowned forms retained for isolated parser/overlay unit
    /// fixtures. The workspace never accepts their legacy filesystem layout.
    Notion(Notion),
    Intent(Intent),
    Constraint(Constraint),
    Bindings(Vec<Binding>),
}

/// Which entity a spec file declared.
///
/// One file declares exactly one entity, `bindings.tel`
/// excepted -- it declares a list, so it names no entity. Recorded per file
/// so that diagnostics can point at the file a duplicate came from, and so
/// that the seal knows what each sealed path stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    Context(ContextId),
    Capability(CapabilityRef),
    QualifiedNotion(NotionRef),
    Notion(NotionName),
    Intent(IntentId),
    Constraint(ConstraintId),
    ContextMap,
    Bindings,
}

/// The whole spec, aggregated from every `.tel` file in the workspace.
#[derive(Debug, Default)]
pub struct TelosModel {
    pub contexts: BTreeMap<ContextId, Context>,
    pub capabilities: BTreeMap<CapabilityRef, Capability>,
    pub context_map: ContextMap,
    /// Canonical qualified glossary. The legacy flat map remains an internal
    /// compatibility aid for in-memory unit fixtures only.
    pub domain_notions: BTreeMap<NotionRef, Notion>,
    pub notion_owners: BTreeMap<NotionRef, Owner>,
    pub notions: BTreeMap<NotionName, Notion>,
    pub intents: BTreeMap<IntentId, Intent>,
    pub intent_owners: BTreeMap<IntentId, Owner>,
    pub constraints: BTreeMap<ConstraintId, Constraint>,
    pub constraint_owners: BTreeMap<ConstraintId, Option<Owner>>,
    pub bindings: Vec<Binding>,
    pub binding_contexts: Vec<ContextId>,
    /// The structural `verifies` relation: which intent owns which scenario.
    pub scenario_owner: BTreeMap<ScenarioId, IntentId>,
    /// Every relation between two entities, declared or derived.
    pub graph: Graph,
    /// Which file declared which entity.
    pub sources: BTreeMap<RepoPath, SourceKind>,
}

impl TelosModel {
    /// Looks up a scenario by id, together with the intent that owns it.
    pub fn scenario(&self, id: ScenarioId) -> Option<(&Intent, &Scenario)> {
        let intent_id = self.scenario_owner.get(&id)?;
        let intent = self.intents.get(intent_id)?;
        let scenario = intent.scenarios.iter().find(|s| s.id == id)?;
        Some((intent, scenario))
    }

    /// Turns a `show`/`impact` argument into the graph node it names, or
    /// `None` when the spec holds no such entity.
    ///
    /// `EntityRef::Change` always resolves to `None`: changes are transaction
    /// records rather than model nodes, and `NodeRef` has no variant for them.
    pub fn resolve(&self, r: &EntityRef) -> Option<NodeRef> {
        match r {
            EntityRef::Context(id) => self
                .contexts
                .contains_key(id)
                .then(|| NodeRef::Context(id.clone())),
            EntityRef::Capability(id) => self
                .capabilities
                .contains_key(id)
                .then(|| NodeRef::Capability(id.clone())),
            EntityRef::Notion(name) => self
                .domain_notions
                .contains_key(name)
                .then(|| NodeRef::QualifiedNotion(name.clone())),
            EntityRef::Intent(id) => self
                .intents
                .contains_key(id)
                .then_some(NodeRef::Intent(*id)),
            EntityRef::Scenario(id) => self
                .scenario(*id)
                .is_some()
                .then_some(NodeRef::Scenario(*id)),
            EntityRef::Constraint(id) => self
                .constraints
                .contains_key(id)
                .then_some(NodeRef::Constraint(*id)),
            EntityRef::Change(_) => None,
        }
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

    // --- resolve ---------------------------------------------------------

    fn model_with_one_of_each() -> TelosModel {
        let mut model = TelosModel::default();
        let notion = Notion {
            name: NotionName::new("Invoice").unwrap(),
            kind: NotionKind::Entity,
            def: "A bill.".to_string(),
            phrase: "invoice".to_string(),
            attrs: vec![],
            rels: vec![],
        };
        model
            .notions
            .insert(NotionName::new("Invoice").unwrap(), notion.clone());
        model.domain_notions.insert(
            NotionRef::new(
                ContextId::new("billing").unwrap(),
                NotionName::new("Invoice").unwrap(),
            ),
            notion,
        );
        model
            .intents
            .insert(IntentId(42), sample_intent(42, vec![sample_scenario(107)]));
        model.scenario_owner.insert(ScenarioId(107), IntentId(42));
        model.constraints.insert(
            ConstraintId(3),
            Constraint {
                id: ConstraintId(3),
                kind: ConstraintKind::Architecture,
                title: "Boundaries".to_string(),
                rule: Rule::Text("Keep them.".to_string()),
                scope: Scope::Global,
                check: None,
            },
        );
        model
    }

    #[test]
    fn resolve_maps_every_entity_kind_to_its_node() {
        let model = model_with_one_of_each();
        let invoice = NotionRef::new(
            ContextId::new("billing").unwrap(),
            NotionName::new("Invoice").unwrap(),
        );
        assert_eq!(
            model.resolve(&EntityRef::Notion(invoice.clone())),
            Some(NodeRef::QualifiedNotion(invoice))
        );
        assert_eq!(
            model.resolve(&EntityRef::Intent(IntentId(42))),
            Some(NodeRef::Intent(IntentId(42)))
        );
        assert_eq!(
            model.resolve(&EntityRef::Scenario(ScenarioId(107))),
            Some(NodeRef::Scenario(ScenarioId(107)))
        );
        assert_eq!(
            model.resolve(&EntityRef::Constraint(ConstraintId(3))),
            Some(NodeRef::Constraint(ConstraintId(3)))
        );
    }

    #[test]
    fn resolve_returns_none_for_an_entity_the_spec_does_not_hold() {
        let model = model_with_one_of_each();
        assert_eq!(
            model.resolve(&EntityRef::Notion(NotionRef::new(
                ContextId::new("billing").unwrap(),
                NotionName::new("Rogue").unwrap(),
            ))),
            None
        );
        assert_eq!(model.resolve(&EntityRef::Intent(IntentId(1))), None);
        assert_eq!(model.resolve(&EntityRef::Scenario(ScenarioId(1))), None);
        assert_eq!(model.resolve(&EntityRef::Constraint(ConstraintId(1))), None);
    }

    #[test]
    fn resolve_never_resolves_a_change() {
        // `NodeRef` has no change variant: transaction records stay outside
        // the specification graph.
        let model = model_with_one_of_each();
        assert_eq!(
            model.resolve(&EntityRef::Change(crate::ids::ChangeId(7))),
            None
        );
    }

    #[test]
    fn resolve_ignores_a_scenario_whose_owner_is_missing() {
        let mut model = TelosModel::default();
        model.scenario_owner.insert(ScenarioId(1), IntentId(1));
        assert_eq!(model.resolve(&EntityRef::Scenario(ScenarioId(1))), None);
    }

    #[test]
    fn tel_file_wraps_each_parsed_kind() {
        let notion = Notion {
            name: NotionName::new("Invoice").unwrap(),
            kind: NotionKind::Entity,
            def: "A billing document.".to_string(),
            phrase: "invoice".to_string(),
            attrs: vec![],
            rels: vec![],
        };
        let file = TelFile::Notion(notion.clone());
        assert_eq!(file, TelFile::Notion(notion));
    }
}
