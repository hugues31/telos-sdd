//! Restartable task execution and evidence tied to exact repository bytes.

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use super::{actions, ledger, model::*, store};
use crate::error::{ErrorCode, TelosError};
use crate::ids::{ChangeId, RepoPath};
use crate::inventory;
use crate::work::{digest, new_id};

pub fn start(
    root: &Path,
    id: &str,
    task_id: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    store::update(
        root,
        id,
        request_id,
        expected,
        "plan.task.start",
        &json!({"task":task_id}),
        |plan| {
            store::require_approved(plan)?;
            let active = store::active(root)?;
            let rework = active.as_ref().is_some_and(|(owner, t)| {
                owner.id == id
                    && t.definition.id == task_id
                    && t.state == TaskState::InProgress
                    && ledger::receipts(root).is_ok_and(|receipts| {
                        receipts.iter().any(|r| Some(&r.id) == t.change.as_ref())
                    })
            });
            if active.is_some() && !rework {
                return Err(invalid("a task already owns this worktree"));
            }
            if !rework && !plan.ready_tasks()?.iter().any(|t| t == task_id) {
                return Err(TelosError::new(
                    ErrorCode::TelosPlanDependencyUnmet,
                    "the task is not ready; complete its dependencies first",
                ));
            }
            let task = plan.task(task_id)?;
            if !rework && !matches!(task.kind, TaskKind::Recovery | TaskKind::Integration) {
                ledger::require_clean(root)?;
            }
            let snapshot = inventory::capture(root)?;
            ledger::require_scope(root, plan, task, &snapshot)?;
            if !plan
                .events
                .iter()
                .any(|e| e.kind == EventKind::TaskStarted && e.revision == plan.revision().number)
                && digest(&snapshot)? != plan.revision().base_digest
            {
                return Err(TelosError::new(
                    ErrorCode::TelosApprovalStale,
                    "the repository changed since this revision was approved",
                ));
            }
            let existing = if rework {
                None
            } else {
                plan.task_view(task).change
            };
            let reserved = if rework {
                None
            } else {
                task.change_id.as_deref()
            };
            let change_id = existing
                .as_deref()
                .or(reserved)
                .map(str::parse)
                .transpose()?
                .unwrap_or(ChangeId::allocate()?);
            if existing.is_none()
                && (store::for_change(root, &change_id.to_string()).is_ok()
                    || root.join(format!("telos/changes/{change_id}.tel")).exists())
            {
                return Err(invalid("the requested change identity is already in use"));
            }
            let change = actions::prepared_change(task, change_id)?;
            let mut update = store::Update::event(
                EventKind::TaskStarted,
                Some(task_id.into()),
                json!({"change":change.id,"snapshot":snapshot,"baseline":digest(&ledger::read(root)?.current)?,"head":inventory::head(root)?,"next_action":task.next_action}),
            );
            update.writes.push((
                RepoPath::new(format!("telos/changes/{}.tel", change.id)),
                Some(crate::emit::emit_change(&change).into_bytes()),
            ));
            Ok(update)
        },
    )
}

pub fn finish(
    root: &Path,
    id: &str,
    task: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    store::update(
        root,
        id,
        request_id,
        expected,
        "plan.task.finish",
        &json!({"task":task}),
        |plan| {
            store::require_approved(plan)?;
            let definition = plan.task(task)?;
            let view = plan.task_view(definition);
            if view.state != TaskState::InProgress {
                return Err(invalid("only an in-progress task can finish"));
            }
            ledger::require_clean(root)?;
            let change = view
                .change
                .as_deref()
                .ok_or_else(|| invalid("the task has no change"))?;
            if !ledger::receipts(root)?
                .iter()
                .any(|r| r.id == change && r.plan == id && r.task == task)
            {
                return Err(invalid(
                    "reconcile the task change before finishing the task",
                ));
            }
            require_evidence(root, plan, Some(task))?;
            Ok(store::Update::event(
                EventKind::TaskCompleted,
                Some(task.into()),
                json!({"change":change,"snapshot":digest(&inventory::capture(root)?)?}),
            ))
        },
    )
}

