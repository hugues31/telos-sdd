//! `telos revert`: leave drift by restoring the bytes recorded in the seal.
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
//! - **It needs the sealed content in the object store.** Every seal writes
//!   the objects it names (`git hash-object -w`), so a project sealed but
//!   never committed still reverts. What can still be missing is a blob
//!   sealed by an older `telos`, or one git pruned as unreachable before a
//!   commit named it; this command then says so
//!   ([`telos_core::git::MISSING_BLOB_HINT`]) rather than silently
//!   restoring nothing.
//!
//! Like `adopt`, it acts on *unclaimed* drift only: a path an open change
//! claims is that change in progress, and throwing it away is
//! `change abandon`'s business, not this command's.

use serde_json::json;

use telos_core::changes::scan_changes;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::state::{compute_state, drift_token};

use crate::commands::{Ctx, project, require_drift};
use crate::envelope::{CmdResult, Outcome};

pub fn run(ctx: &Ctx, expected_state: Option<&str>) -> CmdResult {
    let project = project(ctx)?;
    require_drift(&project, "revert")?;
    let (plan, task) =
        telos_core::plans::store::active(&project.ws.repo_root)?.ok_or_else(|| {
            TelosError::new(
                ErrorCode::TelosPlanRequired,
                "revert requires an approved recovery task",
            )
        })?;
    telos_core::plans::store::require_approved(&plan)?;
    if task.definition.kind != telos_core::plans::model::TaskKind::Recovery {
        return Err(TelosError::new(
            ErrorCode::TelosPlanScopeViolation,
            "revert requires a recovery task",
        ));
    }
    telos_core::plans::ledger::require_scope(
        &project.ws.repo_root,
        &plan,
        &task.definition,
        &telos_core::inventory::capture(&project.ws.repo_root)?,
    )?;
    let current = drift_token(
        &project.ws,
        &project.git,
        &project.lock,
        &project.state.drift,
    )?;
    let authorized = expected_state.unwrap_or(&current).to_string();
    if authorized != current {
        return Err(stale_state());
    }

    let changes = scan_changes(&project.ws)?;
    let boundary = compute_state(&project.ws, &project.lock, &project.git, &changes.infos)?;
    if drift_token(&project.ws, &project.git, &project.lock, &boundary.drift)? != authorized {
        return Err(stale_state());
    }

    let ledger = telos_core::plans::ledger::verify(&project.ws.repo_root)?;
    let delta = telos_core::inventory::restore(&project.ws.repo_root, &ledger.current)?;
    let outcome = telos_core::adopt::RevertOutcome {
        restored: delta
            .iter()
            .filter(|f| f.after.is_some())
            .map(|f| telos_core::ids::RepoPath::new(&f.path))
            .collect(),
        deleted: delta
            .iter()
            .filter(|f| f.after.is_none())
            .map(|f| telos_core::ids::RepoPath::new(&f.path))
            .collect(),
    };

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
