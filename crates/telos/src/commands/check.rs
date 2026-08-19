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

use telos_core::error::{Diagnostic, ErrorCode, TelosError};
use telos_core::git::GitRepo;
use telos_core::state::{ProjectStateKind, compute_state, coverage};
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, require_lock};
use crate::envelope::{CmdResult, Outcome};

/// The hint on `TELOS_DRIFT_DETECTED` from `check --sealed`. Frozen by
/// `docs/contracts.md` -- an M3 skill matches on this string.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

pub fn run(ctx: &Ctx, sealed: bool) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;

    if sealed {
        let lock = require_lock(&ws)?;
        let git = GitRepo::discover(&ctx.cwd)?;
        let report = compute_state(&ws, &lock, &git)?;
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

/// Collapses a full diagnostics list into the single [`TelosError`] the
/// envelope carries.
///
/// `code` and `hint` are the first diagnostic's -- the frozen error body
/// has room for exactly one of each, so `check`, which can find several
/// problems in one pass, surfaces the first (Annex B). The *message*
/// stays multi-line when there is more than one diagnostic: every
/// diagnostic gets its own `file: message` line (via the same
/// `From<Diagnostic>` conversion, applied to each), so a human reading
/// stderr sees everything `check` found in this run. In `--json` mode this
/// means `error.message` can itself carry more than one line -- an agent
/// that only reads the first line still gets the primary diagnosis; this
/// M1 limitation (no `result.diagnostics` array on failure) is documented
/// in `docs/contracts.md`.
fn diagnostics_to_error(diagnostics: Vec<Diagnostic>) -> TelosError {
    let mut iter = diagnostics.into_iter();
    let first = iter
        .next()
        .expect("`load_model` reports at least one diagnostic on `Err`");
    let mut error: TelosError = first.into();
    for diagnostic in iter {
        let extra: TelosError = diagnostic.into();
        error.message.push('\n');
        error.message.push_str(&extra.message);
    }
    error
}
