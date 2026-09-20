//! Capture observed specification drift into a prepared recovery-plan change.
//! Adoption preserves bytes and stages their semantic delta for plan review;
//! only approved recovery execution may later reconcile that delta.

use serde_json::json;

use telos_core::adopt::plan_adopt;
use telos_core::changes::{read_change, scan_changes, write_change};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::model::ChangeStatus;
use telos_core::overlay::validate_ops_idempotent;
use telos_core::state::{compute_state, drift_token};

use crate::commands::change::parse_change_id;
use crate::commands::mutate::require_unclaimed;
use crate::commands::{Ctx, diagnostics_to_error, project, require_drift};
use crate::envelope::{CmdResult, Outcome};

/// `telos adopt --into CHG-<uuid>` captures drift into its recovery task.
pub fn run(ctx: &Ctx, into: Option<&str>, expected_state: Option<&str>) -> CmdResult {
    // A malformed id is the caller's mistake and saying so needs no
    // workspace -- the same order `change abandon` and the staging verbs
    // use.
    let into = into.map(parse_change_id).transpose()?;

    let project = project(ctx)?;
    require_drift(&project, "adopt")?;
    let authorized_state = require_expected_state(&project, expected_state)?;
    let id = into.ok_or_else(|| {
        TelosError::new(
            ErrorCode::TelosPlanRequired,
            "adoption requires --into a prepared recovery-plan change",
        )
    })?;
    let owned = read_change(&project.ws, id)?;
    let (owner, task) =
        telos_core::plans::actions::require_change_contract(&project.ws.repo_root, &owned, false)?;
    if task.definition.kind != telos_core::plans::model::TaskKind::Recovery {
        return Err(TelosError::new(
            ErrorCode::TelosPlanScopeViolation,
            "adoption requires a recovery task",
        ));
    }
    telos_core::plans::ledger::require_scope(
        &project.ws.repo_root,
        &owner,
        &task.definition,
        &telos_core::inventory::capture(&project.ws.repo_root)?,
    )?;

    // Before the allocator, deliberately: [`allocator`] loads the model, and
    // a spec that does not parse is exactly what an unparseable drifted file
    // makes it. The caller must hear about the file, not about the model it
    // broke.
    let plan = plan_adopt(
        &project.ws,
        &project.git,
        &project.lock,
        &project.state.drift,
    )?;

    let mut change = owned;

    // Defensively exclude claimed paths: they are never unclaimed drift, so
    // `plan_adopt` cannot have produced one -- unless a change file was
    // written between `compute_state` and here. The gate costs nothing and
    // the alternative is two changes owning one file.
    for op in &plan.ops {
        if let Some(source) = op.source_path() {
            require_unclaimed(&project, change.id, &source)?;
        }
        require_unclaimed(&project, change.id, &op.target_path())?;
    }

    let adopted = plan.ops.len();
    change.ops.extend(plan.ops);
    if change.status == ChangeStatus::Open {
        change.status = ChangeStatus::Drafted;
    }

    validate_ops_idempotent(&project.ws, &change.ops).map_err(diagnostics_to_error)?;

    require_unchanged_state(&project, &authorized_state)?;

    write_change(&project.ws, &change)?;

    let id = change.id;
    Ok(Outcome {
        result: json!({ "change": id, "ops": adopted, "paths": plan.paths }),
        human: format!("{id}: adopted {adopted} drifted path(s)"),
        next_actions: vec![
            format!("telos change diff {id}"),
            format!("telos plan task import {} {}", owner.id, task.definition.id),
            format!("telos plan diff {}", owner.id),
        ],
    })
}

fn require_expected_state(
    project: &crate::commands::Project,
    expected: Option<&str>,
) -> Result<String, TelosError> {
    let current = drift_token(
        &project.ws,
        &project.git,
        &project.lock,
        &project.state.drift,
    )?;
    let authorized = expected.unwrap_or(&current);
    if authorized != current {
        return Err(stale_state());
    }
    Ok(authorized.to_string())
}

fn require_unchanged_state(
    project: &crate::commands::Project,
    expected: &str,
) -> Result<(), TelosError> {
    let changes = scan_changes(&project.ws)?;
    let current = compute_state(&project.ws, &project.lock, &project.git, &changes.infos)?;
    if drift_token(&project.ws, &project.git, &project.lock, &current.drift)? != expected {
        return Err(stale_state());
    }
    Ok(())
}

fn stale_state() -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        "project drift no longer matches the expected state token",
    )
    .hint("run `telos status` again and review the new drift scope")
}
