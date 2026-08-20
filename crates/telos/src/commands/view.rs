//! `telos view --export <DIR>` publishes a static, sealed documentation
//! snapshot.  The live server belongs to the next task; without `--export`
//! this command deliberately remains unavailable at runtime for now.

use std::path::Path;

use serde_json::json;

use telos_core::state::ProjectStateKind;

use crate::commands::{
    Ctx, diagnostics_to_error, project, require_no_open_changes, require_no_unclaimed_drift,
};
use crate::envelope::{CmdResult, Outcome};
use crate::view::export::export;
use crate::view::model::ViewSnapshot;

pub fn run(ctx: &Ctx, destination: &Path) -> CmdResult {
    let project = project(ctx)?;
    // Match `check --sealed` gate order exactly: damage wins over work in
    // progress, and neither state is permitted to publish as a sealed view.
    require_no_unclaimed_drift(&project)?;
    require_no_open_changes(&project)?;

    debug_assert_eq!(project.state.state, ProjectStateKind::Coherent);
    let model = project.ws.load_model().map_err(diagnostics_to_error)?;
    let snapshot = ViewSnapshot::build(&project.state, &model);
    let files = export(&snapshot, destination)?;
    let files: Vec<String> = files
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();

    Ok(Outcome {
        result: json!({
            "mode": "export",
            "destination": destination,
            "files": files,
        }),
        human: format!(
            "exported {} files to {}",
            files.len(),
            destination.display()
        ),
        next_actions: Vec::new(),
    })
}
