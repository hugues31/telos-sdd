//! Plan definition, approval and progress transitions shared by CLI consumers.

use std::path::Path;

use serde_json::{Value, json};

use super::{model::*, store};
use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::inventory;
use crate::model::{Change, ChangeStatus};
use crate::syntax::parse_change_file;
use crate::work::{digest, now};

pub fn revise(
    root: &Path,
    id: &str,
    definition: Definition,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    let input = serde_json::to_value(&definition).expect("definition");
    store::update(
        root,
        id,
        request_id,
        expected,
        "plan.edit",
        &input,
        |plan| {
            store::require_editable(plan)?;
            validate_definition(&definition, false)?;
            plan.ensure_revision_preserves_completed(&definition)?;
            if let Some(active) = plan.view()?.current_task {
                let change = plan.task_view(plan.task(&active)?).change;
                if super::ledger::receipts(root)?
                    .iter()
                    .any(|r| Some(&r.id) == change.as_ref())
                {
                    return Err(state_error(
                        "finish the reconciled task or start it again for rework before revising",
                    ));
                }
                if plan.view()?.state != PlanState::Paused
                    || !definition
                        .tasks
                        .iter()
                        .any(|t| t.id == active && !t.cancelled)
                {
                    return Err(state_error(
                        "pause before revising an active task, and retain its identity",
                    ));
                }
            }
            for task in &definition.tasks {
                parse_delta(&task.spec_delta)?;
            }
            plan.revisions.push(Revision {
                number: plan.revision().number + 1,
                created_at: now(),
                base_head: inventory::head(root)?,
                base_digest: digest(&inventory::capture(root)?)?,
                definition,
            });
            Ok(store::Update::event(
                EventKind::Revised,
                None,
                json!({"revision": plan.revision().number, "digest": plan.definition_digest()?}),
            ))
        },
    )
}

pub fn approve(
    root: &Path,
    id: &str,
    expected_digest: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    store::update(
        root,
        id,
        request_id,
        expected,
        "plan.approve",
        &json!({"digest": expected_digest}),
        |plan| {
            store::require_editable(plan)?;
            validate_definition(&plan.revision().definition, true)?;
            if plan.definition_digest()? != expected_digest {
                return Err(TelosError::new(
                    ErrorCode::TelosApprovalStale,
                    "the plan revision changed after review",
                ));
            }
            if plan.revision().base_digest != digest(&inventory::capture(root)?)? {
                return Err(TelosError::new(
                    ErrorCode::TelosApprovalStale,
                    "the repository changed after this plan revision was prepared",
                )
                .hint("prepare a fresh revision against the current verified repository state"));
            }
            let mut combined = Vec::new();
            let mut ordered = std::collections::BTreeSet::new();
            while ordered.len() < plan.revision().definition.tasks.len() {
                for task in &plan.revision().definition.tasks {
                    if ordered.contains(&task.id)
                        || !task.depends_on.iter().all(|d| ordered.contains(d))
                    {
                        continue;
                    }
                    ordered.insert(task.id.clone());
                    if task.cancelled || plan.task_view(task).state == TaskState::Done {
                        continue;
                    }
                    let ops = parse_delta(&task.spec_delta)?;
                    for path in ops
                        .iter()
                        .flat_map(|op| [Some(op.target_path()), op.source_path()])
                        .flatten()
                    {
                        if !matches_path(&task.allowed_paths, path.as_str())
                            || !matches_path(&plan.revision().definition.scope, path.as_str())
                        {
                            return Err(TelosError::new(
                                ErrorCode::TelosPlanScopeViolation,
                                format!("planned delta targets `{path}` outside the task scope"),
                            ));
                        }
                    }
                    combined.extend(ops);
                }
            }
            if !combined.is_empty()
                && let Ok(ws) = crate::workspace::Workspace::discover(root)
            {
                crate::config::Config::validate_transition(
                    &ws.config,
                    &crate::overlay::apply_config_ops(&ws.config, &combined),
                )?;
                crate::overlay::validate_ops_idempotent(&ws, &combined)
                    .map_err(|e| TelosError::from(e.into_iter().next().expect("diagnostic")))?;
            }
            if let Some(task) = plan.view()?.current_task {
                super::ledger::require_scope(
                    root,
                    plan,
                    plan.task(&task)?,
                    &inventory::capture(root)?,
                )?;
            } else if !plan
                .revision()
                .definition
                .tasks
                .iter()
                .any(|t| matches!(t.kind, TaskKind::Recovery | TaskKind::Integration))
            {
                super::ledger::require_clean(root)?;
            }
            let mut update = store::Update::event(
                EventKind::Approved,
                None,
                json!({"digest": expected_digest, "source": "human", "baseline":digest(&super::ledger::read(root)?.current)?}),
            );
            if let Some(task) = plan.view()?.current_task {
                let task = plan.task(&task)?;
                let id = plan
                    .task_view(task)
                    .change
                    .ok_or_else(|| state_error("active task has no change"))?
                    .parse()?;
                let change = prepared_change(task, id)?;
                update.writes.push((
                    RepoPath::new(format!("telos/changes/{id}.tel")),
                    Some(crate::emit::emit_change(&change).into_bytes()),
                ));
            }
            Ok(update)
        },
    )
}

