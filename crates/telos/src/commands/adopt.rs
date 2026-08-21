//! `telos adopt [--into CHG-NNNN]`: the exit from drift that keeps the
//! bytes (spec §6, D7).
//!
//! Drift is refused everywhere else -- `change open`, the staging verbs,
//! `approve`, `reconcile` (D17) -- and this is one of the two commands that
//! makes those refusals cheap rather than punitive: an edit made outside the
//! protocol is not lost, it is *routed back in*. Every drifted path becomes a
//! staged op of a change, and from there the ordinary loop applies: `change
//! diff` to review it, `change approve` to freeze it, `change reconcile` to
//! seal it.
//!
//! The flow mirrors [`crate::commands::mutate`]'s, and for the same reason:
//! nothing reaches the disk until everything has been decided.
//!
//! 1. **State first.** Drift is what this command acts on, so its absence is
//!    the one refusal that comes before any work.
//! 2. **The plan is built** ([`plan_adopt`]) from the *unclaimed* drift only
//!    -- a path an open change already claims is that change in progress
//!    (D5), never adopted twice.
//! 3. **The target change is chosen**: a new one, or `--into`'s.
//! 4. **One file, one change** (D5): the claim gate, against every *other*
//!    open change.
//! 5. **The whole delta is validated** ([`validate_ops_idempotent`]) --
//!    including the ops the target change already held.
//! 6. **Only then does anything reach the disk**, change file first,
//!    `counters.toml` second.
//!
//! After a successful `adopt` the project is `changing`, not `coherent`: the
//! drift is claimed now, so it stops counting as drift, but nothing has been
//! resealed. That is the point -- `adopt` captures, `reconcile` seals.

use serde_json::json;

use telos_core::adopt::plan_adopt;
use telos_core::changes::{read_change, scan_changes, write_change};
use telos_core::counters::write_counters;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::model::{Change, ChangeStatus};
use telos_core::overlay::validate_ops_idempotent;
use telos_core::state::{compute_state, drift_token};

use crate::commands::change::parse_change_id;
use crate::commands::mutate::require_unclaimed;
use crate::commands::{Ctx, allocator, diagnostics_to_error, project, require_drift};
use crate::envelope::{CmdResult, Outcome};

/// The motivation a change opened by `adopt` carries. A change file must say
/// why it exists (D1), and «somebody edited the spec outside the protocol»
/// is the honest answer -- the review that follows is where a better one
/// gets written, if the caller wants one.
const MOTIVATION: &str = "adopted drift";

/// `telos adopt`, and `telos adopt --into CHG-NNNN`.
pub fn run(ctx: &Ctx, into: Option<&str>, expected_state: Option<&str>) -> CmdResult {
    // A malformed id is the caller's mistake and saying so needs no
    // workspace -- the same order `change abandon` and the staging verbs
    // use.
    let into = into.map(parse_change_id).transpose()?;

    let project = project(ctx)?;
    require_drift(&project, "adopt")?;
    let authorized_state = require_expected_state(&project, expected_state)?;

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

    let (mut change, alloc) = match into {
        Some(id) => (read_change(&project.ws, id)?, None),
        None => {
            let mut alloc = allocator(&project.ws, &project.lock)?;
            let id = alloc.next_change();
            (
                Change {
                    id,
                    motivation: MOTIVATION.to_string(),
                    status: ChangeStatus::Open,
                    approved_digest: None,
                    ops: Vec::new(),
                    journal: Vec::new(),
                },
                Some(alloc),
            )
        }
    };

    // D5, defensively: a claimed path is never unclaimed drift, so
    // `plan_adopt` cannot have produced one -- unless a change file was
    // written between `compute_state` and here. The gate costs nothing and
    // the alternative is two changes owning one file.
    for op in &plan.ops {
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
    // Only a *new* change spent an id; `--into` reuses one that was
    // allocated and persisted when it was opened (D4).
    if let Some(alloc) = &alloc {
        write_counters(&project.ws, &alloc.counters())?;
    }

    let id = change.id;
    Ok(Outcome {
        result: json!({ "change": id, "ops": adopted, "paths": plan.paths }),
        human: format!("{id}: adopted {adopted} drifted path(s)"),
        next_actions: vec![
            format!("telos change diff {id}"),
            format!("telos change approve {id}"),
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
