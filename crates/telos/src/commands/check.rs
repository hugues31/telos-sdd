//! `telos check [--sealed]`: parses the spec and checks its integrity
//! (spec §3.3), optionally also requiring the project to be sealed and
//! unmodified.
//!
//! `--sealed` checks state *before* parsing -- state comes from
//! [`compute_state`], which never parses anything, so a spec that is both
//! drifted and syntactically broken reports `TELOS_DRIFT_DETECTED`, not a
//! parse error: drift is the more actionable diagnosis, and the more
//! likely actual cause (an in-progress edit, a bad merge). Without
//! `--sealed`, `check` never touches `telos.lock` at all.

use serde_json::{Value, json};

use telos_core::error::{ErrorCode, TelosError};
use telos_core::git::GitRepo;
use telos_core::state::{ProjectStateKind, compute_state, coverage};
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, diagnostics_to_error, require_lock};
use crate::envelope::{CmdResult, Outcome};

/// The hint on `TELOS_DRIFT_DETECTED` from `check --sealed`. Frozen by
/// `docs/contracts.md` -- an M3 skill matches on this string.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

pub fn run(ctx: &Ctx, sealed: bool) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;

    if sealed {
        let lock = require_lock(&ws)?;
        let git = GitRepo::discover(&ctx.cwd)?;
        // `&[]`: `open_change_infos(ws)` is wired in T5.
        let report = compute_state(&ws, &lock, &git, &[])?;
        if report.state != ProjectStateKind::Coherent {
            return Err(TelosError::new(
                ErrorCode::TelosDriftDetected,
                "the project has drifted from its seal",
            )
            .hint(DRIFT_HINT));
        }
    }

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
