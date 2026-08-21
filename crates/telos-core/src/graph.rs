//! The relation graph: every entity of the spec is a node, and every relation
//! between two of them is a directed, labelled edge.
//!
//! Seven of the eight relations are *declared* (`refines`, `requires`,
//! `excludes` between intents; `constrains` from a scoped constraint;
//! `verifies` from a scenario to the intent nesting it; `implements` and
//! `proves` from the bindings file). The eighth, `uses`, is *derived* by the
//! semantic pass's walker over statements, steps and expressions -- it is
//! never written by hand.
//!
//! Determinism is a hard requirement: both adjacency maps are `BTreeMap`s and
//! each adjacency list is kept sorted by `(relation, node)` with duplicates
//! collapsed, so iteration order -- and therefore the order of every
//! diagnostic and of every `impact` report built on top -- depends only on
//! the content of the graph, never on insertion order.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;

use crate::ids::{ConstraintId, IntentId, NotionName, RepoPath, ScenarioId};

/// A node of the relation graph.
///
/// The declaration order of the variants is load-bearing: `NodeRef` derives
/// `Ord`, and that ordering is what ranks the entries of
/// [`Graph::reverse_closure`] (and hence the `impact` command's output) at
/// equal distance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(untagged)]
pub enum NodeRef {
    Notion(NotionName),
    Intent(IntentId),
    Scenario(ScenarioId),
    Constraint(ConstraintId),
    /// A source file bound to an intent by `implements`.
    Code(RepoPath),
    /// A test locator bound to a scenario by `proves`: the full
    /// `path::name` string (or a bare `path`).
    Test(String),
}

impl fmt::Display for NodeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeRef::Notion(n) => n.fmt(f),
            NodeRef::Intent(i) => i.fmt(f),
            NodeRef::Scenario(s) => s.fmt(f),
            NodeRef::Constraint(c) => c.fmt(f),
            NodeRef::Code(p) => p.fmt(f),
            NodeRef::Test(t) => f.write_str(t),
        }
    }
}

/// The label of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
    Refines,
    Requires,
    Excludes,
    Constrains,
    Verifies,
    /// Derived by the semantic pass, never declared.
    Uses,
    Implements,
    Proves,
}

impl Relation {
    /// The relation's spelling in `.tel` sources and in diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Relation::Refines => "refines",
            Relation::Requires => "requires",
            Relation::Excludes => "excludes",
            Relation::Constrains => "constrains",
            Relation::Verifies => "verifies",
            Relation::Uses => "uses",
            Relation::Implements => "implements",
            Relation::Proves => "proves",
        }
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One entry of a [`Graph::reverse_closure`] result: an entity impacted by a
/// change to the queried node, the relation that first reached it, and how
/// many edges away it sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactEntry {
    pub node: NodeRef,
    pub via: Relation,
    pub distance: u32,
}

/// The relation graph: two adjacency maps, forward and reverse.
#[derive(Debug, Default)]
pub struct Graph {
    out: BTreeMap<NodeRef, Vec<(Relation, NodeRef)>>,
    inc: BTreeMap<NodeRef, Vec<(Relation, NodeRef)>>,
}

/// Inserts `edge` in sorted position, doing nothing if it is already there.
fn insert_sorted(edges: &mut Vec<(Relation, NodeRef)>, edge: (Relation, NodeRef)) {
    if let Err(position) = edges.binary_search(&edge) {
        edges.insert(position, edge);
    }
}

impl Graph {
    /// Records `from -rel-> to` in both directions. Adding the same edge
    /// twice is a no-op: a notion used twice by one intent is one edge.
    pub fn add_edge(&mut self, from: NodeRef, rel: Relation, to: NodeRef) {
        insert_sorted(self.out.entry(from.clone()).or_default(), (rel, to.clone()));
        insert_sorted(self.inc.entry(to).or_default(), (rel, from));
    }

    /// The edges leaving `n`, sorted by `(relation, node)`. Empty for a node
    /// the graph does not know.
    pub fn out_edges(&self, n: &NodeRef) -> &[(Relation, NodeRef)] {
        self.out.get(n).map_or(&[], Vec::as_slice)
    }

    /// The edges entering `n`, each paired with the node it comes *from*,
    /// sorted by `(relation, node)`. Empty for a node the graph does not
    /// know.
    pub fn in_edges(&self, n: &NodeRef) -> &[(Relation, NodeRef)] {
        self.inc.get(n).map_or(&[], Vec::as_slice)
    }

