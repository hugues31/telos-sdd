use std::collections::BTreeSet;

use serde::Serialize;
use telos_core::emit::{
    emit_owned_constraint, emit_owned_intent, emit_owned_notion, emit_scenario_fragment,
    emit_statement_fragment, statement_template,
};
use telos_core::graph::{NodeRef, Relation};
use telos_core::ids::Owner;
use telos_core::model::{
    Binding, ConstraintKind, ContextKind, IntentStatus, NotionKind, Scope, TelosModel,
};
use telos_core::state::{DriftKind, ProjectStateKind, StateReport, coverage as model_coverage};

use crate::projection::{applicable_constraints, implementations, proofs};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ViewSnapshot {
    pub(crate) dashboard: DashboardView,
    pub(crate) coverage: CoverageView,
    pub(crate) contexts: Vec<ContextView>,
    pub(crate) notions: Vec<NotionView>,
    pub(crate) intents: Vec<IntentView>,
    pub(crate) scenarios: Vec<ScenarioView>,
    pub(crate) constraints: Vec<ConstraintView>,
    pub(crate) implementations: Vec<ImplementationView>,
    pub(crate) proofs: Vec<ProofView>,
    pub(crate) nodes: Vec<GraphNodeView>,
    pub(crate) edges: Vec<GraphEdgeView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DashboardView {
    pub(crate) state: String,
    pub(crate) drift: Vec<DriftView>,
    pub(crate) open_changes: Vec<OpenChangeView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DriftView {
    pub(crate) path: String,
    pub(crate) kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OpenChangeView {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CoverageView {
    pub(crate) notions: u32,
    pub(crate) constraints: u32,
    pub(crate) intents_total: u32,
    pub(crate) intents_active: u32,
    pub(crate) intents_implemented: u32,
    pub(crate) scenarios_total: u32,
    pub(crate) scenarios_proved: u32,
    pub(crate) rows: Vec<CoverageRowView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CoverageRowView {
    pub(crate) intent: String,
    pub(crate) scenario: String,
    pub(crate) test: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NotionView {
    pub(crate) name: String,
    pub(crate) owner: String,
    pub(crate) kind: String,
    pub(crate) definition: String,
    pub(crate) canonical: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct IntentView {
    pub(crate) id: String,
    pub(crate) owner: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) telos: String,
    pub(crate) canonical: String,
    pub(crate) statement: StatementView,
    pub(crate) notions: Vec<String>,
    pub(crate) constraints: Vec<ConstraintRefView>,
    pub(crate) implements: Vec<String>,
    pub(crate) scenarios: Vec<ScenarioView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StatementView {
    pub(crate) template: String,
    pub(crate) canonical: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ScenarioView {
    pub(crate) id: String,
    pub(crate) intent: String,
    pub(crate) title: String,
    pub(crate) canonical: String,
    pub(crate) notions: Vec<String>,
    pub(crate) proves: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ConstraintRefView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) scope: String,
    pub(crate) canonical: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ConstraintView {
    pub(crate) id: String,
    pub(crate) owner: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) scope: String,
    pub(crate) canonical: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ContextView {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) definition: String,
    pub(crate) capabilities: Vec<CapabilityView>,
    pub(crate) dependencies: Vec<ContextDependencyView>,
    pub(crate) health: ContextHealthView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CapabilityView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ContextDependencyView {
    pub(crate) supplier: String,
    pub(crate) mappings: Vec<NotionMappingView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NotionMappingView {
    pub(crate) from: String,
    pub(crate) to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ContextHealthView {
    pub(crate) intents: u32,
    pub(crate) active_intents: u32,
    pub(crate) scenarios: u32,
    pub(crate) proved_scenarios: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct ImplementationView {
    pub(crate) path: String,
    pub(crate) intent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct ProofView {
    pub(crate) test: String,
    pub(crate) scenario: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub(crate) enum GraphKey {
    Context(String),
    Capability(String),
    Notion(String),
    Intent(String),
    Scenario(String),
    Constraint(String),
    Code(String),
    Test(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GraphNodeView {
    pub(crate) key: GraphKey,
    pub(crate) label: String,
    pub(crate) parent: Option<GraphKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GraphEdgeView {
    pub(crate) from: GraphKey,
    pub(crate) relation: String,
    pub(crate) to: GraphKey,
}

impl ViewSnapshot {
    pub(crate) fn build(state: &StateReport, model: &TelosModel) -> Self {
        let notions = model
            .domain_notions
            .iter()
            .filter_map(|(reference, notion)| {
                let owner = model.notion_owners.get(reference)?;
                Some(NotionView {
                    name: reference.to_string(),
                    owner: owner.to_string(),
                    kind: notion_kind(notion.kind).to_string(),
                    definition: notion.def.clone(),
                    canonical: emit_owned_notion(owner, notion),
                })
            })
            .collect();

        let mut scenarios = Vec::new();
        let intents: Vec<IntentView> = model
            .intents
            .values()
            .filter_map(|intent| {
                let owner = model.intent_owners.get(&intent.id)?;
                let intent_proofs = proofs(model, intent);
                let scenario_views: Vec<ScenarioView> = intent
                    .scenarios
                    .iter()
                    .map(|scenario| ScenarioView {
                        id: scenario.id.to_string(),
                        intent: intent.id.to_string(),
                        title: scenario.title.clone(),
                        canonical: emit_scenario_fragment(scenario),
                        notions: uses_from(model, &NodeRef::Scenario(scenario.id)),
                        proves: intent_proofs
                            .iter()
                            .filter(|proof| proof.scenario == scenario.id)
                            .map(|proof| proof.test.clone())
                            .collect(),
                    })
                    .collect();
                scenarios.extend(scenario_views.iter().cloned());

                let mut notion_names = uses_from(model, &NodeRef::Intent(intent.id));
                for scenario in &scenario_views {
                    notion_names.extend(scenario.notions.iter().cloned());
                }
                notion_names.sort();
                notion_names.dedup();

                Some(IntentView {
                    id: intent.id.to_string(),
                    owner: owner.to_string(),
                    title: intent.title.clone(),
                    status: intent_status(intent.status).to_string(),
                    telos: intent.telos.clone(),
                    canonical: emit_owned_intent(owner, intent),
                    statement: StatementView {
                        template: statement_template(&intent.statement).to_string(),
                        canonical: emit_statement_fragment(&intent.statement),
                    },
                    notions: notion_names,
                    constraints: applicable_constraints(model, intent.id)
                        .into_iter()
                        .map(|entry| ConstraintRefView {
                            id: entry.constraint.id.to_string(),
                            title: entry.constraint.title.clone(),
                            scope: entry.scope.to_string(),
                            canonical: model
                                .constraint_owners
                                .get(&entry.constraint.id)
                                .map(|owner| {
                                    emit_owned_constraint(owner.as_ref(), entry.constraint)
                                })
                                .unwrap_or_default(),
                        })
                        .collect(),
                    implements: implementations(model, intent.id)
                        .into_iter()
                        .map(|path| path.to_string())
                        .collect(),
                    scenarios: scenario_views,
                })
            })
            .collect();
        scenarios.sort_by(|a, b| a.id.cmp(&b.id));

        let constraints = model
            .constraints
            .values()
            .filter_map(|constraint| {
                let owner = model.constraint_owners.get(&constraint.id)?;
                Some(ConstraintView {
                    id: constraint.id.to_string(),
                    owner: owner
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "project".to_string()),
                    kind: constraint_kind(constraint.kind).to_string(),
                    title: constraint.title.clone(),
                    scope: match &constraint.scope {
                        Scope::Global => "global".to_string(),
                        Scope::Intents(ids) => ids
                            .iter()
                            .map(|id| id.node.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    },
                    canonical: emit_owned_constraint(owner.as_ref(), constraint),
                })
            })
            .collect();

        let contexts = model
            .contexts
            .values()
            .map(|context| {
                let capabilities = model
                    .capabilities
                    .values()
                    .filter(|capability| capability.id.context == context.id)
                    .map(|capability| CapabilityView {
                        id: capability.id.to_string(),
                        title: capability.title.clone(),
                        definition: capability.def.clone(),
                    })
                    .collect();
                let dependencies = model
                    .context_map
                    .dependencies
                    .iter()
                    .filter(|dependency| dependency.consumer == context.id)
                    .map(|dependency| ContextDependencyView {
                        supplier: dependency.supplier.to_string(),
                        mappings: dependency
                            .mappings
                            .iter()
                            .map(|mapping| NotionMappingView {
                                from: mapping.from.to_string(),
                                to: mapping.to.to_string(),
                            })
                            .collect(),
                    })
                    .collect();
                let owned_intents: Vec<_> = model
                    .intents
                    .values()
                    .filter(|intent| {
                        model
                            .intent_owners
                            .get(&intent.id)
                            .is_some_and(|owner| owner.context == context.id)
                    })
                    .collect();
                let scenarios = owned_intents
                    .iter()
                    .map(|intent| intent.scenarios.len() as u32)
                    .sum();
                let proved_scenarios = owned_intents
                    .iter()
                    .flat_map(|intent| &intent.scenarios)
                    .filter(|scenario| {
                        model.bindings.iter().any(|binding| {
                            matches!(binding, Binding::Proves { scenario: held, .. } if held.node == scenario.id)
                        })
                    })
                    .count() as u32;
                ContextView {
                    id: context.id.to_string(),
                    kind: context_kind(context.kind).to_string(),
                    title: context.title.clone(),
                    definition: context.def.clone(),
                    capabilities,
                    dependencies,
                    health: ContextHealthView {
                        intents: owned_intents.len() as u32,
                        active_intents: owned_intents
                            .iter()
                            .filter(|intent| intent.status == IntentStatus::Active)
                            .count() as u32,
                        scenarios,
                        proved_scenarios,
                    },
                }
            })
            .collect();

        let mut implementations = BTreeSet::new();
        let mut proof_views = BTreeSet::new();
        for binding in &model.bindings {
            match binding {
                Binding::Implements { path, intent } => {
                    implementations.insert(ImplementationView {
                        path: path.to_string(),
                        intent: intent.node.to_string(),
                    });
                }
                Binding::Proves { test, scenario } => {
                    proof_views.insert(ProofView {
                        test: test.to_string(),
                        scenario: scenario.node.to_string(),
                    });
                }
            }
        }

        let model_nodes = model_nodes(model);
        let nodes = model_nodes
            .iter()
            .map(|node| graph_node(model, node))
            .collect();
        let mut graph_edges = BTreeSet::new();
        for from in &model_nodes {
            for (relation, to) in model.graph.out_edges(from) {
                graph_edges.insert((from.clone(), *relation, to.clone()));
            }
        }
        let edges = graph_edges
            .into_iter()
            .map(|(from, relation, to)| GraphEdgeView {
                from: GraphKey::from(&from),
                relation: relation.as_str().to_string(),
                to: GraphKey::from(&to),
            })
            .collect();

        let raw_coverage = model_coverage(model);
        let mut rows = Vec::new();
        for intent in &intents {
            for scenario in &intent.scenarios {
                if scenario.proves.is_empty() {
                    rows.push(CoverageRowView {
                        intent: intent.id.clone(),
                        scenario: scenario.id.clone(),
                        test: None,
                    });
                } else {
                    rows.extend(scenario.proves.iter().map(|test| CoverageRowView {
                        intent: intent.id.clone(),
                        scenario: scenario.id.clone(),
                        test: Some(test.clone()),
                    }));
                }
            }
        }
        let coverage = CoverageView {
            notions: raw_coverage.notions,
            constraints: raw_coverage.constraints,
            intents_total: raw_coverage.intents_total,
            intents_active: raw_coverage.intents_active,
            intents_implemented: raw_coverage.intents_implemented,
            scenarios_total: raw_coverage.scenarios_total,
            scenarios_proved: raw_coverage.scenarios_proved,
            rows,
        };

        Self {
            dashboard: DashboardView {
                state: state_kind(state.state).to_string(),
                drift: state
                    .drift
                    .iter()
                    .map(|entry| DriftView {
                        path: entry.path.to_string(),
                        kind: drift_kind(entry.kind).to_string(),
                    })
                    .collect(),
                open_changes: state
                    .open_changes
                    .iter()
                    .map(|change| OpenChangeView {
                        id: change.id.to_string(),
                        status: change.status.clone(),
                        obligations: change.obligations.clone(),
                    })
                    .collect(),
            },
            coverage,
            contexts,
            notions,
            intents,
            scenarios,
            constraints,
            implementations: implementations.into_iter().collect(),
            proofs: proof_views.into_iter().collect(),
            nodes,
            edges,
        }
    }
}

impl From<&NodeRef> for GraphKey {
    fn from(node: &NodeRef) -> Self {
        match node {
            NodeRef::Context(id) => Self::Context(id.to_string()),
            NodeRef::Capability(id) => Self::Capability(id.to_string()),
            NodeRef::QualifiedNotion(name) => Self::Notion(name.to_string()),
            NodeRef::Notion(name) => Self::Notion(name.to_string()),
            NodeRef::Intent(id) => Self::Intent(id.to_string()),
            NodeRef::Scenario(id) => Self::Scenario(id.to_string()),
            NodeRef::Constraint(id) => Self::Constraint(id.to_string()),
            NodeRef::Code(path) => Self::Code(path.to_string()),
            NodeRef::Test(test) => Self::Test(test.clone()),
        }
    }
}

fn model_nodes(model: &TelosModel) -> BTreeSet<NodeRef> {
    let mut nodes = BTreeSet::new();
    nodes.extend(model.contexts.keys().cloned().map(NodeRef::Context));
    nodes.extend(model.capabilities.keys().cloned().map(NodeRef::Capability));
    if model.domain_notions.is_empty() {
        nodes.extend(model.notions.keys().cloned().map(NodeRef::Notion));
    } else {
        nodes.extend(
            model
                .domain_notions
                .keys()
                .cloned()
                .map(NodeRef::QualifiedNotion),
        );
    }
    nodes.extend(model.intents.keys().copied().map(NodeRef::Intent));
    nodes.extend(model.scenario_owner.keys().copied().map(NodeRef::Scenario));
    nodes.extend(model.constraints.keys().copied().map(NodeRef::Constraint));
    for binding in &model.bindings {
        match binding {
            Binding::Implements { path, .. } => {
                nodes.insert(NodeRef::Code(path.clone()));
            }
            Binding::Proves { test, .. } => {
                nodes.insert(NodeRef::Test(test.to_string()));
            }
        }
    }
    nodes
}

fn graph_node(model: &TelosModel, node: &NodeRef) -> GraphNodeView {
    let label = match node {
        NodeRef::Context(id) => model
            .contexts
            .get(id)
            .map(|context| context.title.as_str())
            .unwrap_or(id.as_str()),
        NodeRef::Capability(id) => model
            .capabilities
            .get(id)
            .map(|capability| capability.title.as_str())
            .unwrap_or(""),
        NodeRef::QualifiedNotion(name) => model
            .domain_notions
            .get(name)
            .map(|notion| notion.def.as_str())
            .unwrap_or(name.notion.as_str()),
        NodeRef::Notion(name) => model
            .notions
            .get(name)
            .map(|notion| notion.def.as_str())
            .unwrap_or(name.as_str()),
        NodeRef::Intent(id) => model
            .intents
            .get(id)
            .map(|intent| intent.title.as_str())
            .unwrap_or(""),
        NodeRef::Scenario(id) => model
            .scenario(*id)
            .map(|(_, scenario)| scenario.title.as_str())
            .unwrap_or(""),
        NodeRef::Constraint(id) => model
            .constraints
            .get(id)
            .map(|constraint| constraint.title.as_str())
            .unwrap_or(""),
        NodeRef::Code(path) => path.as_str(),
        NodeRef::Test(test) => test.as_str(),
    };
    GraphNodeView {
        key: GraphKey::from(node),
        label: label.to_string(),
        parent: graph_parent(model, node),
    }
}

fn owner_key(owner: &Owner) -> GraphKey {
    match owner.capability_ref() {
        Some(capability) => GraphKey::Capability(capability.to_string()),
        None => GraphKey::Context(owner.context.to_string()),
    }
}

fn graph_parent(model: &TelosModel, node: &NodeRef) -> Option<GraphKey> {
    match node {
        NodeRef::Capability(id) => Some(GraphKey::Context(id.context.to_string())),
        NodeRef::QualifiedNotion(id) => model.notion_owners.get(id).map(owner_key),
        NodeRef::Intent(id) => model.intent_owners.get(id).map(owner_key),
        NodeRef::Scenario(id) => model
            .scenario_owner
            .get(id)
            .and_then(|intent| model.intent_owners.get(intent))
            .map(owner_key),
        NodeRef::Constraint(id) => model
            .constraint_owners
            .get(id)
            .and_then(Option::as_ref)
            .map(owner_key),
        NodeRef::Context(_) | NodeRef::Notion(_) | NodeRef::Code(_) | NodeRef::Test(_) => None,
    }
}

fn uses_from(model: &TelosModel, node: &NodeRef) -> Vec<String> {
    model
        .graph
        .out_edges(node)
        .iter()
        .filter_map(|(relation, target)| match (relation, target) {
            (Relation::Uses, NodeRef::Notion(name)) => Some(name.to_string()),
            (Relation::Uses, NodeRef::QualifiedNotion(name)) => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

fn state_kind(state: ProjectStateKind) -> &'static str {
    match state {
        ProjectStateKind::Coherent => "coherent",
        ProjectStateKind::Changing => "changing",
        ProjectStateKind::Drifted => "drifted",
    }
}

fn drift_kind(kind: DriftKind) -> &'static str {
    match kind {
        DriftKind::Modified => "modified",
        DriftKind::Missing => "missing",
        DriftKind::Untracked => "untracked",
    }
}

fn intent_status(status: IntentStatus) -> &'static str {
    match status {
        IntentStatus::Draft => "draft",
        IntentStatus::Active => "active",
        IntentStatus::Deprecated => "deprecated",
    }
}

fn notion_kind(kind: NotionKind) -> &'static str {
    match kind {
        NotionKind::Actor => "actor",
        NotionKind::Entity => "entity",
        NotionKind::Value => "value",
        NotionKind::Event => "event",
        NotionKind::State => "state",
    }
}

fn context_kind(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::Core => "core",
        ContextKind::Supporting => "supporting",
        ContextKind::Generic => "generic",
    }
}

fn constraint_kind(kind: ConstraintKind) -> &'static str {
    match kind {
        ConstraintKind::Stack => "stack",
        ConstraintKind::Architecture => "architecture",
        ConstraintKind::Quality => "quality",
        ConstraintKind::Security => "security",
        ConstraintKind::Convention => "convention",
    }
}

#[cfg(test)]
pub(crate) fn all_relations_fixture_model() -> TelosModel {
    use std::fs;
    use std::path::PathBuf;

    use telos_core::workspace::Workspace;

    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing");
    let fixture = tempfile::tempdir().expect("temporary relation fixture");
    for directory in [
        "telos/contexts/billing/notions",
        "telos/contexts/billing/capabilities/invoicing/notions",
        "telos/contexts/billing/capabilities/invoicing/intents",
        "telos/contexts/billing/capabilities/settlement/notions",
        "telos/contexts/billing/capabilities/settlement/intents",
        "telos/contexts/billing/constraints",
    ] {
        fs::create_dir_all(fixture.path().join(directory)).unwrap();
    }
    for relative in [
        "telos/telos.toml",
        "telos/context-map.tel",
        "telos/contexts/billing/context.tel",
        "telos/contexts/billing/bindings.tel",
        "telos/contexts/billing/notions/Customer.tel",
        "telos/contexts/billing/notions/Invoice.tel",
        "telos/contexts/billing/capabilities/invoicing/capability.tel",
        "telos/contexts/billing/capabilities/invoicing/notions/InvoiceIssued.tel",
        "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel",
        "telos/contexts/billing/capabilities/settlement/capability.tel",
        "telos/contexts/billing/capabilities/settlement/notions/PaymentReceived.tel",
    ] {
        fs::copy(source.join(relative), fixture.path().join(relative)).unwrap();
    }
    let intent_path = "telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel";
    let intent = fs::read_to_string(source.join(intent_path))
        .unwrap()
        .replace(
            "  requires INT-0017",
            "  refines INT-0017\n  requires INT-0017\n  excludes INT-0017",
        );
    fs::write(fixture.path().join(intent_path), intent).unwrap();
    let constraint_path = "telos/contexts/billing/constraints/CON-0003.tel";
    fs::copy(
        source.join(constraint_path),
        fixture.path().join(constraint_path),
    )
    .unwrap();

    Workspace::discover(fixture.path())
        .unwrap()
        .load_model()
        .expect("all-relations fixture passes semantic validation")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use telos_core::graph::{NodeRef, Relation};
    use telos_core::ids::{IntentId, RepoPath};
    use telos_core::model::{Binding, TelosModel};
    use telos_core::span::{Sp, Span};
    use telos_core::state::{ProjectStateKind, StateReport};
    use telos_core::workspace::Workspace;

    use super::{GraphKey, ViewSnapshot};

    fn graph_key_id(key: &GraphKey) -> &str {
        match key {
            GraphKey::Context(id)
            | GraphKey::Capability(id)
            | GraphKey::Notion(id)
            | GraphKey::Intent(id)
            | GraphKey::Scenario(id)
            | GraphKey::Constraint(id)
            | GraphKey::Code(id)
            | GraphKey::Test(id) => id,
        }
    }

    fn fixture_model() -> TelosModel {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing");
        let workspace = Workspace::discover(&fixture).expect("Billing workspace is discoverable");
        workspace
            .load_model()
            .expect("Billing fixture is a valid model")
    }

    fn fixture_snapshot() -> ViewSnapshot {
        let model = fixture_model();
        let state = StateReport {
            state: ProjectStateKind::Coherent,
            drift: vec![],
            open_changes: vec![],
        };
        ViewSnapshot::build(&state, &model)
    }

    fn graph_parent(snapshot: &ViewSnapshot, key: GraphKey) -> Option<GraphKey> {
        snapshot
            .nodes
            .iter()
            .find(|node| node.key == key)
            .unwrap_or_else(|| panic!("missing graph node {key:?}"))
            .parent
            .clone()
    }

    #[test]
    fn snapshot_assigns_authoritative_compound_parents() {
        let snapshot = fixture_snapshot();

        assert_eq!(
            graph_parent(&snapshot, GraphKey::Context("billing".to_string())),
            None
        );
        assert_eq!(
            graph_parent(
                &snapshot,
                GraphKey::Capability("billing/invoicing".to_string())
            ),
            Some(GraphKey::Context("billing".to_string()))
        );
        assert_eq!(
            graph_parent(&snapshot, GraphKey::Notion("billing/Customer".to_string())),
            Some(GraphKey::Context("billing".to_string()))
        );
        assert_eq!(
            graph_parent(
                &snapshot,
                GraphKey::Notion("billing/InvoiceIssued".to_string())
            ),
            Some(GraphKey::Capability("billing/invoicing".to_string()))
        );
        assert_eq!(
            graph_parent(&snapshot, GraphKey::Intent("INT-0017".to_string())),
            Some(GraphKey::Capability("billing/invoicing".to_string()))
        );
        assert_eq!(
            graph_parent(&snapshot, GraphKey::Scenario("SCN-0091".to_string())),
            Some(GraphKey::Capability("billing/invoicing".to_string()))
        );
        assert_eq!(
            graph_parent(&snapshot, GraphKey::Constraint("CON-0003".to_string())),
            Some(GraphKey::Context("billing".to_string()))
        );
        assert_eq!(
            graph_parent(
                &snapshot,
                GraphKey::Code("src/billing/invoice.rs".to_string())
            ),
            None
        );
        assert_eq!(
            graph_parent(
                &snapshot,
                GraphKey::Test(
                    "tests/billing.rs::scn_0107_full_payment_settles_the_invoice".to_string()
                )
            ),
            None
        );
    }

    #[test]
    fn snapshot_contains_graph_relations_and_cross_references() {
        let snapshot = fixture_snapshot();

        assert!(
            snapshot
                .nodes
                .iter()
                .any(|node| node.key == GraphKey::Intent("INT-0042".to_string()))
        );
        assert!(snapshot.edges.iter().any(|edge| {
            edge.from == GraphKey::Intent("INT-0042".to_string())
                && edge.relation == "requires"
                && edge.to == GraphKey::Intent("INT-0017".to_string())
        }));

        let intent = snapshot
            .intents
            .iter()
            .find(|intent| intent.id == IntentId(42).to_string())
            .unwrap();
        assert_eq!(intent.implements, ["src/billing/invoice.rs"]);
        assert_eq!(intent.statement.template, "event-driven");
        assert_eq!(
            intent.statement.canonical,
            concat!(
                "  statement event-driven {\n",
                "    when   PaymentReceived on Invoice\n",
                "    system shall set Invoice.state = settled\n",
                "  }\n",
            )
        );
        assert_eq!(intent.scenarios[0].id, "SCN-0107");
        assert_eq!(
            intent.scenarios[0].canonical,
            concat!(
                "  scenario SCN-0107 \"full payment settles the invoice\" {\n",
                "    given Invoice { state: open, balance: \"120.00 EUR\" }\n",
                "    when  PaymentReceived { amount: \"120.00 EUR\" }\n",
                "    then  Invoice.state == settled\n",
                "  }\n",
            )
        );
        assert_eq!(
            intent.scenarios[0].proves,
            ["tests/billing.rs::scn_0107_full_payment_settles_the_invoice"]
        );
        assert!(
            intent
                .constraints
                .iter()
                .any(|constraint| constraint.id == "CON-0003")
        );
    }

    #[test]
    fn snapshot_orders_every_collection_from_literal_billing_values() {
        let snapshot = fixture_snapshot();

        assert_eq!(
            snapshot
                .notions
                .iter()
                .map(|notion| notion.name.as_str())
                .collect::<Vec<_>>(),
            [
                "billing/Customer",
                "billing/Invoice",
                "billing/InvoiceIssued",
                "billing/PaymentReceived",
            ]
        );
        assert_eq!(
            snapshot
                .intents
                .iter()
                .map(|intent| intent.id.as_str())
                .collect::<Vec<_>>(),
            ["INT-0017", "INT-0042"]
        );
        assert_eq!(
            snapshot
                .scenarios
                .iter()
                .map(|scenario| scenario.id.as_str())
                .collect::<Vec<_>>(),
            ["SCN-0091", "SCN-0107"]
        );
        assert_eq!(
            snapshot
                .constraints
                .iter()
                .map(|constraint| constraint.id.as_str())
                .collect::<Vec<_>>(),
            ["CON-0003"]
        );
        assert_eq!(
            snapshot
                .implementations
                .iter()
                .map(|binding| (binding.path.as_str(), binding.intent.as_str()))
                .collect::<Vec<_>>(),
            [("src/billing/invoice.rs", "INT-0042")]
        );
        assert_eq!(
            snapshot
                .proofs
                .iter()
                .map(|binding| (binding.test.as_str(), binding.scenario.as_str()))
                .collect::<Vec<_>>(),
            [(
                "tests/billing.rs::scn_0107_full_payment_settles_the_invoice",
                "SCN-0107"
            )]
        );
        assert_eq!(
            snapshot
                .nodes
                .iter()
                .map(|node| graph_key_id(&node.key))
                .collect::<Vec<_>>(),
            [
                "billing",
                "billing/invoicing",
                "billing/settlement",
                "billing/Customer",
                "billing/Invoice",
                "billing/InvoiceIssued",
                "billing/PaymentReceived",
                "INT-0017",
                "INT-0042",
                "SCN-0091",
                "SCN-0107",
                "CON-0003",
                "src/billing/invoice.rs",
                "tests/billing.rs::scn_0107_full_payment_settles_the_invoice",
            ]
        );
        assert_eq!(
            snapshot
                .edges
                .iter()
                .map(|edge| {
                    (
                        graph_key_id(&edge.from),
                        edge.relation.as_str(),
                        graph_key_id(&edge.to),
                    )
                })
                .collect::<Vec<_>>(),
            [
                ("billing/invoicing", "belongs-to", "billing"),
                ("billing/settlement", "belongs-to", "billing"),
                ("billing/Customer", "belongs-to", "billing"),
                ("billing/Invoice", "belongs-to", "billing"),
                ("billing/InvoiceIssued", "belongs-to", "billing/invoicing"),
                (
                    "billing/PaymentReceived",
                    "belongs-to",
                    "billing/settlement"
                ),
                ("INT-0017", "belongs-to", "billing/invoicing"),
                ("INT-0017", "uses", "billing/Invoice"),
                ("INT-0017", "uses", "billing/InvoiceIssued"),
                ("INT-0042", "belongs-to", "billing/settlement"),
                ("INT-0042", "requires", "INT-0017"),
                ("INT-0042", "uses", "billing/Invoice"),
                ("INT-0042", "uses", "billing/PaymentReceived"),
                ("SCN-0091", "verifies", "INT-0017"),
                ("SCN-0091", "uses", "billing/Customer"),
                ("SCN-0091", "uses", "billing/Invoice"),
                ("SCN-0091", "uses", "billing/InvoiceIssued"),
                ("SCN-0107", "verifies", "INT-0042"),
                ("SCN-0107", "uses", "billing/Invoice"),
                ("SCN-0107", "uses", "billing/PaymentReceived"),
                ("CON-0003", "belongs-to", "billing"),
                ("src/billing/invoice.rs", "implements", "INT-0042"),
                (
                    "tests/billing.rs::scn_0107_full_payment_settles_the_invoice",
                    "proves",
                    "SCN-0107"
                ),
            ]
        );
    }

    #[test]
    fn snapshot_owns_project_state_and_coverage_values() {
        let snapshot = fixture_snapshot();

        assert_eq!(snapshot.dashboard.state, "coherent");
        assert!(snapshot.dashboard.drift.is_empty());
        assert!(snapshot.dashboard.open_changes.is_empty());
        assert_eq!(snapshot.coverage.notions, 4);
        assert_eq!(snapshot.coverage.constraints, 1);
        assert_eq!(snapshot.coverage.intents_total, 2);
        assert_eq!(snapshot.coverage.intents_active, 2);
        assert_eq!(snapshot.coverage.intents_implemented, 1);
        assert_eq!(snapshot.coverage.scenarios_total, 2);
        assert_eq!(snapshot.coverage.scenarios_proved, 1);
    }

    #[test]
    fn coverage_rows_include_every_scenario_and_each_literal_proof() {
        let snapshot = fixture_snapshot();

        assert_eq!(
            snapshot
                .coverage
                .rows
                .iter()
                .map(|row| {
                    (
                        row.intent.as_str(),
                        row.scenario.as_str(),
                        row.test.as_deref(),
                    )
                })
                .collect::<Vec<_>>(),
            [
                ("INT-0017", "SCN-0091", None),
                (
                    "INT-0042",
                    "SCN-0107",
                    Some("tests/billing.rs::scn_0107_full_payment_settles_the_invoice"),
                ),
            ]
        );
    }

    #[test]
    fn graph_keys_keep_colliding_display_strings_distinct() {
        let mut model = fixture_model();
        let path = RepoPath::new("INT-0042");
        model.bindings.push(Binding::Implements {
            path: path.clone(),
            intent: Sp {
                node: IntentId(17),
                span: Span::default(),
            },
        });
        model.graph.add_edge(
            NodeRef::Code(path),
            Relation::Implements,
            NodeRef::Intent(IntentId(17)),
        );
        let snapshot = ViewSnapshot::build(
            &StateReport {
                state: ProjectStateKind::Coherent,
                drift: vec![],
                open_changes: vec![],
            },
            &model,
        );

        let intent = GraphKey::Intent("INT-0042".to_string());
        let code = GraphKey::Code("INT-0042".to_string());
        assert!(snapshot.nodes.iter().any(|node| node.key == intent));
        assert!(snapshot.nodes.iter().any(|node| node.key == code));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.from == code
                && edge.relation == "implements"
                && edge.to == GraphKey::Intent("INT-0017".to_string())
        }));
    }

    #[test]
    fn validated_model_projects_strategic_and_tactical_relations() {
        let model = super::all_relations_fixture_model();
        let snapshot = ViewSnapshot::build(
            &StateReport {
                state: ProjectStateKind::Coherent,
                drift: vec![],
                open_changes: vec![],
            },
            &model,
        );

        let relations: BTreeSet<_> = snapshot
            .edges
            .iter()
            .map(|edge| edge.relation.as_str())
            .collect();
        assert_eq!(
            relations,
            BTreeSet::from([
                "belongs-to",
                "refines",
                "requires",
                "excludes",
                "verifies",
                "uses",
                "implements",
                "proves",
            ])
        );
        assert!(snapshot.edges.iter().any(|edge| {
            edge.from == GraphKey::Capability("billing/invoicing".to_string())
                && edge.relation == "belongs-to"
                && edge.to == GraphKey::Context("billing".to_string())
        }));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.from == GraphKey::Intent("INT-0042".to_string())
                && edge.relation == "uses"
                && edge.to == GraphKey::Notion("billing/Invoice".to_string())
        }));
    }
}