pub fn checkpoint(
    root: &Path,
    id: &str,
    summary: &str,
    next_action: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    let input = json!({"summary": summary, "next_action": next_action});
    store::update(
        root,
        id,
        request_id,
        expected,
        "plan.checkpoint",
        &input,
        |plan| {
            store::require_editable(plan)?;
            let view = plan.view()?;
            let snapshot = inventory::capture(root)?;
            if let Some(task) = view.current_task.as_deref() {
                super::ledger::require_scope(root, plan, plan.task(task)?, &snapshot)?;
            }
            Ok(store::Update::event(
                EventKind::Checkpoint,
                view.current_task,
                json!({"summary": summary, "next_action": next_action, "snapshot": snapshot, "head": inventory::head(root)?}),
            ))
        },
    )
}

pub fn transition(
    root: &Path,
    id: &str,
    task: Option<&str>,
    kind: EventKind,
    reason: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    let input = json!({"task": task, "kind": kind, "reason": reason});
    store::update(
        root,
        id,
        request_id,
        expected,
        "plan.transition",
        &input,
        |plan| {
            store::require_editable(plan)?;
            let view = plan.view()?;
            match kind {
                EventKind::Paused => {}
                EventKind::Continued => {
                    if view.state != PlanState::Paused || !view.approved {
                        return Err(state_error("only an approved paused plan can continue"));
                    }
                    if let Some((other, _)) = store::active(root)?
                        && other.id != id
                    {
                        return Err(state_error("another plan owns this worktree"));
                    }
                }
                EventKind::TaskBlocked | EventKind::TaskUnblocked => {
                    let task = plan.task(task.ok_or_else(|| state_error("a task is required"))?)?;
                    let state = plan.task_view(task).state;
                    if matches!(state, TaskState::Done | TaskState::Cancelled) {
                        return Err(state_error("a terminal task cannot be blocked or resumed"));
                    }
                    if kind == EventKind::TaskUnblocked && state != TaskState::Blocked {
                        return Err(state_error("the task is not blocked"));
                    }
                    if reason.trim().is_empty() {
                        return Err(state_error("record a reason or resolution"));
                    }
                }
                EventKind::Cancelled => {
                    if view
                        .tasks
                        .iter()
                        .any(|t| t.change.is_some() && t.state != TaskState::Done)
                    {
                        return Err(state_error(
                            "resolve or abandon every open change before cancelling the plan",
                        ));
                    }
                }
                _ => return Err(state_error("unsupported direct plan transition")),
            }
            Ok(store::Update::event(
                kind,
                task.map(str::to_owned),
                json!({"reason": reason}),
            ))
        },
    )
}

pub fn parse_delta(source: &str) -> Result<Vec<crate::model::StagedOp>, TelosError> {
    if source.trim().is_empty() {
        return Ok(vec![]);
    }
    let source = format!(
        "change {} \"Plan delta\" {{\n status drafted\n{source}\n}}\n",
        crate::ids::ChangeId(0)
    );
    parse_change_file(&RepoPath::new("telos/changes/plan-delta.tel"), &source)
        .map(|c| c.ops)
        .map_err(|errors| errors.into_iter().next().expect("parse diagnostic").into())
}

