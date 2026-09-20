//! Versioned execution contracts and their append-only progress journal.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::work::{digest, new_id, now, validate_id};

pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Brief {
    pub summary: String,
    pub users: Vec<String>,
    pub exclusions: Vec<String>,
    pub decisions: Vec<Decision>,
    pub questions: Vec<Question>,
    pub brainstormed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub id: String,
    pub text: String,
    pub state: DecisionState,
    pub source: DecisionSource,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionState {
    Proposed,
    Accepted,
    Rejected,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    User,
    ExistingInstruction,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub text: String,
    pub blocking: bool,
    #[serde(default)]
    pub answer: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Behavior,
    Refactor,
    Tests,
    Tooling,
    Docs,
    Integration,
    Recovery,
    Bootstrap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Validation {
    Command { name: String, argv: Vec<String> },
    Scenario { id: String },
    Review { name: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Task {
    pub id: String,
    /// Optional preallocated identity for reproducible plan imports. Omitted
    /// identities are allocated randomly when the task is prepared or started.
    pub change_id: Option<String>,
    pub title: String,
    pub kind: TaskKind,
    pub depends_on: Vec<String>,
    pub targets: Vec<String>,
    pub identity_moves: BTreeMap<String, String>,
    pub allowed_paths: Vec<String>,
    pub spec_delta: String,
    pub acceptance: Vec<String>,
    pub validation: Vec<Validation>,
    pub next_action: String,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Definition {
    pub title: String,
    pub request: String,
    pub goal: String,
    pub success_criteria: Vec<String>,
    pub brief: Brief,
    pub scope: Vec<String>,
    pub tasks: Vec<Task>,
    pub validation: Vec<Validation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub number: u64,
    pub created_at: String,
    pub base_head: Option<String>,
    pub base_digest: String,
    pub definition: Definition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Opened,
    Revised,
    Approved,
    ChangePrepared,
    TaskStarted,
    Checkpoint,
    TaskBlocked,
    TaskUnblocked,
    TaskCompleted,
    Paused,
    Continued,
    Cancelled,
    Completed,
    ValidationStarted,
    ValidationRecorded,
    ChangeReconciled,
    ChangeAbandoned,
    EvidenceStarted,
    EvidenceRecorded,
    Recovered,
    Integrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub operation: String,
    pub input_digest: String,
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub version: u64,
    pub at: String,
    pub revision: u64,
    pub task: Option<String>,
    pub kind: EventKind,
    pub data: Value,
    pub request: Request,
    pub previous: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub format: u32,
    pub id: String,
    pub created_at: String,
    pub revisions: Vec<Revision>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    #[default]
    Todo,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    Draft,
    Ready,
    Approved,
    Running,
    Paused,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub percent: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub definition: Task,
    pub state: TaskState,
    pub change: Option<String>,
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanView {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub revision: u64,
    pub version: u64,
    pub digest: String,
    pub state: PlanState,
    pub approved: bool,
    pub progress: Progress,
    pub current_task: Option<String>,
    pub last_activity: String,
    pub tasks: Vec<TaskView>,
    pub brief: Brief,
    pub success_criteria: Vec<String>,
    pub scope: Vec<String>,
    pub validation: Vec<Validation>,
    pub events: Vec<Event>,
}

impl Plan {
    pub fn revision(&self) -> &Revision {
        self.revisions.last().expect("validated nonempty revisions")
    }
    pub fn version(&self) -> u64 {
        self.events.last().map_or(0, |e| e.version)
    }
    pub fn definition_digest(&self) -> Result<String, TelosError> {
        digest(self.revision())
    }

    pub fn validate(&self) -> Result<(), TelosError> {
        validate_id("PLN", &self.id)?;
        if self.format != FORMAT || self.revisions.is_empty() {
            return Err(invalid("unsupported or incomplete plan format"));
        }
        for (n, revision) in self.revisions.iter().enumerate() {
            if revision.number != n as u64 + 1 {
                return Err(invalid("plan revision sequence is incomplete"));
            }
            validate_definition(&revision.definition, false)?;
        }
        let mut previous = String::new();
        let mut requests = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for (n, event) in self.events.iter().enumerate() {
            validate_id("EVT", &event.id)?;
            if event.version != n as u64 + 1
                || event.previous != previous
                || event.revision == 0
                || event.revision > self.revisions.len() as u64
                || !requests.insert(&event.request.id)
                || !identities.insert(&event.id)
            {
                return Err(invalid(
                    "plan journal has a broken chain or duplicate event/request",
                ));
            }
            if let Some(task) = &event.task
                && !self.revisions[event.revision as usize - 1]
                    .definition
                    .tasks
                    .iter()
                    .any(|t| &t.id == task)
            {
                return Err(invalid(format!("event references unknown task `{task}`")));
            }
            if event.kind == EventKind::Approved {
                let expected = digest(&self.revisions[event.revision as usize - 1])?;
                if event.data.get("digest").and_then(Value::as_str) != Some(expected.as_str()) {
                    return Err(invalid("plan approval no longer matches its revision"));
                }
            }
            previous = digest(event)?;
        }
        self.validate_transitions()
    }

    fn validate_transitions(&self) -> Result<(), TelosError> {
        let mut revision = 1;
        let mut approved = false;
        let mut paused = false;
        let mut terminal = false;
        let mut started = BTreeSet::new();
        let mut done = BTreeSet::new();
        let mut reconciled = BTreeSet::new();
        let bootstrap =
            self.events.first().is_some_and(|e| {
                e.kind == EventKind::Opened && e.data["source"] == "explicit_init"
            }) && self.revisions.len() == 1
                && self
                    .revision()
                    .definition
                    .tasks
                    .iter()
                    .all(|t| t.kind == TaskKind::Bootstrap);
        if self
            .events
            .first()
            .is_none_or(|e| e.kind != EventKind::Opened || e.revision != 1)
        {
            return Err(invalid("a plan journal must begin with its opening event"));
        }
        for (index, event) in self.events.iter().enumerate() {
            if terminal {
                return Err(invalid("a terminal plan cannot receive new events"));
            }
            if event.kind == EventKind::Revised {
                revision += 1;
                approved = false;
                paused = false;
                if event.data["digest"] != digest(&self.revisions[event.revision as usize - 1])? {
                    return Err(invalid("revision event digest mismatch"));
                }
            }
            if event.revision != revision {
                return Err(invalid("event revision is not the current revision"));
            }
            let definition = &self.revisions[revision as usize - 1].definition;
            let task = event.task.as_deref();
            match event.kind {
                EventKind::Opened if index != 0 => return Err(invalid("duplicate opening event")),
                EventKind::Approved => {
                    validate_definition(definition, true)?;
                    approved = true;
                    paused = false;
                }
                EventKind::Paused => paused = true,
                EventKind::Continued => {
                    if !approved || !paused {
                        return Err(invalid("invalid plan continuation"));
                    }
                    paused = false;
                }
                EventKind::TaskStarted => {
                    let id = task.ok_or_else(|| invalid("task start needs an identity"))?;
                    let definition = definition
                        .tasks
                        .iter()
                        .find(|t| t.id == id)
                        .expect("validated task");
                    if !approved
                        || paused
                        || (!started.is_empty()
                            && !(started.len() == 1
                                && started.contains(id)
                                && reconciled.contains(id)))
                        || done.contains(id)
                        || definition.cancelled
                        || !definition
                            .depends_on
                            .iter()
                            .all(|dep| done.contains(dep.as_str()))
                    {
                        return Err(invalid(
                            "task started outside its approved dependency order",
                        ));
                    }
                    crate::work::validate_id(
                        "CHG",
                        event.data["change"]
                            .as_str()
                            .ok_or_else(|| invalid("task start needs a change"))?,
                    )?;
                    reconciled.remove(id);
                    started.insert(id);
                }
                EventKind::ChangeReconciled => {
                    let id = task.ok_or_else(|| invalid("reconciliation needs a task"))?;
                    if !approved || paused || !started.contains(id) {
                        return Err(invalid("reconciliation has no executing task"));
                    }
                    reconciled.insert(id);
                }
                EventKind::ChangeAbandoned => {
                    if let Some(id) = task {
                        started.remove(id);
                        reconciled.remove(id);
                    }
                }
                EventKind::ValidationRecorded => {
                    if !self.events[..index].iter().any(|start| {
                        start.kind == EventKind::ValidationStarted
                            && start.task == event.task
                            && start.data["attempt"] == event.data["attempt"]
                            && start.data["snapshot"] == event.data["snapshot"]
                    }) {
                        return Err(invalid("validation result has no matching start"));
                    }
                }
                EventKind::TaskCompleted | EventKind::Completed => {
                    if !approved || paused {
                        return Err(invalid("completion requires current approval"));
                    }
                    if !bootstrap {
                        let validations = if let Some(id) = task {
                            if !started.contains(id) || !reconciled.contains(id) {
                                return Err(invalid("task completion needs a reconciled change"));
                            }
                            &definition
                                .tasks
                                .iter()
                                .find(|t| t.id == id)
                                .expect("validated task")
                                .validation
                        } else {
                            if definition
                                .tasks
                                .iter()
                                .any(|t| !t.cancelled && !done.contains(t.id.as_str()))
                            {
                                return Err(invalid("plan completed before its tasks"));
                            }
                            &definition.validation
                        };
                        for validation in validations {
                            let name = super::execution::validation_name(validation);
                            let evidence = self.events[..index].iter().rev().find(|e| {
                                e.kind == EventKind::ValidationRecorded
                                    && e.revision == revision
                                    && e.task.as_deref() == task
                                    && e.data["name"] == name
                            });
                            if evidence.is_none_or(|e| {
                                e.data["passed"] != true
                                    || e.data["snapshot"] != event.data["snapshot"]
                            }) {
                                return Err(invalid("completion has no current passing evidence"));
                            }
                        }
                    }
                    if event.kind == EventKind::TaskCompleted {
                        let id =
                            task.ok_or_else(|| invalid("task completion needs an identity"))?;
                        started.remove(id);
                        done.insert(id);
                    } else {
                        terminal = true;
                    }
                }
                EventKind::Cancelled => terminal = true,
                _ => {}
            }
        }
        if revision != self.revisions.len() as u64 {
            return Err(invalid("revision has no journal event"));
        }
        Ok(())
    }

    pub fn record(
        &mut self,
        kind: EventKind,
        task: Option<String>,
        data: Value,
        request: Request,
    ) -> Result<(), TelosError> {
        let event = Event {
            id: new_id("EVT")?,
            version: self.version() + 1,
            at: now(),
            revision: self.revision().number,
            task,
            kind,
            data,
            request,
            previous: self
                .events
                .last()
                .map(digest)
                .transpose()?
                .unwrap_or_default(),
        };
        self.events.push(event);
        self.validate()
    }

    pub fn replay(
        &self,
        id: &str,
        operation: &str,
        input: &Value,
    ) -> Result<Option<Value>, TelosError> {
        let Some(event) = self.events.iter().find(|e| e.request.id == id) else {
            return Ok(None);
        };
        if event.request.operation != operation || event.request.input_digest != digest(input)? {
            return Err(TelosError::new(
                ErrorCode::TelosRequestIdConflict,
                "request identity was already used for different input",
            ));
        }
        Ok(Some(event.request.result.clone()))
    }

    pub fn task(&self, id: &str) -> Result<&Task, TelosError> {
        self.revision()
            .definition
            .tasks
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| {
                TelosError::new(
                    ErrorCode::TelosReferenceUnknown,
                    format!("unknown task `{id}` in {}", self.id),
                )
            })
    }

    pub fn task_view(&self, task: &Task) -> TaskView {
        let mut view = TaskView {
            definition: task.clone(),
            state: TaskState::Todo,
            change: None,
            blocker: None,
        };
        for event in self
            .events
            .iter()
            .filter(|e| e.task.as_deref() == Some(task.id.as_str()))
        {
            match event.kind {
                EventKind::ChangePrepared | EventKind::TaskStarted => {
                    if event.kind == EventKind::TaskStarted {
                        view.state = TaskState::InProgress;
                    }
                    view.change = event
                        .data
                        .get("change")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(view.change);
                }
                EventKind::TaskBlocked => {
                    view.state = TaskState::Blocked;
                    view.blocker = event
                        .data
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                EventKind::TaskUnblocked => {
                    view.state = if view.change.is_some() {
                        TaskState::InProgress
                    } else {
                        TaskState::Todo
                    };
                    view.blocker = None;
                }
                EventKind::TaskCompleted => {
                    view.state = TaskState::Done;
                    view.blocker = None;
                }
                EventKind::ChangeAbandoned => {
                    view.change = None;
                    view.state = TaskState::Todo;
                }
                _ => {}
            }
        }
        if task.cancelled {
            view.state = TaskState::Cancelled;
        }
        view
    }

    pub fn view(&self) -> Result<PlanView, TelosError> {
        self.validate()?;
        let revision = self.revision();
        let definition = &revision.definition;
        let digest = self.definition_digest()?;
        let approved = self.events.iter().any(|e| {
            e.kind == EventKind::Approved
                && e.revision == revision.number
                && e.data["digest"] == digest
        });
        let tasks: Vec<_> = definition.tasks.iter().map(|t| self.task_view(t)).collect();
        let done = tasks.iter().filter(|t| t.state == TaskState::Done).count();
        let total = tasks
            .iter()
            .filter(|t| t.state != TaskState::Cancelled)
            .count();
        let current_task = tasks
            .iter()
            .find(|t| {
                t.state == TaskState::InProgress
                    || (t.state == TaskState::Blocked && t.change.is_some())
            })
            .map(|t| t.definition.id.clone());
        let mut state = if approved {
            PlanState::Approved
        } else if validate_definition(definition, true).is_ok() {
            PlanState::Ready
        } else {
            PlanState::Draft
        };
        for event in &self.events {
            match event.kind {
                EventKind::TaskStarted | EventKind::Continued => state = PlanState::Running,
                EventKind::Paused => state = PlanState::Paused,
                EventKind::Completed => state = PlanState::Completed,
                EventKind::Cancelled => state = PlanState::Cancelled,
                EventKind::Revised => {
                    state = if approved {
                        PlanState::Approved
                    } else if validate_definition(definition, true).is_ok() {
                        PlanState::Ready
                    } else {
                        PlanState::Draft
                    }
                }
                EventKind::Approved if event.revision == revision.number => {
                    state = PlanState::Approved
                }
                _ => {}
            }
        }
        if matches!(state, PlanState::Running | PlanState::Approved)
            && tasks
                .iter()
                .any(|t| t.state == TaskState::Blocked && t.change.is_some())
        {
            state = PlanState::Blocked;
        }
        if matches!(state, PlanState::Running | PlanState::Approved)
            && current_task.is_none()
            && total != done
            && !tasks.iter().any(|t| {
                t.state == TaskState::Todo
                    && t.definition.depends_on.iter().all(|id| {
                        tasks
                            .iter()
                            .any(|d| &d.definition.id == id && d.state == TaskState::Done)
                    })
            })
        {
            state = PlanState::Blocked;
        }
        Ok(PlanView {
            id: self.id.clone(),
            title: definition.title.clone(),
            goal: definition.goal.clone(),
            revision: revision.number,
            version: self.version(),
            digest,
            state,
            approved,
            progress: Progress {
                done,
                total,
                percent: (total > 0).then(|| (100 * done / total) as u8),
            },
            current_task,
            last_activity: self
                .events
                .last()
                .map_or(&self.created_at, |e| &e.at)
                .clone(),
            tasks,
            brief: definition.brief.clone(),
            success_criteria: definition.success_criteria.clone(),
            scope: definition.scope.clone(),
            validation: definition.validation.clone(),
            events: self.events.clone(),
        })
    }

    pub fn ready_tasks(&self) -> Result<Vec<String>, TelosError> {
        let view = self.view()?;
        if !view.approved
            || matches!(
                view.state,
                PlanState::Paused | PlanState::Completed | PlanState::Cancelled
            )
        {
            return Ok(vec![]);
        }
        Ok(view
            .tasks
            .iter()
            .filter(|t| {
                t.state == TaskState::Todo
                    && t.definition.depends_on.iter().all(|id| {
                        view.tasks
                            .iter()
                            .any(|d| &d.definition.id == id && d.state == TaskState::Done)
                    })
            })
            .map(|t| t.definition.id.clone())
            .collect())
    }

    pub fn ensure_revision_preserves_completed(
        &self,
        definition: &Definition,
    ) -> Result<(), TelosError> {
        for task in &self.revision().definition.tasks {
            if self.task_view(task).change.is_some()
                && !definition.tasks.iter().any(|t| t.id == task.id)
            {
                return Err(invalid(
                    "abandon a prepared change before removing its task",
                ));
            }
            if self.task_view(task).state == TaskState::Done
                && definition.tasks.iter().find(|t| t.id == task.id) != Some(task)
            {
                return Err(invalid(format!(
                    "completed task `{}` is immutable; add a new task",
                    task.id
                )));
            }
        }
        Ok(())
    }
}

pub fn validate_definition(definition: &Definition, ready: bool) -> Result<(), TelosError> {
    let mut ids = BTreeSet::new();
    for task in &definition.tasks {
        if let Some(id) = &task.change_id {
            crate::work::validate_id("CHG", id)?;
        }
        let number = task.id.strip_prefix("TSK-").unwrap_or("");
        if number.len() < 3 || !number.bytes().all(|b| b.is_ascii_digit()) || !ids.insert(&task.id)
        {
            return Err(invalid(format!(
                "invalid or duplicate task identity `{}`",
                task.id
            )));
        }
        validate_paths(&task.allowed_paths)?;
        if ready
            && !task.cancelled
            && (task.title.trim().is_empty()
                || task.acceptance.is_empty()
                || task.validation.is_empty())
        {
            return Err(invalid(format!(
                "task `{}` needs a title, acceptance criteria and validation",
                task.id
            )));
        }
        for validation in &task.validation {
            match validation {
                Validation::Command { name, argv }
                    if name.trim().is_empty()
                        || argv.is_empty()
                        || argv[0].trim().is_empty()
                        || argv.iter().any(|arg| arg.contains('\0')) =>
                {
                    return Err(invalid("invalid validation command"));
                }
                Validation::Scenario { id } if id.parse::<crate::ids::ScenarioId>().is_err() => {
                    return Err(invalid("invalid validation scenario"));
                }
                Validation::Review { name } if name.trim().is_empty() => {
                    return Err(invalid("review validation needs a name"));
                }
                _ => {}
            }
        }
    }
    validate_paths(&definition.scope)?;
    for validations in std::iter::once(&definition.validation)
        .chain(definition.tasks.iter().map(|t| &t.validation))
    {
        let mut names = BTreeSet::new();
        for validation in validations {
            let name = super::execution::validation_name(validation);
            if name.trim().is_empty() || !names.insert(name) {
                return Err(invalid(
                    "validation names must be nonempty and unique within their task or plan",
                ));
            }
            if let Validation::Command { argv, .. } = validation
                && (argv.is_empty()
                    || argv[0].trim().is_empty()
                    || argv.iter().any(|a| a.contains('\0')))
            {
                return Err(invalid("invalid validation command"));
            }
        }
    }
    let tasks: BTreeMap<_, _> = definition.tasks.iter().map(|t| (&t.id, t)).collect();
    for task in &definition.tasks {
        for dep in &task.depends_on {
            if !ids.contains(dep) {
                return Err(invalid(format!("unknown dependency `{dep}`")));
            }
        }
    }
    let mut visited = BTreeSet::new();
    fn visit<'a>(
        id: &'a String,
        tasks: &BTreeMap<&'a String, &'a Task>,
        seen: &mut BTreeSet<&'a String>,
        active: &mut BTreeSet<&'a String>,
    ) -> Result<(), TelosError> {
        if seen.contains(id) {
            return Ok(());
        }
        if !active.insert(id) {
            return Err(TelosError::new(
                ErrorCode::TelosCycleDetected,
                "task dependencies contain a cycle",
            ));
        }
        for dep in &tasks[id].depends_on {
            visit(dep, tasks, seen, active)?;
        }
        active.remove(id);
        seen.insert(id);
        Ok(())
    }
    for id in tasks.keys() {
        visit(id, &tasks, &mut visited, &mut BTreeSet::new())?;
    }
    if ready {
        if definition.title.trim().is_empty()
            || definition.goal.trim().is_empty()
            || definition.request.trim().is_empty()
            || definition.success_criteria.is_empty()
            || definition.tasks.iter().all(|t| t.cancelled)
            || definition.validation.is_empty()
            || !definition.brief.brainstormed
            || definition.brief.summary.trim().is_empty()
        {
            return Err(invalid(
                "complete the brief, goal, success criteria and tasks before approval",
            ));
        }
        if definition
            .brief
            .questions
            .iter()
            .any(|q| q.blocking && q.answer.as_ref().is_none_or(|a| a.trim().is_empty()))
        {
            return Err(invalid("the plan has unanswered blocking questions"));
        }
        if definition
            .brief
            .decisions
            .iter()
            .any(|d| d.state == DecisionState::Accepted && d.source == DecisionSource::Agent)
        {
            return Err(invalid(
                "an agent proposal is not an accepted user decision",
            ));
        }
    }
    Ok(())
}

fn validate_paths(paths: &[String]) -> Result<(), TelosError> {
    for path in paths {
        RepoPath::parse(path)?;
        if path.starts_with(".git/") || path == ".git" || path.starts_with("telos/.runtime/") {
            return Err(invalid("plan paths cannot target Git or runtime metadata"));
        }
        globset::GlobBuilder::new(path)
            .literal_separator(true)
            .build()
            .map_err(|e| invalid(e.to_string()))?;
    }
    Ok(())
}

pub fn matches_path(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|pattern| {
        globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .is_ok_and(|glob| glob.compile_matcher().is_match(path))
    })
}

pub fn request(
    id: String,
    operation: &str,
    input: &Value,
    result: Value,
) -> Result<Request, TelosError> {
    if id.trim().is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        return Err(invalid("invalid request identity"));
    }
    Ok(Request {
        id,
        operation: operation.into(),
        input_digest: digest(input)?,
        result,
    })
}

pub fn check_version(plan: &Plan, expected: Option<u64>) -> Result<(), TelosError> {
    if expected.is_some_and(|v| v != plan.version()) {
        return Err(TelosError::new(
            ErrorCode::TelosPlanVersionStale,
            format!(
                "plan {} is at version {}, not {}",
                plan.id,
                plan.version(),
                expected.unwrap()
            ),
        )
        .hint("read `telos plan resume --json` before updating the plan"));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosIntegrityViolation, message)
}

pub fn empty_event_result(plan: &Plan) -> Value {
    json!({"plan": plan.id, "version": plan.version() + 1})
}
