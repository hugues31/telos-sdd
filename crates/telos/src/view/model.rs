use std::collections::BTreeSet;

use serde::Serialize;
use telos_core::emit::{emit_constraint, emit_intent, emit_notion};
use telos_core::graph::{NodeRef, Relation};
use telos_core::ids::IntentId;
use telos_core::model::{Binding, ConstraintKind, IntentStatus, NotionKind, Scope, TelosModel};
use telos_core::state::{DriftKind, ProjectStateKind, StateReport, coverage as model_coverage};

use crate::projection::{applicable_constraints, implementations, proofs};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ViewSnapshot {
    pub(crate) dashboard: DashboardView,
    pub(crate) coverage: CoverageView,
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
    pub(crate) subject: String,
    pub(crate) covered: Option<u32>,
    pub(crate) total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NotionView {
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) definition: String,
    pub(crate) canonical: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct IntentView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) telos: String,
    pub(crate) canonical: String,
    pub(crate) notions: Vec<String>,
    pub(crate) constraints: Vec<ConstraintRefView>,
    pub(crate) implements: Vec<String>,
    pub(crate) scenarios: Vec<ScenarioView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ScenarioView {
    pub(crate) id: String,
    pub(crate) intent: String,
    pub(crate) title: String,
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
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) scope: String,
    pub(crate) canonical: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GraphNodeView {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GraphEdgeView {
    pub(crate) from: String,
    pub(crate) relation: String,
    pub(crate) to: String,
}

impl ViewSnapshot {
    pub(crate) fn build(state: &StateReport, model: &TelosModel) -> Self {
        let notions = model
            .notions
            .values()
            .map(|notion| NotionView {
                name: notion.name.to_string(),
                kind: notion_kind(notion.kind).to_string(),
                definition: notion.def.clone(),
                canonical: emit_notion(notion),
            })
            .collect();

        let mut scenarios = Vec::new();
        let intents = model
            .intents
            .values()
            .map(|intent| {
                let intent_proofs = proofs(model, intent);
                let scenario_views: Vec<ScenarioView> = intent
                    .scenarios
                    .iter()
                    .map(|scenario| ScenarioView {
                        id: scenario.id.to_string(),
                        intent: intent.id.to_string(),
                        title: scenario.title.clone(),
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

                IntentView {
                    id: intent.id.to_string(),
                    title: intent.title.clone(),
                    status: intent_status(intent.status).to_string(),
                    telos: intent.telos.clone(),
                    canonical: emit_intent(intent),
                    notions: notion_names,
                    constraints: applicable_constraints(model, intent.id)
                        .into_iter()
                        .map(|entry| ConstraintRefView {
                            id: entry.constraint.id.to_string(),
                            title: entry.constraint.title.clone(),
                            scope: entry.scope.to_string(),
                            canonical: emit_constraint(entry.constraint),
                        })
                        .collect(),
                    implements: implementations(model, intent.id)
                        .into_iter()
                        .map(|path| path.to_string())
                        .collect(),
                    scenarios: scenario_views,
                }
            })
            .collect();
        scenarios.sort_by(|a, b| a.id.cmp(&b.id));

        let constraints = model
            .constraints
            .values()
            .map(|constraint| ConstraintView {
                id: constraint.id.to_string(),
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
                canonical: emit_constraint(constraint),
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
                from: from.to_string(),
                relation: relation.as_str().to_string(),
                to: to.to_string(),
            })
            .collect();

        let raw_coverage = model_coverage(model);
        let coverage = CoverageView {
            notions: raw_coverage.notions,
            constraints: raw_coverage.constraints,
            intents_total: raw_coverage.intents_total,
            intents_active: raw_coverage.intents_active,
            intents_implemented: raw_coverage.intents_implemented,
            scenarios_total: raw_coverage.scenarios_total,
            scenarios_proved: raw_coverage.scenarios_proved,
            rows: vec![
                CoverageRowView {
                    subject: "notions".to_string(),
                    covered: None,
                    total: raw_coverage.notions,
                },
                CoverageRowView {
                    subject: "constraints".to_string(),
                    covered: None,
                    total: raw_coverage.constraints,
                },
                CoverageRowView {
                    subject: "intents implemented".to_string(),
                    covered: Some(raw_coverage.intents_implemented),
                    total: raw_coverage.intents_total,
                },
                CoverageRowView {
                    subject: "scenarios proved".to_string(),
                    covered: Some(raw_coverage.scenarios_proved),
                    total: raw_coverage.scenarios_total,
                },
            ],
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

    pub(crate) fn intent(&self, id: IntentId) -> Option<&IntentView> {
        let id = id.to_string();
        self.intents.iter().find(|intent| intent.id == id)
    }
}

fn model_nodes(model: &TelosModel) -> BTreeSet<NodeRef> {
    let mut nodes = BTreeSet::new();
    nodes.extend(model.notions.keys().cloned().map(NodeRef::Notion));
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
    let (kind, label) = match node {
        NodeRef::Notion(name) => (
            "notion",
            model
                .notions
                .get(name)
                .map(|notion| notion.def.as_str())
                .unwrap_or(name.as_str()),
        ),
        NodeRef::Intent(id) => (
            "intent",
            model
                .intents
                .get(id)
                .map(|intent| intent.title.as_str())
                .unwrap_or(""),
        ),
        NodeRef::Scenario(id) => (
            "scenario",
            model
                .scenario(*id)
                .map(|(_, scenario)| scenario.title.as_str())
                .unwrap_or(""),
        ),
        NodeRef::Constraint(id) => (
            "constraint",
            model
                .constraints
                .get(id)
                .map(|constraint| constraint.title.as_str())
                .unwrap_or(""),
        ),
        NodeRef::Code(path) => ("code", path.as_str()),
        NodeRef::Test(test) => ("test", test.as_str()),
    };
    GraphNodeView {
        id: node.to_string(),
        kind: kind.to_string(),
        label: label.to_string(),
    }
}

fn uses_from(model: &TelosModel, node: &NodeRef) -> Vec<String> {
    model
        .graph
        .out_edges(node)
        .iter()
        .filter_map(|(relation, target)| match (relation, target) {
            (Relation::Uses, NodeRef::Notion(name)) => Some(name.to_string()),
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
mod tests {
    use std::path::PathBuf;

    use telos_core::ids::IntentId;
    use telos_core::state::{ProjectStateKind, StateReport};
    use telos_core::workspace::Workspace;

    use super::ViewSnapshot;

    fn fixture_snapshot() -> ViewSnapshot {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing");
        let workspace = Workspace::discover(&fixture).expect("Billing workspace is discoverable");
        let model = workspace
            .load_model()
            .expect("Billing fixture is a valid model");
        let state = StateReport {
            state: ProjectStateKind::Coherent,
            drift: vec![],
            open_changes: vec![],
        };
        ViewSnapshot::build(&state, &model)
    }

    #[test]
    fn snapshot_contains_graph_relations_and_cross_references() {
        let snapshot = fixture_snapshot();

        assert!(snapshot.nodes.iter().any(|node| node.id == "INT-0042"));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.from == "INT-0042" && edge.relation == "requires" && edge.to == "INT-0017"
        }));

        let intent = snapshot.intent(IntentId(42)).unwrap();
        assert_eq!(intent.implements, ["src/billing/invoice.rs"]);
        assert_eq!(intent.scenarios[0].id, "SCN-0107");
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
            ["Customer", "Invoice", "InvoiceIssued", "PaymentReceived"]
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
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            [
                "Customer",
                "Invoice",
                "InvoiceIssued",
                "PaymentReceived",
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
                .map(|edge| { (edge.from.as_str(), edge.relation.as_str(), edge.to.as_str(),) })
                .collect::<Vec<_>>(),
            [
                ("INT-0017", "uses", "Invoice"),
                ("INT-0017", "uses", "InvoiceIssued"),
                ("INT-0042", "requires", "INT-0017"),
                ("INT-0042", "uses", "Invoice"),
                ("INT-0042", "uses", "PaymentReceived"),
                ("SCN-0091", "verifies", "INT-0017"),
                ("SCN-0091", "uses", "Customer"),
                ("SCN-0091", "uses", "Invoice"),
                ("SCN-0091", "uses", "InvoiceIssued"),
                ("SCN-0107", "verifies", "INT-0042"),
                ("SCN-0107", "uses", "Invoice"),
                ("SCN-0107", "uses", "PaymentReceived"),
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
}
