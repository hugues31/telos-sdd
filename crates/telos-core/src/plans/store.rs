//! The sole storage boundary for native plans and their progress.

use std::path::Path;

use serde_json::{Value, json};

use super::model::*;
use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::inventory;
use crate::repo_fs::RepoFs;
use crate::syntax::record;
use crate::transaction::{Writer, require_recovered};
use crate::work::{digest, new_id, now, validate_id};

pub struct Update {
    pub kind: EventKind,
    pub task: Option<String>,
    pub data: Value,
    pub writes: Vec<(RepoPath, Option<Vec<u8>>)>,
}

impl Update {
    pub fn event(kind: EventKind, task: Option<String>, data: Value) -> Self {
        Self {
            kind,
            task,
            data,
            writes: vec![],
        }
    }
}

pub fn path(id: &str) -> Result<RepoPath, TelosError> {
    validate_id("PLN", id)?;
    Ok(RepoPath::new(format!("telos/plans/{id}.tel")))
}

pub fn bytes(plan: &Plan) -> Result<Vec<u8>, TelosError> {
    plan.validate()?;
    Ok(record::emit("plan", &plan.id, plan)?.into_bytes())
}

pub fn decode(id: &str, bytes: &[u8]) -> Result<Plan, TelosError> {
    let source = std::str::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
    let (header, plan): (_, Plan) = record::parse("plan", source)?;
    if header != id || plan.id != id {
        return Err(invalid("plan filename, header and identity do not match"));
    }
    plan.validate()?;
    Ok(plan)
}

pub fn read(root: &Path, id: &str) -> Result<Plan, TelosError> {
    require_recovered(root)?;
    let file = path(id)?;
    let bytes = RepoFs::open(root)?.read_optional(&file)?.ok_or_else(|| {
        TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("unknown plan `{id}`"),
        )
    })?;
    decode(id, &bytes)
}

pub fn list(root: &Path) -> Result<Vec<Plan>, TelosError> {
    require_recovered(root)?;
    let fs = RepoFs::open(root)?;
    fs.list_files(&RepoPath::new("telos/plans"))?
        .into_iter()
        .filter_map(|name| name.strip_suffix(".tel").map(str::to_owned))
        .map(|id| read(root, &id))
        .collect()
}

pub fn open(root: &Path, title: &str, request_id: &str) -> Result<Value, TelosError> {
    let writer = Writer::acquire(root)?;
    let input = json!({"title": title});
    for plan in list(root)? {
        if let Some(result) = plan.replay(request_id, "plan.open", &input)? {
            return Ok(result);
        }
    }
    if title.trim().is_empty() {
        return Err(invalid("a plan needs a title"));
    }
    let at = now();
    let id = new_id("PLN")?;
    let mut plan = Plan {
        format: FORMAT,
        id: id.clone(),
        created_at: at.clone(),
        events: vec![],
        revisions: vec![Revision {
            number: 1,
            created_at: at,
            base_head: inventory::head(root)?,
            base_digest: digest(&inventory::capture(root)?)?,
            definition: Definition {
                title: title.into(),
                request: title.into(),
                ..Default::default()
            },
        }],
    };
    let result = json!({"plan": id, "version": 1, "state": "draft"});
    plan.record(
        EventKind::Opened,
        None,
        input.clone(),
        request(request_id.into(), "plan.open", &input, result.clone())?,
    )?;
    writer.publish(vec![(path(&id)?, Some(bytes(&plan)?))])?;
    Ok(result)
}

pub fn update(
    root: &Path,
    id: &str,
    request_id: &str,
    expected: Option<u64>,
    operation: &str,
    input: &Value,
    apply: impl FnOnce(&mut Plan) -> Result<Update, TelosError>,
) -> Result<Value, TelosError> {
    let writer = Writer::acquire(root)?;
    let mut plan = read(root, id)?;
    if let Some(result) = plan.replay(request_id, operation, input)? {
        return Ok(result);
    }
    for other in list(root)?.into_iter().filter(|p| p.id != id) {
        if other.events.iter().any(|e| e.request.id == request_id) {
            return Err(TelosError::new(
                ErrorCode::TelosRequestIdConflict,
                "request identity belongs to another plan",
            ));
        }
    }
    check_version(&plan, expected)?;
    let mut update = apply(&mut plan)?;
    let result = json!({"plan": id, "version": plan.version() + 1, "result": update.data});
    plan.record(
        update.kind,
        update.task,
        update.data,
        request(request_id.into(), operation, input, result.clone())?,
    )?;
    update.writes.push((path(id)?, Some(bytes(&plan)?)));
    writer.publish(update.writes)?;
    Ok(result)
}

pub fn require_editable(plan: &Plan) -> Result<(), TelosError> {
    if matches!(
        plan.view()?.state,
        PlanState::Completed | PlanState::Cancelled
    ) {
        return Err(invalid("a terminal plan is immutable; open a new plan"));
    }
    Ok(())
}

pub fn require_approved(plan: &Plan) -> Result<(), TelosError> {
    let view = plan.view()?;
    if !view.approved
        || matches!(
            view.state,
            PlanState::Paused | PlanState::Completed | PlanState::Cancelled
        )
    {
        return Err(TelosError::new(ErrorCode::TelosPlanNotApproved, "the plan has no executable approval")
            .hint("review the current plan revision and approve its exact digest, or continue the paused plan"));
    }
    Ok(())
}

pub fn active(root: &Path) -> Result<Option<(Plan, TaskView)>, TelosError> {
    let mut active = None;
    for plan in list(root)? {
        let view = plan.view()?;
        for task in view.tasks.into_iter().filter(|t| {
            t.state == TaskState::InProgress
                || (t.state == TaskState::Blocked && t.change.is_some())
        }) {
            if active.is_some() {
                return Err(invalid(
                    "multiple tasks claim this worktree; resolve the conflicting plan histories",
                ));
            }
            active = Some((plan.clone(), task));
        }
    }
    Ok(active)
}

pub fn for_change(root: &Path, change: &str) -> Result<(Plan, TaskView), TelosError> {
    let mut found = None;
    for plan in list(root)? {
        for task in plan.view()?.tasks {
            if task.change.as_deref() == Some(change) {
                if found.is_some() {
                    return Err(invalid(format!(
                        "change `{change}` belongs to multiple tasks"
                    )));
                }
                found = Some((plan.clone(), task));
            }
        }
    }
    found.ok_or_else(|| {
        TelosError::new(
            ErrorCode::TelosPlanRequired,
            format!("change `{change}` is not attached to a plan task"),
        )
    })
}

fn invalid(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosHistoryConflict, message)
}
