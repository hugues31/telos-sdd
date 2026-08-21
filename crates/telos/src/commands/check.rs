//! `telos check [--sealed]`: parses the spec and checks its integrity,
//! optionally also requiring the project to be sealed and
//! unmodified.
//!
//! `--sealed` checks state *before* parsing -- state comes from
//! `compute_state`, which never parses a spec file, so a spec that is both
//! drifted and syntactically broken reports `TELOS_DRIFT_DETECTED`, not a
//! parse error: drift is the more actionable diagnosis, and the more
//! likely actual cause (an in-progress edit, a bad merge). Without
//! `--sealed`, `check` never touches `telos.lock` at all.
//!
//! "Sealed and unmodified" is also false while a change is open, which is
//! why `--sealed` refuses `changing` too -- with `TELOS_CHANGE_STATE_INVALID`
//! rather than the drift code: nothing is damaged there, there is
//! simply work in flight, and the remedy is to finish or drop it.

use serde_json::{Value, json};

use telos_core::state::coverage;
use telos_core::workspace::Workspace;

use crate::commands::{
    Ctx, diagnostics_to_error, project, require_no_open_changes, require_no_unclaimed_drift,
};
use crate::envelope::{CmdResult, Outcome};

pub fn run(ctx: &Ctx, sealed: bool) -> CmdResult {
    let ws = if sealed {
        let project = project(ctx)?;
        // Both non-coherent states are refused, each under its own code and
        // in state priority order: unclaimed drift first (damage), then an
        // open change (work in progress). Two remedies, two diagnoses.
        require_no_unclaimed_drift(&project)?;
        require_no_open_changes(&project)?;
        project.ws
    } else {
        Workspace::discover(&ctx.cwd)?
    };

    match ws.load_model() {
        Ok(model) => {
            let cov = coverage(&model);
            Ok(Outcome {
                result: json!({ "diagnostics": Vec::<Value>::new() }),
                human: format!(
                    "check passed: {} notions, {} constraints, {} intents, {} scenarios",
                    cov.notions, cov.constraints, cov.intents_total, cov.scenarios_total
                ),
                next_actions: Vec::new(),
            })
        }
        Err(diagnostics) => Err(diagnostics_to_error(diagnostics)),
    }
}
