//! `telos gherkin`: the `.feature` projection of the spec, printed.
//!
//! The command writes nothing: it prints. Whether the files themselves are
//! written and sealed is governed by `[gherkin] enabled`.
//!
//! `--change` renders what a change would produce, so its prose can be read
//! before the change is approved.

use serde_json::{Value, json};

use telos_core::changes::read_change;
use telos_core::error::TelosError;
use telos_core::gherkin::render_features;
use telos_core::ids::ChangeId;
use telos_core::model::TelosModel;
use telos_core::overlay::{apply_ops_idempotent, fold_journal_bindings, parse_base};
use telos_core::semantic::build_model;
use telos_core::workspace::Workspace;

use crate::commands::change::parse_change_id;
use crate::commands::{Ctx, diagnostics_to_error};
use crate::envelope::{CmdResult, Outcome};

pub fn run(ctx: &Ctx, change: Option<&str>) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let model = match change {
        None => ws.load_model().map_err(diagnostics_to_error)?,
        Some(id) => post_model(&ws, parse_change_id(id)?)?,
    };
    let features = render_features(&model);

    let result = json!({
        "features": features
            .iter()
            .map(|(path, content)| json!({"path": path.as_str(), "content": content}))
            .collect::<Vec<Value>>(),
    });

    let human = if features.is_empty() {
        "no intents, so no features".to_string()
    } else {
        features
            .iter()
            .map(|(path, content)| format!("# {path}\n{content}"))
            .collect::<Vec<String>>()
            .join("\n")
    };

    Ok(Outcome {
        result,
        human,
        next_actions: Vec::new(),
    })
}

/// The change's post model: its ops replayed idempotently over the sealed
/// base, its journal folded into bindings, then the semantic pass. The same
/// construction `reconcile` builds, so a preview cannot disagree with what
/// reconciling would seal.
fn post_model(ws: &Workspace, id: ChangeId) -> Result<TelosModel, TelosError> {
    let change = read_change(ws, id)?;
    let base = parse_base(ws).map_err(diagnostics_to_error)?;
    let folded = fold_journal_bindings(apply_ops_idempotent(base, &change.ops), &change);
    build_model(folded).map_err(diagnostics_to_error)
}