pub fn canonical_delta(change: &Change) -> String {
    change
        .ops
        .iter()
        .map(crate::emit::emit_op)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn prepare(
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
        "plan.task.prepare",
        &json!({"task":task_id}),
        |plan| {
            store::require_editable(plan)?;
            if plan.view()?.approved {
                return Err(state_error(
                    "revise an approved plan before preparing a different delta",
                ));
            }
            let task = plan.task(task_id)?;
            if let Some(change) = plan.task_view(task).change {
                return Err(state_error(format!("task already owns {change}")));
            }
            let id = task
                .change_id
                .as_deref()
                .map(str::parse)
                .transpose()?
                .unwrap_or(crate::ids::ChangeId::allocate()?);
            if store::for_change(root, &id.to_string()).is_ok()
                || root.join(format!("telos/changes/{id}.tel")).exists()
                || root.join(format!("telos/history/{id}.tel")).exists()
            {
                return Err(state_error(
                    "the requested change identity is already in use",
                ));
            }
            let mut change = prepared_change(task, id)?;
            change.status = if change.ops.is_empty() {
                ChangeStatus::Open
            } else {
                ChangeStatus::Drafted
            };
            change.approved_digest = None;
            let mut update = store::Update::event(
                EventKind::ChangePrepared,
                Some(task_id.into()),
                json!({"change":change.id}),
            );
            update.writes.push((
                RepoPath::new(format!("telos/changes/{}.tel", change.id)),
                Some(crate::emit::emit_change(&change).into_bytes()),
            ));
            Ok(update)
        },
    )
}