pub fn complete(
    root: &Path,
    id: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    store::update(
        root,
        id,
        request_id,
        expected,
        "plan.complete",
        &json!({}),
        |plan| {
            store::require_approved(plan)?;
            if plan
                .view()?
                .tasks
                .iter()
                .any(|t| !matches!(t.state, TaskState::Done | TaskState::Cancelled))
            {
                return Err(invalid(
                    "finish every non-cancelled task before completing the plan",
                ));
            }
            ledger::require_clean(root)?;
            require_evidence(root, plan, None)?;
            Ok(store::Update::event(
                EventKind::Completed,
                None,
                json!({"snapshot":digest(&inventory::capture(root)?)?}),
            ))
        },
    )
}

fn validations<'a>(plan: &'a Plan, task: Option<&str>) -> Result<&'a [Validation], TelosError> {
    Ok(match task {
        Some(id) => &plan.task(id)?.validation,
        None => &plan.revision().definition.validation,
    })
}

pub fn pending_attempt(plan: &Plan) -> Option<&Event> {
    plan.events.iter().rev().find(|start| {
        start.kind == EventKind::ValidationStarted
            && !plan.events.iter().any(|end| {
                (end.kind == EventKind::ValidationRecorded
                    && end.data["attempt"] == start.data["attempt"])
                    || (end.kind == EventKind::ValidationStarted
                        && end.version > start.version
                        && end.data["retries_unknown"] == true)
            })
    })
}

/// Each invocation executes at most one named command. A durable started event
/// precedes the process; reusing its request ID never executes it twice.
// Mirrors the independently optional CLI evidence and retry inputs.
#[allow(clippy::too_many_arguments)]
pub fn verify(
    root: &Path,
    id: &str,
    task: Option<&str>,
    name: &str,
    review: Option<&str>,
    retry_unknown: bool,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    let input = json!({"task":task,"name":name,"review":review,"retry_unknown":retry_unknown});
    let before = store::read(root, id)?;
    if let Some(result) = before.replay(
        &format!("{request_id}:result"),
        "plan.verify.result",
        &input,
    )? {
        return Ok(result);
    }
    if before
        .replay(request_id, "plan.verify.start", &input)?
        .is_some()
    {
        return Err(invalid(
            "validation has no durable result; inspect the runner, then retry explicitly with a new request identity and --retry-unknown",
        ));
    }
    let validation = validations(&before, task)?
        .iter()
        .find(|v| validation_name(v) == name)
        .cloned()
        .ok_or_else(|| invalid(format!("unknown validation `{name}`")))?;
    let attempt = new_id("RUN")?;
    let snapshot = inventory::capture(root)?;
    let snapshot_digest = digest(&snapshot)?;
    store::update(
        root,
        id,
        request_id,
        Some(expected.unwrap_or(before.version())),
        "plan.verify.start",
        &input,
        |plan| {
            store::require_approved(plan)?;
            if let Some(task) = task {
                if plan.task_view(plan.task(task)?).state != TaskState::InProgress {
                    return Err(invalid("start the task before validating it"));
                }
                ledger::require_scope(root, plan, plan.task(task)?, &snapshot)?;
            } else {
                ledger::require_clean(root)?;
            }
            if pending_attempt(plan).is_some() && !retry_unknown {
                return Err(invalid(
                    "a validation has an unknown outcome; inspect it before using --retry-unknown",
                ));
            }
            Ok(store::Update::event(
                EventKind::ValidationStarted,
                task.map(str::to_owned),
                json!({"attempt":attempt,"name":name,"snapshot":snapshot_digest,"retries_unknown":retry_unknown}),
            ))
        },
    )?;
    let (passed, details) = match &validation {
        Validation::Command { argv, .. } => {
            match Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
            {
                Ok(status) => (
                    status.success(),
                    json!({"exit_code":status.code(),"argv":argv}),
                ),
                Err(error) => (false, json!({"error":error.to_string(),"argv":argv})),
            }
        }
        Validation::Review { .. } => (
            review.is_some_and(|s| !s.trim().is_empty()),
            json!({"note":review}),
        ),
        Validation::Scenario { id } => {
            let ws = crate::workspace::Workspace::discover(root)?;
            let model = ws
                .load_model()
                .map_err(|e| TelosError::from(e.into_iter().next().expect("diagnostic")))?;
            let scenario = id.parse()?;
            let proved = model.bindings.iter().any(
                |b| matches!(b,crate::model::Binding::Proves {scenario:s,..} if s.node == scenario),
            );
            (
                proved && ledger::require_clean(root).is_ok(),
                json!({"scenario":id,"source":"reconciled_proof"}),
            )
        }
    };
    let after = inventory::capture(root)?;
    let unchanged = snapshot == after;
    store::update(
        root,
        id,
        &format!("{request_id}:result"),
        None,
        "plan.verify.result",
        &input,
        |plan| {
            let approved =
                plan.view()?.approved && plan.revision().number == before.revision().number;
            Ok(store::Update::event(
                EventKind::ValidationRecorded,
                task.map(str::to_owned),
                json!({"attempt":attempt,"name":name,"snapshot":snapshot_digest,"passed":passed && unchanged && approved,"unchanged":unchanged,"details":details}),
            ))
        },
    )
}

