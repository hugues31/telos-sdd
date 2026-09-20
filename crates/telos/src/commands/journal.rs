//! Durable execution attempts for the existing proof and publication commands.

use serde_json::json;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::plans::{model::EventKind, store};

use crate::envelope::CmdResult;

pub enum Owner<'a> {
    Active,
    Change(&'a str),
    Task(&'a str, &'a str),
}

pub fn run(
    root: &std::path::Path,
    owner: Owner<'_>,
    operation: &str,
    request: &str,
    expected: Option<u64>,
    execute: impl FnOnce() -> CmdResult,
) -> CmdResult {
    let input = json!({"command":operation});
    for plan in store::list(root)? {
        if let Some(value) = plan.replay(&format!("{request}:result"), "cli.result", &input)? {
            return serde_json::from_value(value["result"]["outcome"].clone())
                .map_err(|e| TelosError::new(ErrorCode::TelosHistoryConflict, e.to_string()))?;
        }
        if plan.replay(request, "cli.start", &input)?.is_some() {
            return Err(TelosError::new(
                ErrorCode::TelosChangeStateInvalid,
                "this execution attempt has no durable result; inspect plan resume before issuing a new request",
            ));
        }
    }
    let executing = matches!(owner, Owner::Active);
    let owner = match owner {
        Owner::Active => store::active(root)?,
        Owner::Change(id) => store::for_change(root, id).ok(),
        Owner::Task(id, task) => store::read(root, id)
            .ok()
            .and_then(|p| p.task(task).ok().map(|t| p.task_view(t)).map(|t| (p, t))),
    };
    let Some((plan, task)) = owner else {
        return execute();
    };
    let attempt = json!({"attempt":request,"command":operation,"snapshot":telos_core::work::digest(&telos_core::inventory::capture(root)?)?});
    store::update(
        root,
        &plan.id,
        request,
        expected,
        "cli.start",
        &input,
        |plan| {
            store::require_editable(plan)?;
            if executing {
                store::require_approved(plan)?;
                telos_core::plans::ledger::require_scope(
                    root,
                    plan,
                    &task.definition,
                    &telos_core::inventory::capture(root)?,
                )?;
            }
            Ok(store::Update::event(
                EventKind::EvidenceStarted,
                Some(task.definition.id.clone()),
                attempt,
            ))
        },
    )?;
    let outcome = execute();
    if telos_core::transaction::require_recovered(root).is_err() {
        return outcome;
    }
    store::update(
        root,
        &plan.id,
        &format!("{request}:result"),
        None,
        "cli.result",
        &input,
        |_| {
            Ok(store::Update::event(
                EventKind::EvidenceRecorded,
                Some(task.definition.id),
                json!({"attempt":request,"command":operation,"outcome":outcome}),
            ))
        },
    )?;
    outcome
}