pub fn import_change(
    root: &Path,
    id: &str,
    task_id: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<Value, TelosError> {
    let plan = store::read(root, id)?;
    let view = plan.task_view(plan.task(task_id)?);
    let change = view
        .change
        .ok_or_else(|| state_error("prepare a task change first"))?;
    let ws = crate::workspace::Workspace::discover(root)?;
    let change = crate::changes::read_change(&ws, change.parse()?)?;
    let mut definition = plan.revision().definition.clone();
    definition
        .tasks
        .iter_mut()
        .find(|t| t.id == task_id)
        .expect("known task")
        .spec_delta = canonical_delta(&change);
    revise(root, id, definition, request_id, expected)
}

pub fn prepared_change(task: &Task, id: crate::ids::ChangeId) -> Result<Change, TelosError> {
    let mut change = Change {
        id,
        motivation: task.title.clone(),
        status: ChangeStatus::Approved,
        approved_digest: None,
        ops: parse_delta(&task.spec_delta)?,
        journal: vec![],
    };
    change.approved_digest = Some(change.ops_digest());
    Ok(change)
}

pub fn abandon(root: &Path, id: &str, request_id: &str) -> Result<Value, TelosError> {
    let (plan, task) = store::for_change(root, id)?;
    store::update(
        root,
        &plan.id,
        request_id,
        None,
        "change.abandon",
        &json!({"change":id}),
        |plan| {
            store::require_editable(plan)?;
            super::ledger::require_clean(root)?;
            let path = RepoPath::new(format!("telos/changes/{id}.tel"));
            let bytes = crate::repo_fs::RepoFs::open(root)?.read(&path)?;
            let mut update = store::Update::event(
                EventKind::ChangeAbandoned,
                Some(task.definition.id.clone()),
                json!({"change":id,"record":String::from_utf8_lossy(&bytes),"reason":"explicit_abandon"}),
            );
            update.writes.push((path, None));
            Ok(update)
        },
    )
}

pub fn require_change_contract(
    root: &Path,
    change: &Change,
    executing: bool,
) -> Result<(Plan, TaskView), TelosError> {
    let (plan, task) = store::for_change(root, &change.id.to_string())?;
    if executing {
        store::require_approved(&plan)?;
        let boundary = plan.events.iter().rev().find(|e| {
            e.kind == EventKind::Approved
                || (e.kind == EventKind::TaskStarted
                    && e.task.as_deref() == Some(task.definition.id.as_str()))
        });
        if let Some(baseline) = boundary
            .and_then(|e| e.data.get("baseline"))
            .and_then(Value::as_str)
            && baseline != digest(&super::ledger::read(root)?.current)?
        {
            return Err(TelosError::new(
                ErrorCode::TelosApprovalStale,
                "the task's repository baseline changed; prepare a new revision",
            ));
        }
        if task.state != TaskState::InProgress {
            return Err(TelosError::new(
                ErrorCode::TelosPlanRequired,
                "start the owning plan task before implementation",
            ));
        }
        let planned = prepared_change(&task.definition, change.id)?;
        if planned.ops_digest() != change.ops_digest() {
            return Err(TelosError::new(
                ErrorCode::TelosPlanScopeViolation,
                "the change delta differs from the approved plan task",
            )
            .hint("prepare and approve a new plan revision before extending the delta"));
        }
    } else if plan.view()?.approved {
        return Err(TelosError::new(
            ErrorCode::TelosPlanScopeViolation,
            "approved task deltas are immutable; revise the plan instead",
        ));
    }
    Ok((plan, task))
}

fn state_error(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosChangeStateInvalid, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::git_output;

    fn definition() -> Definition {
        Definition {
            title: "Document the runner".into(),
            request: "Explain how to run tests".into(),
            goal: "A maintainer can run the tests".into(),
            success_criteria: vec!["Instructions reviewed".into()],
            brief: Brief {
                summary: "Document the existing runner; do not change behavior".into(),
                brainstormed: true,
                ..Default::default()
            },
            scope: vec!["README.md".into()],
            tasks: vec![Task {
                id: "TSK-001".into(),
                title: "Write runner instructions".into(),
                kind: TaskKind::Docs,
                allowed_paths: vec!["README.md".into()],
                acceptance: vec!["Commands are correct".into()],
                validation: vec![Validation::Review {
                    name: "Review commands".into(),
                }],
                ..Default::default()
            }],
            validation: vec![Validation::Review {
                name: "final-review".into(),
            }],
        }
    }

    #[test]
    fn approval_survives_checkpoints_and_request_replay_survives_stale_versions() {
        let root = tempfile::tempdir().unwrap();
        git_output(root.path(), &["init", "--quiet"]).unwrap();
        super::super::ledger::bootstrap(root.path()).unwrap();
        let opened = store::open(root.path(), "Documentation", "open-once").unwrap();
        assert_eq!(
            store::open(root.path(), "Documentation", "open-once").unwrap(),
            opened
        );
        let id = opened["plan"].as_str().unwrap();
        revise(root.path(), id, definition(), "define", Some(1)).unwrap();
        let plan = store::read(root.path(), id).unwrap();
        approve(
            root.path(),
            id,
            &plan.definition_digest().unwrap(),
            "approve",
            Some(2),
        )
        .unwrap();
        let first = checkpoint(
            root.path(),
            id,
            "Ready to work",
            "Start TSK-001",
            "checkpoint",
            Some(3),
        )
        .unwrap();
        assert_eq!(
            checkpoint(
                root.path(),
                id,
                "Ready to work",
                "Start TSK-001",
                "checkpoint",
                Some(3)
            )
            .unwrap(),
            first
        );
        assert!(
            store::read(root.path(), id)
                .unwrap()
                .view()
                .unwrap()
                .approved
        );
        assert_eq!(
            checkpoint(root.path(), id, "Different", "Start", "checkpoint", None)
                .unwrap_err()
                .code,
            ErrorCode::TelosRequestIdConflict
        );
        assert_eq!(
            checkpoint(root.path(), id, "New", "Start", "new", Some(3))
                .unwrap_err()
                .code,
            ErrorCode::TelosPlanVersionStale
        );
    }

    #[test]
    fn unresolved_questions_and_dependency_cycles_cannot_be_approved() {
        let mut def = definition();
        def.brief.questions.push(Question {
            id: "Q1".into(),
            text: "Which runner?".into(),
            blocking: true,
            answer: None,
        });
        assert!(validate_definition(&def, true).is_err());
        def.brief.questions.clear();
        def.tasks[0].depends_on.push("TSK-001".into());
        assert_eq!(
            validate_definition(&def, true).unwrap_err().code,
            ErrorCode::TelosCycleDetected
        );
    }
}