pub fn validation_name(validation: &Validation) -> &str {
    match validation {
        Validation::Command { name, .. } | Validation::Review { name } => name,
        Validation::Scenario { id } => id,
    }
}

fn require_evidence(root: &Path, plan: &Plan, task: Option<&str>) -> Result<(), TelosError> {
    let snapshot = digest(&inventory::capture(root)?)?;
    for validation in validations(plan, task)? {
        let name = validation_name(validation);
        let last = plan.events.iter().rev().find(|e| {
            e.kind == EventKind::ValidationRecorded
                && e.revision == plan.revision().number
                && e.task.as_deref() == task
                && e.data["name"] == name
        });
        if !last.is_some_and(|e| e.data["passed"] == true && e.data["snapshot"] == snapshot) {
            return Err(TelosError::new(
                ErrorCode::TelosPlanValidationFailed,
                format!("validation `{name}` has no passing evidence for the current repository"),
            ));
        }
    }
    Ok(())
}

pub fn resume(root: &Path, id: &str) -> Result<Value, TelosError> {
    let plan = store::read(root, id)?;
    let ledger = ledger::verify(root)?;
    let snapshot = inventory::capture(root)?;
    let changes = inventory::changes(&ledger.current, &snapshot);
    let view = plan.view()?;
    let checkpoint = plan
        .events
        .iter()
        .rev()
        .find(|e| matches!(e.kind, EventKind::Checkpoint | EventKind::TaskStarted));
    let since_checkpoint = checkpoint
        .and_then(|e| e.data.get("snapshot"))
        .and_then(|v| serde_json::from_value::<inventory::Snapshot>(v.clone()).ok())
        .map(|old| inventory::changes(&old, &snapshot));
    let outside_scope: Vec<_> = changes
        .iter()
        .filter(|f| {
            view.current_task
                .as_deref()
                .and_then(|t| plan.task(t).ok())
                .is_none_or(|t| {
                    !matches_path(&t.allowed_paths, &f.path)
                        || !matches_path(&plan.revision().definition.scope, &f.path)
                })
        })
        .collect();
    Ok(
        json!({"plan":view,"ready_tasks":plan.ready_tasks()?,"checkpoint":checkpoint,
        "changes":changes,"changes_since_checkpoint":since_checkpoint,"unplanned":outside_scope,
        "unknown_validation":pending_attempt(&plan),"unknown_execution":plan.events.iter().rev().find(|e| e.kind == EventKind::EvidenceStarted && !plan.events.iter().any(|end| end.kind == EventKind::EvidenceRecorded && end.data["attempt"] == e.data["attempt"])),"head":inventory::head(root)?,
        "next_action":if !outside_scope.is_empty() {"Resolve changes outside the approved task"} else if pending_attempt(&plan).is_some() {"Inspect the interrupted validation before retrying"} else if !view.approved {"Review and approve the current plan revision"} else {"Continue the current task or start a ready task"}}),
    )
}

fn invalid(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosChangeStateInvalid, message)
}
