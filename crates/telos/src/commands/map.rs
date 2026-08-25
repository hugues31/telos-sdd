//! Read or transactionally replace the explicit bounded-context map.

use serde_json::json;

use telos_core::changes::{read_change, write_change};
use telos_core::emit::emit_context_map;
use telos_core::ids::RepoPath;
use telos_core::model::{ChangeStatus, StagedOp};
use telos_core::overlay::validate_ops_idempotent;
use telos_core::syntax::parse_context_map_file;
use telos_core::workspace::Workspace;

use crate::commands::change::parse_change_id;
use crate::commands::mutate::require_unclaimed;
use crate::commands::{Ctx, diagnostics_to_error, project, require_no_unclaimed_drift};
use crate::envelope::{CmdResult, Outcome};

const MAP_PATH: &str = "telos/context-map.tel";

pub fn run(ctx: &Ctx, change: Option<&str>, payload: Option<&str>) -> CmdResult {
    if let Some(change) = change {
        return stage(ctx, change, payload.unwrap_or_default());
    }
    let ws = Workspace::discover(&ctx.cwd)?;
    let model = ws.load_model().map_err(diagnostics_to_error)?;
    Ok(Outcome {
        result: json!(model.context_map),
        human: emit_context_map(&model.context_map)
            .trim_end_matches('\n')
            .to_string(),
        next_actions: Vec::new(),
    })
}

fn stage(ctx: &Ctx, change: &str, raw: &str) -> CmdResult {
    let id = parse_change_id(change)?;
    let project = project(ctx)?;
    require_no_unclaimed_drift(&project)?;
    let mut change = read_change(&project.ws, id)?;
    let path = RepoPath::new(MAP_PATH);
    let map = parse_context_map_file(&path, raw).map_err(diagnostics_to_error)?;
    let op = StagedOp::EditContextMap(map.clone());
    require_unclaimed(&project, id, &op.target_path())?;

    if let Some(existing) = change
        .ops
        .iter_mut()
        .find(|candidate| matches!(candidate, StagedOp::EditContextMap(_)))
    {
        *existing = op;
    } else {
        change.ops.push(op);
    }
    if change.status == ChangeStatus::Open {
        change.status = ChangeStatus::Drafted;
    }
    validate_ops_idempotent(&project.ws, &change.ops).map_err(diagnostics_to_error)?;
    write_change(&project.ws, &change)?;

    Ok(Outcome {
        result: json!({
            "change": id,
            "path": MAP_PATH,
            "claims": change.claims(),
            "map": map,
        }),
        human: format!("{id}: edit context-map {MAP_PATH}"),
        next_actions: vec![format!("telos change diff {id}")],
    })
}