    /// Looks for a cycle over the single relation `rel`, ignoring every other
    /// edge.
    ///
    /// Returns the cycle's path with the entry node repeated at the end, so
    /// that `INT-0017 -> INT-0042 -> INT-0017` prints as written. The search
    /// walks nodes in sorted order and stops at the first cycle found.
    pub fn find_cycle(&self, rel: Relation) -> Option<Vec<NodeRef>> {
        let mut finished: BTreeSet<&NodeRef> = BTreeSet::new();
        let mut path: Vec<&NodeRef> = Vec::new();
        let mut on_path: BTreeSet<&NodeRef> = BTreeSet::new();
        for start in self.out.keys() {
            if finished.contains(start) {
                continue;
            }
            if let Some(cycle) = self.walk(start, rel, &mut finished, &mut path, &mut on_path) {
                return Some(cycle);
            }
        }
        None
    }

    /// One depth-first descent of [`Graph::find_cycle`]: grey nodes are the
    /// ones on `path`, black ones are in `finished`.
    fn walk<'a>(
        &'a self,
        node: &'a NodeRef,
        rel: Relation,
        finished: &mut BTreeSet<&'a NodeRef>,
        path: &mut Vec<&'a NodeRef>,
        on_path: &mut BTreeSet<&'a NodeRef>,
    ) -> Option<Vec<NodeRef>> {
        path.push(node);
        on_path.insert(node);
        for (edge_rel, to) in self.out_edges(node) {
            if *edge_rel != rel {
                continue;
            }
            if on_path.contains(to) {
                let entry = path.iter().position(|n| *n == to).expect("grey node");
                let mut cycle: Vec<NodeRef> = path[entry..].iter().map(|n| (*n).clone()).collect();
                cycle.push(to.clone());
                return Some(cycle);
            }
            if finished.contains(to) {
                continue;
            }
            // `to` is a key of `out` whenever it has outgoing edges; when it
            // has none it cannot start a cycle, but it still must be walked
            // through the borrowed key to keep lifetimes uniform.
            let to = self.out.get_key_value(to).map_or(to, |(key, _)| key);
            if let Some(cycle) = self.walk(to, rel, finished, path, on_path) {
                return Some(cycle);
            }
        }
        on_path.remove(node);
        path.pop();
        finished.insert(node);
        None
    }

    /// Every entity reachable *backwards* from `from` over any relation --
    /// what a change to `from` can impact.
    ///
    /// Breadth-first over `in_edges`, so `distance` is the shortest-path
    /// length in edges and `via` is the relation of the edge that reached the
    /// node first. Each node appears once; the start node never appears, even
    /// when it sits on a cycle. The result is sorted by `(distance, node)`.
    pub fn reverse_closure(&self, from: &NodeRef) -> Vec<ImpactEntry> {
        let mut seen: BTreeSet<NodeRef> = BTreeSet::new();
        seen.insert(from.clone());

        let mut entries: Vec<ImpactEntry> = Vec::new();
        let mut frontier: Vec<NodeRef> = vec![from.clone()];
        let mut distance: u32 = 0;

        while !frontier.is_empty() {
            distance += 1;
            let mut next: Vec<NodeRef> = Vec::new();
            // The frontier is sorted and each adjacency list is sorted, so
            // which edge reaches a node "first" -- and therefore its `via` --
            // is a function of the graph alone.
            for node in &frontier {
                for (rel, source) in self.in_edges(node) {
                    if seen.insert(source.clone()) {
                        entries.push(ImpactEntry {
                            node: source.clone(),
                            via: *rel,
                            distance,
                        });
                        next.push(source.clone());
                    }
                }
            }
            next.sort();
            frontier = next;
        }

        entries.sort_by(|a, b| (a.distance, &a.node).cmp(&(b.distance, &b.node)));
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notion(name: &str) -> NodeRef {
        NodeRef::Notion(NotionName::new(name).unwrap())
    }

    fn intent(n: u32) -> NodeRef {
        NodeRef::Intent(IntentId(n))
    }

    fn scenario(n: u32) -> NodeRef {
        NodeRef::Scenario(ScenarioId(n))
    }

    // --- NodeRef / Relation --------------------------------------------

    #[test]
    fn node_ref_ordering_follows_the_canonical_variant_order() {
        // Load-bearing: `impact` ranks equal-distance entries with this
        // ordering, and its golden output depends on it.
        let mut nodes = vec![
            NodeRef::Test("tests/a.rs::x".to_string()),
            NodeRef::Code(RepoPath::new("src/a.rs")),
            NodeRef::Constraint(ConstraintId(3)),
            scenario(107),
            intent(42),
            notion("Invoice"),
        ];
        nodes.sort();
        assert_eq!(
            nodes,
            vec![
                notion("Invoice"),
                intent(42),
                scenario(107),
                NodeRef::Constraint(ConstraintId(3)),
                NodeRef::Code(RepoPath::new("src/a.rs")),
                NodeRef::Test("tests/a.rs::x".to_string()),
            ]
        );
    }

    #[test]
    fn node_ref_displays_the_bare_identity() {
        assert_eq!(notion("Invoice").to_string(), "Invoice");
        assert_eq!(intent(42).to_string(), "INT-0042");
        assert_eq!(scenario(107).to_string(), "SCN-0107");
        assert_eq!(NodeRef::Constraint(ConstraintId(3)).to_string(), "CON-0003");
        assert_eq!(
            NodeRef::Code(RepoPath::new("src/billing/invoice.rs")).to_string(),
            "src/billing/invoice.rs"
        );
        assert_eq!(
            NodeRef::Test("tests/billing.rs::scn_0107".to_string()).to_string(),
            "tests/billing.rs::scn_0107"
        );
    }

    #[test]
    fn node_ref_serializes_untagged() {
        assert_eq!(
            serde_json::to_string(&intent(42)).unwrap(),
            "\"INT-0042\"",
            "a node is its own identity in JSON, not a tagged variant"
        );
        assert_eq!(
            serde_json::to_string(&notion("Invoice")).unwrap(),
            "\"Invoice\""
        );
    }

    #[test]
    fn relation_serializes_and_displays_lowercase() {
        assert_eq!(
            serde_json::to_string(&Relation::Constrains).unwrap(),
            "\"constrains\""
        );
        assert_eq!(Relation::Uses.to_string(), "uses");
        assert_eq!(Relation::Refines.as_str(), "refines");
    }

    #[test]
    fn relation_ordering_follows_the_canonical_variant_order() {
        let mut relations = vec![
            Relation::Proves,
            Relation::Uses,
            Relation::Verifies,
            Relation::Refines,
        ];
        relations.sort();
        assert_eq!(
            relations,
            vec![
                Relation::Refines,
                Relation::Verifies,
                Relation::Uses,
                Relation::Proves
            ]
        );
    }

    // --- add_edge / out_edges / in_edges --------------------------------

    #[test]
    fn add_edge_records_both_directions() {
        let mut g = Graph::default();
        g.add_edge(intent(42), Relation::Requires, intent(17));
        assert_eq!(g.out_edges(&intent(42)), [(Relation::Requires, intent(17))]);
        assert_eq!(g.in_edges(&intent(17)), [(Relation::Requires, intent(42))]);
    }

    #[test]
    fn edges_of_an_unknown_node_are_empty() {
        let g = Graph::default();
        assert!(g.out_edges(&intent(1)).is_empty());
        assert!(g.in_edges(&intent(1)).is_empty());
    }

    #[test]
    fn the_same_edge_added_twice_is_stored_once() {
        // A notion used by both halves of a statement is one `uses` edge.
        let mut g = Graph::default();
        g.add_edge(intent(42), Relation::Uses, notion("Invoice"));
        g.add_edge(intent(42), Relation::Uses, notion("Invoice"));
        assert_eq!(g.out_edges(&intent(42)).len(), 1);
        assert_eq!(g.in_edges(&notion("Invoice")).len(), 1);
    }

    #[test]
    fn edges_are_kept_sorted_by_relation_then_node() {
        let mut g = Graph::default();
        g.add_edge(intent(42), Relation::Uses, notion("PaymentReceived"));
        g.add_edge(intent(42), Relation::Uses, notion("Invoice"));
        g.add_edge(intent(42), Relation::Requires, intent(17));
        assert_eq!(
            g.out_edges(&intent(42)),
            [
                (Relation::Requires, intent(17)),
                (Relation::Uses, notion("Invoice")),
                (Relation::Uses, notion("PaymentReceived")),
            ]
        );
    }

    // --- find_cycle ------------------------------------------------------

    #[test]
    fn find_cycle_returns_none_on_an_acyclic_graph() {
        let mut g = Graph::default();
        g.add_edge(intent(3), Relation::Requires, intent(2));
        g.add_edge(intent(2), Relation::Requires, intent(1));
        g.add_edge(intent(3), Relation::Requires, intent(1));
        assert_eq!(g.find_cycle(Relation::Requires), None);
    }

    #[test]
    fn find_cycle_detects_a_self_loop() {
        let mut g = Graph::default();
        g.add_edge(intent(1), Relation::Requires, intent(1));
        assert_eq!(
            g.find_cycle(Relation::Requires),
            Some(vec![intent(1), intent(1)])
        );
    }

    #[test]
    fn find_cycle_returns_the_path_with_the_entry_node_repeated() {
        let mut g = Graph::default();
        g.add_edge(intent(1), Relation::Requires, intent(2));
        g.add_edge(intent(2), Relation::Requires, intent(3));
        g.add_edge(intent(3), Relation::Requires, intent(1));
        assert_eq!(
            g.find_cycle(Relation::Requires),
            Some(vec![intent(1), intent(2), intent(3), intent(1)])
        );
    }

    #[test]
    fn find_cycle_reports_a_cycle_that_does_not_contain_the_first_node() {
        // INT-0001 leads into a 2-cycle without being part of it: the path
        // must start at the cycle's entry node, not at the DFS root.
        let mut g = Graph::default();
        g.add_edge(intent(1), Relation::Requires, intent(2));
        g.add_edge(intent(2), Relation::Requires, intent(3));
        g.add_edge(intent(3), Relation::Requires, intent(2));
        assert_eq!(
            g.find_cycle(Relation::Requires),
            Some(vec![intent(2), intent(3), intent(2)])
        );
    }

    #[test]
    fn find_cycle_considers_one_relation_at_a_time() {
        // A cycle needs edges of a single kind: mixing `requires` with
        // `refines` is not a cycle on either.
        let mut g = Graph::default();
        g.add_edge(intent(1), Relation::Requires, intent(2));
        g.add_edge(intent(2), Relation::Refines, intent(1));
        assert_eq!(g.find_cycle(Relation::Requires), None);
        assert_eq!(g.find_cycle(Relation::Refines), None);
    }

    #[test]
    fn find_cycle_does_not_re_walk_a_shared_subtree() {
        // A diamond: INT-0004 reaches INT-0001 twice. Revisiting a finished
        // node must not be mistaken for a cycle.
        let mut g = Graph::default();
        g.add_edge(intent(4), Relation::Refines, intent(2));
        g.add_edge(intent(4), Relation::Refines, intent(3));
        g.add_edge(intent(2), Relation::Refines, intent(1));
        g.add_edge(intent(3), Relation::Refines, intent(1));
        assert_eq!(g.find_cycle(Relation::Refines), None);
    }

    // --- reverse_closure -------------------------------------------------

    #[test]
    fn reverse_closure_of_an_isolated_node_is_empty() {
        let g = Graph::default();
        assert_eq!(g.reverse_closure(&notion("Invoice")), vec![]);
    }

    #[test]
    fn reverse_closure_ranks_by_distance_then_node() {
        let mut g = Graph::default();
        g.add_edge(intent(42), Relation::Uses, notion("Invoice"));
        g.add_edge(intent(17), Relation::Uses, notion("Invoice"));
        g.add_edge(scenario(107), Relation::Verifies, intent(42));
        assert_eq!(
            g.reverse_closure(&notion("Invoice")),
            vec![
                ImpactEntry {
                    node: intent(17),
                    via: Relation::Uses,
                    distance: 1
                },
                ImpactEntry {
                    node: intent(42),
                    via: Relation::Uses,
                    distance: 1
                },
                ImpactEntry {
                    node: scenario(107),
                    via: Relation::Verifies,
                    distance: 2
                },
            ]
        );
    }

    #[test]
    fn reverse_closure_keeps_the_shortest_distance_and_its_relation() {
        // SCN-0107 reaches Invoice directly (`uses`, 1) and through INT-0042
        // (`verifies` then `uses`, 2): the short path wins, once.
        let mut g = Graph::default();
        g.add_edge(intent(42), Relation::Uses, notion("Invoice"));
        g.add_edge(scenario(107), Relation::Uses, notion("Invoice"));
        g.add_edge(scenario(107), Relation::Verifies, intent(42));
        assert_eq!(
            g.reverse_closure(&notion("Invoice")),
            vec![
                ImpactEntry {
                    node: intent(42),
                    via: Relation::Uses,
                    distance: 1
                },
                ImpactEntry {
                    node: scenario(107),
                    via: Relation::Uses,
                    distance: 1
                },
            ]
        );
    }

    #[test]
    fn reverse_closure_excludes_the_start_node_even_on_a_cycle() {
        let mut g = Graph::default();
        g.add_edge(intent(1), Relation::Requires, intent(2));
        g.add_edge(intent(2), Relation::Requires, intent(1));
        assert_eq!(
            g.reverse_closure(&intent(1)),
            vec![ImpactEntry {
                node: intent(2),
                via: Relation::Requires,
                distance: 1
            }]
        );
    }

    #[test]
    fn reverse_closure_walks_backwards_only() {
        // INT-0042 uses Invoice; nothing points at INT-0042, so a change to
        // INT-0042 impacts nobody -- the forward edge must not be followed.
        let mut g = Graph::default();
        g.add_edge(intent(42), Relation::Uses, notion("Invoice"));
        assert_eq!(g.reverse_closure(&intent(42)), vec![]);
    }
}
