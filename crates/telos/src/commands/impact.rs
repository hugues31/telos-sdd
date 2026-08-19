//! `telos impact <id|Name>`: everything a change to one entity would ripple
//! into -- the relation graph's reverse closure, walked backwards one hop of
//! relation at a time.
//!
//! Argument resolution is identical to `show`'s (same parse, same
//! not-found suggestions), so it goes through
//! [`crate::commands::resolve_or_hint`] rather than reimplementing either.

use serde_json::json;

use telos_core::graph::{ImpactEntry, NodeRef};
use telos_core::ids::EntityRef;
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, diagnostics_to_error, resolve_or_hint, unparsable};
use crate::envelope::{CmdResult, Outcome};

pub fn run(ctx: &Ctx, target: &str) -> CmdResult {
    let entity_ref: EntityRef = target.parse().map_err(|_| unparsable(target))?;

    let ws = Workspace::discover(&ctx.cwd)?;
    let model = ws.load_model().map_err(diagnostics_to_error)?;

    let node = resolve_or_hint(&model, &entity_ref)?;
    let impacted = model.graph.reverse_closure(&node);

    Ok(Outcome {
        result: json!({ "id": node, "impacted": impacted_json(&impacted) }),
        human: human(&node, &impacted),
        next_actions: Vec::new(),
    })
}

/// `[{"id": ..., "via": ..., "distance": ...}, ...]`, already sorted
/// `(distance, id)` by [`telos_core::graph::Graph::reverse_closure`].
fn impacted_json(impacted: &[ImpactEntry]) -> serde_json::Value {
    impacted
        .iter()
        .map(|entry| json!({ "id": entry.node, "via": entry.via, "distance": entry.distance }))
        .collect()
}

/// A header naming the queried entity, then one `  <id>  (via <rel>,
/// distance <n>)` line per impacted entry.
fn human(node: &NodeRef, impacted: &[ImpactEntry]) -> String {
    let mut lines = vec![format!("impact of {node}:")];
    for entry in impacted {
        lines.push(format!(
            "  {}  (via {}, distance {})",
            entry.node, entry.via, entry.distance
        ));
    }
    lines.join("\n")
}
