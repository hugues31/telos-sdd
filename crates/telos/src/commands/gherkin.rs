//! `telos gherkin`: the `.feature` projection of the spec, printed.
//!
//! Writes nothing and seals nothing. The files this renders become sealed
//! spec in a later phase; here the command exists so a human can read the
//! prose a change would produce *before* approving it, rather than
//! discovering it after reconcile.

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
        // `parse_change_id` rather than a local parse: a mistyped `--change`
        // is the same class of mistake as one that does not exist, and every
        // other command carrying this flag reports both under one code.
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
/// base, its journal folded into bindings, then the semantic pass -- the same
/// construction `reconcile` builds, so the prose a human previews here can
/// never disagree with the prose reconciling that change would seal.
fn post_model(ws: &Workspace, id: ChangeId) -> Result<TelosModel, TelosError> {
    let change = read_change(ws, id)?;
    let base = parse_base(ws).map_err(diagnostics_to_error)?;
    let folded = fold_journal_bindings(apply_ops_idempotent(base, &change.ops), &change);
    build_model(folded).map_err(diagnostics_to_error)
}
