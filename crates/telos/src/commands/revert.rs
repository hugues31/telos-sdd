//! `telos revert`: the exit from drift that keeps the seal (spec §6, D7).
//!
//! The mirror image of [`crate::commands::adopt`]. Where `adopt` decides the
//! working tree is right and the seal has to catch up, `revert` decides the
//! seal is right and the working tree has to go back: every sealed path is
//! rewritten from the blob its OID names, and every spec file the seal never
//! held is deleted.
//!
//! Two consequences worth knowing before running it:
//!
//! - **It destroys the drifted bytes.** There is no undo beyond what git
//!   already holds. `telos status` names the paths first, and `telos adopt`
//!   is the other exit.
//! - **It needs the sealed content in the object store.** A seal records
//!   OIDs; it does not write objects. On a project sealed but never
//!   committed, the content those OIDs name does not exist anywhere, and
//!   this command says so ([`telos_core::git::MISSING_BLOB_HINT`]) rather
//!   than silently restoring nothing.
//!
//! Like `adopt`, it acts on *unclaimed* drift only: a path an open change
//! claims is that change in progress (D5), and throwing it away is
//! `change abandon`'s business, not this command's.

use serde_json::json;

use telos_core::adopt::revert;
use telos_core::changes::scan_changes;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::state::{compute_state, drift_token};

use crate::commands::{Ctx, project, require_drift};
use crate::envelope::{CmdResult, Outcome};

pub fn run(ctx: &Ctx, expected_state: Option<&str>) -> CmdResult {
    let project = project(ctx)?;
    require_drift(&project, "revert")?;
    let current = drift_token(&project.lock, &project.state.drift);
    let authorized = expected_state.unwrap_or(&current).to_string();
    if authorized != current {
        return Err(stale_state());
    }

    let changes = scan_changes(&project.ws)?;
    let boundary = compute_state(&project.ws, &project.lock, &project.git, &changes.infos)?;
    if drift_token(&project.lock, &boundary.drift) != authorized {
        return Err(stale_state());
    }

    let outcome = revert(
        &project.ws,
        &project.git,
        &project.lock,
        &project.state.drift,
    )?;

    let human = format!(
        "restored {} path(s), deleted {}",
        outcome.restored.len(),
        outcome.deleted.len()
    );
    Ok(Outcome {
        result: json!({ "restored": outcome.restored, "deleted": outcome.deleted }),
        human,
        next_actions: vec!["telos status".to_string()],
    })
}

fn stale_state() -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        "project drift no longer matches the expected state token",
    )
    .hint("run `telos status` again and review the new drift scope")
}
