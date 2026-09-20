//! Repository-wide attribution. The ledger is a projection of immutable receipts.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{actions, model::*, store};
use crate::error::{ErrorCode, TelosError};
use crate::ids::RepoPath;
use crate::inventory::{self, FileChange, Snapshot};
use crate::model::{Change, SourceKind, TelosModel};
use crate::repo_fs::RepoFs;
use crate::syntax::record;
use crate::transaction::{Writer, require_recovered};
use crate::work::{digest, new_id, now};

pub type Publication = Vec<(RepoPath, Option<Vec<u8>>)>;

pub const PATH: &str = "telos/ledger.tel";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub uid: String,
    pub selector: String,
    pub path: String,
    pub definition_digest: String,
    pub implementation_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityEvent {
    pub uid: String,
    pub selector: String,
    pub previous_selector: Option<String>,
    pub kind: String,
    pub path: String,
    pub definition_digest: String,
    pub implementation_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub format: u32,
    pub id: String,
    pub plan: String,
    pub revision: u64,
    pub approval: String,
    pub task: String,
    pub at: String,
    pub head: Option<String>,
    pub previous: String,
    pub parents: Vec<String>,
    pub before: String,
    pub after: String,
    pub files: Vec<FileChange>,
    pub observed_files: Vec<FileChange>,
    pub entities: Vec<EntityEvent>,
    pub change: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub format: u32,
    pub bootstrap_plan: String,
    pub observed_at: String,
    pub baseline: Snapshot,
    pub current: Snapshot,
    pub identities: BTreeMap<String, Identity>,
    pub initial_identities: BTreeMap<String, Identity>,
    pub incoming_identities: BTreeMap<String, Identity>,
    pub receipts: Vec<String>,
    pub frontier: String,
    pub pending_parents: Vec<String>,
}

pub fn read(root: &Path) -> Result<Ledger, TelosError> {
    require_recovered(root)?;
    let bytes = RepoFs::open(root)?
        .read_optional(&RepoPath::new(PATH))?
        .ok_or_else(|| {
            TelosError::new(
                ErrorCode::TelosPlanRequired,
                "this workspace has no native plan ledger",
            )
            .hint(
                "initialize a new workspace with this version; legacy workspaces are not migrated",
            )
        })?;
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Result<Ledger, TelosError> {
    let text = std::str::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
    let (id, ledger): (_, Ledger) = record::parse("ledger", text)?;
    if id != "repository" || ledger.format != FORMAT {
        return Err(invalid("unsupported ledger format"));
    }
    Ok(ledger)
}

pub fn receipt_path(id: &str) -> Result<RepoPath, TelosError> {
    crate::work::validate_id("CHG", id)?;
    Ok(RepoPath::new(format!("telos/history/{id}.tel")))
}

pub fn receipts(root: &Path) -> Result<Vec<Receipt>, TelosError> {
    let ledger = read(root)?;
    let fs = RepoFs::open(root)?;
    ledger
        .receipts
        .iter()
        .map(|id| {
            let bytes = fs.read(&receipt_path(id)?)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| invalid(e.to_string()))?;
            let (header, receipt): (_, Receipt) = record::parse("receipt", text)?;
            if &header != id || &receipt.id != id || receipt.format != FORMAT {
                return Err(invalid("receipt identity mismatch"));
            }
            Ok(receipt)
        })
        .collect()
}

/// Explicit initialization observes pre-existing files; it never invents an
/// implementation date for them. Only this bootstrap can create a baseline.
pub fn bootstrap(root: &Path) -> Result<(), TelosError> {
    let writer = Writer::acquire(root)?;
    if writer.read(&RepoPath::new(PATH))?.is_some() {
        return Err(invalid("a plan ledger already exists"));
    }
    let mut snapshot = inventory::capture(root)?;
    inventory::store(root, &snapshot)?;
    let ignore_path = RepoPath::new("telos/.gitignore");
    let mut ignore = writer.read(&ignore_path)?.unwrap_or_default();
    if !ignore.ends_with(b"\n") && !ignore.is_empty() {
        ignore.push(b'\n');
    }
    ignore.extend_from_slice(b".runtime/\n");
    snapshot.insert(
        ignore_path.to_string(),
        inventory::FileState {
            oid: inventory::hash_bytes(root, Some(ignore_path.as_str()), &ignore, true)?,
            mode: "100644".into(),
        },
    );
    let id = new_id("PLN")?;
    let at = now();
    let definition = Definition {
        title: "Initialize repository governance".into(),
        request: "telos init".into(),
        goal: "Track every subsequent repository change".into(),
        success_criteria: vec!["The initial inventory is recorded".into()],
        brief: Brief {
            summary: "Observe the initial repository; prior implementation dates are unknown"
                .into(),
            brainstormed: true,
            ..Default::default()
        },
        scope: vec!["**".into()],
        tasks: vec![Task {
            id: "TSK-001".into(),
            title: "Record the initial inventory".into(),
            kind: TaskKind::Bootstrap,
            allowed_paths: vec!["**".into()],
            acceptance: vec!["Inventory recorded".into()],
            validation: vec![Validation::Review {
                name: "Explicit initialization".into(),
            }],
            ..Default::default()
        }],
        validation: vec![Validation::Review {
            name: "Explicit initialization".into(),
        }],
    };
    let mut plan = Plan {
        format: FORMAT,
        id: id.clone(),
        created_at: at.clone(),
        revisions: vec![Revision {
            number: 1,
            created_at: at.clone(),
            base_head: inventory::head(root)?,
            base_digest: digest(&snapshot)?,
            definition,
        }],
        events: vec![],
    };
    for (kind, task, data) in [
        (EventKind::Opened, None, json!({"source": "explicit_init"})),
        (
            EventKind::Approved,
            None,
            json!({"digest": plan.definition_digest()?, "source": "explicit_init"}),
        ),
        (
            EventKind::TaskCompleted,
            Some("TSK-001".into()),
            json!({"observed": true, "snapshot": digest(&snapshot)?}),
        ),
        (EventKind::Completed, None, json!({"observed": true})),
    ] {
        let req = request(new_id("REQ")?, "init", &data, data.clone())?;
        plan.record(kind, task, data, req)?;
    }
    let mut identities = BTreeMap::new();
    if let Ok(ws) = crate::workspace::Workspace::discover(root)
        && let Ok(model) = ws.load_model()
    {
        for (selector, path) in entity_paths(&model) {
            let (definition_digest, implementation_digest) =
                entity_digests(&model, &selector, &snapshot)?;
            identities.insert(
                selector.clone(),
                Identity {
                    uid: new_id("ENT")?,
                    selector,
                    path,
                    definition_digest,
                    implementation_digest,
                },
            );
        }
    }
    let ledger = Ledger {
        format: FORMAT,
        bootstrap_plan: id.clone(),
        observed_at: at,
        baseline: snapshot.clone(),
        current: snapshot,
        initial_identities: identities.clone(),
        identities,
        incoming_identities: BTreeMap::new(),
        receipts: vec![],
        frontier: String::new(),
        pending_parents: vec![],
    };
    writer.publish(vec![
        (ignore_path, Some(ignore)),
        (store::path(&id)?, Some(store::bytes(&plan)?)),
        (
            RepoPath::new(PATH),
            Some(record::emit("ledger", "repository", &ledger)?.into_bytes()),
        ),
    ])?;
    Ok(())
}

pub fn require_clean(root: &Path) -> Result<(), TelosError> {
    let ledger = verify(root)?;
    if !ledger.pending_parents.is_empty() {
        return Err(invalid(
            "finish the integration change before certifying the repository",
        ));
    }
    let delta = inventory::changes(&ledger.current, &inventory::capture(root)?);
    if !delta.is_empty() {
        return Err(unplanned(&delta));
    }
    Ok(())
}

pub fn require_scope(
    root: &Path,
    plan: &Plan,
    task: &Task,
    snapshot: &Snapshot,
) -> Result<(), TelosError> {
    let ledger = read(root)?;
    let delta = inventory::changes(&ledger.current, snapshot);
    let outside: Vec<_> = delta
        .into_iter()
        .filter(|f| {
            !matches_path(&plan.revision().definition.scope, &f.path)
                || !matches_path(&task.allowed_paths, &f.path)
        })
        .collect();
    if !outside.is_empty() {
        return Err(unplanned(&outside));
    }
    Ok(())
}

/// Validates receipt links and replays every file transition from the observed
/// baseline. A deleted, reordered, duplicated or altered receipt is an error.
pub fn verify(root: &Path) -> Result<Ledger, TelosError> {
    let ledger = read(root)?;
    let receipts = receipts(root)?;
    let plans = store::list(root)?
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect();
    verify_records(&ledger, &receipts, &plans)?;
    let fs = RepoFs::open(root)?;
    for file in fs.list_files(&RepoPath::new("telos/history"))? {
        if let Some(id) = file.strip_suffix(".tel")
            && !ledger.receipts.iter().any(|known| known == id)
        {
            return Err(invalid("history contains an unattached receipt"));
        }
    }
    Ok(ledger)
}

fn verify_records(
    ledger: &Ledger,
    receipts: &[Receipt],
    plans: &BTreeMap<String, Plan>,
) -> Result<(), TelosError> {
    let bootstrap = plans
        .get(&ledger.bootstrap_plan)
        .ok_or_else(|| invalid("observed baseline plan is missing"))?;
    if bootstrap.revisions.len() != 1
        || bootstrap.revision().base_digest != digest(&ledger.baseline)?
        || bootstrap.created_at != ledger.observed_at
        || bootstrap.view()?.state != PlanState::Completed
        || bootstrap
            .events
            .first()
            .is_none_or(|e| e.data["source"] != "explicit_init")
    {
        return Err(invalid(
            "observed baseline does not match its initialization plan",
        ));
    }
    let mut snapshots = BTreeMap::from([(String::new(), ledger.baseline.clone())]);
    let mut identities = BTreeMap::from([(String::new(), ledger.initial_identities.clone())]);
    let mut ids = std::collections::BTreeSet::new();
    for receipt in receipts {
        let mut snapshot = snapshots
            .get(&receipt.previous)
            .ok_or_else(|| invalid("receipt parent is missing or out of order"))?
            .clone();
        if !ids.insert(receipt.id.clone())
            || receipt.before != digest(&snapshot)?
            || receipt.parents.iter().any(|p| !snapshots.contains_key(p))
        {
            return Err(invalid("receipt chain is incomplete or conflicting"));
        }
        let plan = plans
            .get(&receipt.plan)
            .ok_or_else(|| invalid("receipt references a missing plan"))?;
        let revision = plan
            .revisions
            .get(receipt.revision.saturating_sub(1) as usize)
            .ok_or_else(|| invalid("receipt references an unknown revision"))?;
        if digest(revision)? != receipt.approval
            || !plan.events.iter().any(|e| {
                e.kind == EventKind::Approved
                    && e.revision == receipt.revision
                    && e.data["digest"] == receipt.approval
            })
        {
            return Err(invalid("receipt has no matching plan approval"));
        }
        let task = revision
            .definition
            .tasks
            .iter()
            .find(|t| t.id == receipt.task)
            .ok_or_else(|| invalid("receipt references an unknown task"))?;
        let receipt_digest = digest(receipt)?;
        if !plan.events.iter().any(|e| {
            e.kind == EventKind::ChangeReconciled
                && e.revision == receipt.revision
                && e.task.as_deref() == Some(receipt.task.as_str())
                && e.data["receipt"] == receipt_digest
        }) {
            return Err(invalid("receipt is not attached to its plan journal"));
        }
        let change = crate::syntax::parse_change_file(
            &RepoPath::new(format!("telos/history/{}.tel", receipt.id)),
            &receipt.change,
        )
        .map_err(|_| invalid("retained change cannot be parsed"))?;
        if change.id.to_string() != receipt.id
            || actions::prepared_change(task, change.id)?.ops_digest() != change.ops_digest()
        {
            return Err(invalid(
                "receipt change does not match its approved task delta",
            ));
        }
        for file in &receipt.files {
            RepoPath::parse(&file.path)?;
            if !inventory::is_managed(&file.path)
                || snapshot.get(&file.path) != file.before.as_ref()
                || !matches_path(&task.allowed_paths, &file.path)
                || !matches_path(&revision.definition.scope, &file.path)
            {
                return Err(invalid(format!(
                    "invalid receipt transition for `{}`",
                    file.path
                )));
            }
            match &file.after {
                Some(state) => {
                    snapshot.insert(file.path.clone(), state.clone());
                }
                None => {
                    snapshot.remove(&file.path);
                }
            }
        }
        if receipt.after != digest(&snapshot)? {
            return Err(invalid("receipt result inventory mismatch"));
        }
        let mut registry = identities
            .get(&receipt.previous)
            .expect("verified parent")
            .clone();
        for event in &receipt.entities {
            crate::work::validate_id("ENT", &event.uid)?;
            if let Some(previous) = &event.previous_selector {
                registry.remove(previous);
            }
            if event.kind == "removed" {
                registry.remove(&event.selector);
            } else {
                registry.insert(
                    event.selector.clone(),
                    Identity {
                        uid: event.uid.clone(),
                        selector: event.selector.clone(),
                        path: event.path.clone(),
                        definition_digest: event.definition_digest.clone(),
                        implementation_digest: event.implementation_digest.clone(),
                    },
                );
            }
        }
        identities.insert(receipt_digest.clone(), registry);
        snapshots.insert(receipt_digest, snapshot);
    }
    if snapshots.get(&ledger.frontier) != Some(&ledger.current)
        || ledger
            .pending_parents
            .iter()
            .any(|p| !snapshots.contains_key(p))
    {
        return Err(invalid("ledger does not match its receipt frontier"));
    }
    if ledger.receipts != receipts.iter().map(|r| r.id.clone()).collect::<Vec<_>>() {
        return Err(invalid("receipt ordering mismatch"));
    }
    if identities.get(&ledger.frontier) != Some(&ledger.identities) {
        return Err(invalid("entity identities do not match retained history"));
    }
    let mut incoming = BTreeMap::new();
    for parent in &ledger.pending_parents {
        incoming.extend(identities.get(parent).expect("verified parent").clone());
    }
    if ledger.incoming_identities != incoming {
        return Err(invalid(
            "incoming entity identities do not match integration parents",
        ));
    }
    Ok(())
}

/// Prepares metadata for the same publication as the functional seal. No files
/// are changed here. Identity continuity follows explicit moves, not name reuse.
pub fn publication(
    root: &Path,
    change: &Change,
    model: &TelosModel,
    after: Snapshot,
) -> Result<Publication, TelosError> {
    let (mut plan, task) = actions::require_change_contract(root, change, true)?;
    let mut ledger = verify(root)?;
    require_scope(root, &plan, &task.definition, &after)?;
    let files = inventory::changes(&ledger.current, &after);
    let paths = entity_paths(model);
    let mut entities = Vec::new();
    let mut identities = BTreeMap::new();
    for (from, to) in &task.definition.identity_moves {
        if !ledger.identities.contains_key(from)
            || paths.contains_key(from)
            || !paths.contains_key(to)
            || ledger.identities.contains_key(to)
        {
            return Err(invalid(format!(
                "identity move `{from}` -> `{to}` must replace an existing entity with a new selector"
            )));
        }
    }
    let mut used = std::collections::BTreeSet::new();
    for (selector, path) in &paths {
        let old = ledger
            .identities
            .get(selector)
            .or_else(|| ledger.incoming_identities.get(selector))
            .or_else(|| {
                task.definition
                    .identity_moves
                    .iter()
                    .find_map(|(from, to)| {
                        (to == selector)
                            .then(|| ledger.identities.get(from))
                            .flatten()
                    })
            })
            .or_else(|| {
                change.ops.iter().find_map(|op| {
                    let source = (op.target_path().as_str() == path)
                        .then(|| op.source_path())
                        .flatten()?;
                    let candidates: Vec<_> = ledger
                        .identities
                        .values()
                        .filter(|i| i.path == source.as_str() && !paths.contains_key(&i.selector))
                        .collect();
                    (candidates.len() == 1).then(|| candidates[0])
                })
            });
        let (definition_digest, implementation_digest) = entity_digests(model, selector, &after)?;
        let identity = Identity {
            uid: old
                .map(|i| Ok(i.uid.clone()))
                .unwrap_or_else(|| new_id("ENT"))?,
            selector: selector.clone(),
            path: path.clone(),
            definition_digest,
            implementation_digest,
        };
        if !used.insert(identity.uid.clone()) {
            return Err(invalid("two entities cannot share an identity"));
        }
        let event = |kind: &str| EntityEvent {
            uid: identity.uid.clone(),
            selector: selector.clone(),
            previous_selector: old
                .filter(|i| i.selector != *selector)
                .map(|i| i.selector.clone()),
            kind: kind.into(),
            path: path.clone(),
            definition_digest: identity.definition_digest.clone(),
            implementation_digest: identity.implementation_digest.clone(),
        };
        if old.is_none() {
            entities.push(event("defined"));
        } else if old.is_some_and(|i| i.path != *path || i.selector != *selector) {
            entities.push(event("moved"));
        } else if old.is_some_and(|i| i.definition_digest != identity.definition_digest) {
            entities.push(event("definition_changed"));
        }
        if old.map_or(
            identity.implementation_digest != digest(&Vec::<serde_json::Value>::new())?,
            |i| i.implementation_digest != identity.implementation_digest,
        ) {
            entities.push(event("implementation_changed"));
        }
        // Imported identities still need a primary-parent transition even when
        // the imported definition and implementation are unchanged.
        if old.is_some()
            && !ledger.identities.values().any(|i| i.uid == identity.uid)
            && !entities.iter().any(|e| e.uid == identity.uid)
        {
            entities.push(event("integrated"));
        }
        identities.insert(selector.clone(), identity);
    }
    for old in ledger.identities.values() {
        if !identities.values().any(|i| i.uid == old.uid) {
            entities.push(EntityEvent {
                uid: old.uid.clone(),
                selector: old.selector.clone(),
                previous_selector: None,
                kind: "removed".into(),
                path: old.path.clone(),
                definition_digest: old.definition_digest.clone(),
                implementation_digest: old.implementation_digest.clone(),
            });
        }
    }
    let receipt = Receipt {
        format: FORMAT,
        id: change.id.to_string(),
        plan: plan.id.clone(),
        revision: plan.revision().number,
        approval: plan.definition_digest()?,
        task: task.definition.id.clone(),
        at: now(),
        head: inventory::head(root)?,
        previous: ledger.frontier.clone(),
        parents: ledger.pending_parents.clone(),
        before: digest(&ledger.current)?,
        after: digest(&after)?,
        files,
        observed_files: if task.definition.kind == TaskKind::Recovery {
            plan.events
                .iter()
                .rev()
                .find(|e| {
                    e.kind == EventKind::TaskStarted
                        && e.task.as_deref() == Some(task.definition.id.as_str())
                })
                .and_then(|e| serde_json::from_value::<Snapshot>(e.data["snapshot"].clone()).ok())
                .map(|observed| inventory::changes(&observed, &after))
                .unwrap_or_default()
        } else {
            vec![]
        },
        entities,
        change: crate::emit::emit_change(change),
    };
    let data =
        json!({"change": receipt.id, "receipt": digest(&receipt)?, "snapshot": receipt.after});
    plan.record(
        EventKind::ChangeReconciled,
        Some(task.definition.id),
        data.clone(),
        request(new_id("REQ")?, "change.reconcile", &data, data.clone())?,
    )?;
    ledger.current = after;
    ledger.identities = identities;
    ledger.frontier = digest(&receipt)?;
    ledger.pending_parents.clear();
    ledger.incoming_identities.clear();
    ledger.receipts.push(receipt.id.clone());
    Ok(vec![
        (
            receipt_path(&receipt.id)?,
            Some(record::emit("receipt", &receipt.id, &receipt)?.into_bytes()),
        ),
        (
            RepoPath::new(PATH),
            Some(record::emit("ledger", "repository", &ledger)?.into_bytes()),
        ),
        (store::path(&plan.id)?, Some(store::bytes(&plan)?)),
    ])
}

/// Semantic serialization drops parser spans, so formatting and neighboring
/// entities do not create false definition events. Bindings include their exact
/// code/test object IDs, including newly added bindings to unchanged files.
fn entity_digests(
    model: &TelosModel,
    selector: &str,
    snapshot: &Snapshot,
) -> Result<(String, String), TelosError> {
    let mut definitions = BTreeMap::new();
    for (id, value) in &model.contexts {
        definitions.insert(
            id.to_string(),
            serde_json::to_value(value).expect("context"),
        );
    }
    for (id, value) in &model.capabilities {
        definitions.insert(
            id.to_string(),
            serde_json::to_value(value).expect("capability"),
        );
    }
    for (id, value) in &model.domain_notions {
        definitions.insert(id.to_string(), serde_json::to_value(value).expect("notion"));
    }
    for (id, value) in &model.notions {
        definitions
            .entry(id.to_string())
            .or_insert_with(|| serde_json::to_value(value).expect("notion"));
    }
    for (id, value) in &model.intents {
        definitions.insert(id.to_string(), serde_json::to_value(value).expect("intent"));
        for scenario in &value.scenarios {
            definitions.insert(
                scenario.id.to_string(),
                serde_json::to_value(scenario).expect("scenario"),
            );
        }
    }
    for (id, value) in &model.constraints {
        definitions.insert(
            id.to_string(),
            serde_json::to_value(value).expect("constraint"),
        );
    }
    let mut bindings: Vec<_> = model
        .bindings
        .iter()
        .filter(|binding| match binding {
            crate::model::Binding::Implements { intent, .. } => intent.node.to_string() == selector,
            crate::model::Binding::Proves { scenario, .. } => scenario.node.to_string() == selector,
        })
        .map(|binding| json!({"binding":binding,"file":snapshot.get(binding.code_path().as_str())}))
        .collect();
    bindings.sort_by_key(|v| v.to_string());
    Ok((
        digest(
            definitions
                .get(selector)
                .ok_or_else(|| invalid("missing semantic entity"))?,
        )?,
        digest(&bindings)?,
    ))
}

pub fn entity_paths(model: &TelosModel) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for (path, source) in &model.sources {
        let selector = match source {
            SourceKind::Context(id) => id.to_string(),
            SourceKind::Capability(id) => id.to_string(),
            SourceKind::QualifiedNotion(id) => id.to_string(),
            SourceKind::Notion(id) => id.to_string(),
            SourceKind::Intent(id) => id.to_string(),
            SourceKind::Constraint(id) => id.to_string(),
            SourceKind::ContextMap | SourceKind::Bindings => continue,
        };
        result.insert(selector, path.to_string());
        if let SourceKind::Intent(id) = source
            && let Some(intent) = model.intents.get(id)
        {
            for scenario in &intent.scenarios {
                result.insert(scenario.id.to_string(), path.to_string());
            }
        }
    }
    result
}

pub fn history(root: &Path, target: Option<&str>) -> Result<Vec<Receipt>, TelosError> {
    verify(root)?;
    let receipts = receipts(root)?;
    let ledger = read(root)?;
    let uid = target.and_then(|target| {
        ledger
            .identities
            .get(target)
            .map(|i| i.uid.clone())
            .or_else(|| {
                receipts
                    .iter()
                    .rev()
                    .flat_map(|r| r.entities.iter().rev())
                    .find(|e| {
                        e.selector == target
                            || e.uid == target
                            || e.previous_selector.as_deref() == Some(target)
                    })
                    .map(|e| e.uid.clone())
            })
    });
    Ok(receipts
        .into_iter()
        .filter(|r| {
            target.is_none_or(|t| {
                r.plan == t
                    || r.id == t
                    || r.files.iter().chain(&r.observed_files).any(|f| f.path == t)
                    || r.entities.iter().any(|e| Some(&e.uid) == uid.as_ref())
            })
        })
        .collect())
}

/// Import a branch's immutable work records after its code has been merged.
/// Existing histories must be prefixes, never arbitrarily selected conflict
/// resolutions. The integration receipt subsequently records the merged bytes.
pub fn integrate(
    root: &Path,
    plan_id: &str,
    source: &str,
    request_id: &str,
    expected: Option<u64>,
) -> Result<serde_json::Value, TelosError> {
    inventory::at_commit(root, source)?;
    store::update(
        root,
        plan_id,
        request_id,
        expected,
        "plan.integrate",
        &json!({"source":source}),
        |plan| {
            store::require_approved(plan)?;
            let task_id = plan
                .view()?
                .current_task
                .ok_or_else(|| invalid("start the integration task first"))?;
            if plan.task(&task_id)?.kind != TaskKind::Integration {
                return Err(invalid(
                    "importing branch history requires an integration task",
                ));
            }
            let mut current = read(root)?;
            let local_plans = store::list(root)?
                .into_iter()
                .map(|p| (p.id.clone(), p))
                .collect();
            verify_records(&current, &receipts(root)?, &local_plans)?;
            let foreign = decode(&inventory::git_output(
                root,
                &["show", &format!("{source}:{PATH}")],
            )?)?;
            if current.baseline != foreign.baseline
                || current.initial_identities != foreign.initial_identities
                || current.bootstrap_plan != foreign.bootstrap_plan
            {
                return Err(invalid(
                    "the histories do not share the same observed repository baseline",
                ));
            }
            if !foreign.pending_parents.is_empty() {
                return Err(invalid("the source branch has an unfinished integration"));
            }
            let mut plans: BTreeMap<_, _> = store::list(root)?
                .into_iter()
                .map(|p| (p.id.clone(), p))
                .collect();
            let names = inventory::git_output(
                root,
                &[
                    "ls-tree",
                    "-r",
                    "--name-only",
                    "-z",
                    source,
                    "--",
                    "telos/plans",
                ],
            )?;
            let mut writes = Vec::new();
            for name in names.split(|b| *b == 0).filter(|b| !b.is_empty()) {
                let name = std::str::from_utf8(name).map_err(|e| invalid(e.to_string()))?;
                let Some(id) = name
                    .strip_prefix("telos/plans/")
                    .and_then(|n| n.strip_suffix(".tel"))
                else {
                    continue;
                };
                let bytes = inventory::git_output(root, &["show", &format!("{source}:{name}")])?;
                let incoming = store::decode(id, &bytes)?;
                let use_incoming = if let Some(local) = plans.get(id) {
                    if local.events.starts_with(&incoming.events)
                        && local.revisions.starts_with(&incoming.revisions)
                    {
                        false
                    } else if incoming.events.starts_with(&local.events)
                        && incoming.revisions.starts_with(&local.revisions)
                    {
                        true
                    } else {
                        return Err(invalid(format!(
                            "plan `{id}` has divergent progress; resolve it explicitly before integration"
                        )));
                    }
                } else {
                    true
                };
                if use_incoming {
                    if id == plan_id {
                        return Err(invalid(
                            "the source cannot advance the active integration plan",
                        ));
                    }
                    writes.push((store::path(id)?, Some(bytes)));
                    plans.insert(id.into(), incoming);
                }
            }
            let mut foreign_receipts = Vec::new();
            let mut all = receipts(root)?;
            for id in &foreign.receipts {
                let path = receipt_path(id)?;
                let bytes =
                    inventory::git_output(root, &["show", &format!("{source}:{}", path.as_str())])?;
                let (header, receipt): (_, Receipt) = record::parse(
                    "receipt",
                    std::str::from_utf8(&bytes).map_err(|e| invalid(e.to_string()))?,
                )?;
                if &header != id || &receipt.id != id {
                    return Err(invalid("source receipt identity mismatch"));
                }
                foreign_receipts.push(receipt.clone());
                if let Some(local) = all.iter().find(|r| &r.id == id) {
                    if local != &receipt {
                        return Err(invalid(format!("receipt `{id}` differs between branches")));
                    }
                } else {
                    current.receipts.push(id.clone());
                    all.push(receipt);
                    writes.push((path, Some(bytes)));
                }
            }
            verify_records(&foreign, &foreign_receipts, &plans)?;
            if foreign.frontier != current.frontier
                && !foreign.frontier.is_empty()
                && !current.pending_parents.contains(&foreign.frontier)
            {
                current.pending_parents.push(foreign.frontier.clone());
            }
            if current.pending_parents.contains(&foreign.frontier) {
                current.incoming_identities.extend(foreign.identities);
            }
            verify_records(&current, &all, &plans)?;
            writes.push((
                RepoPath::new(PATH),
                Some(record::emit("ledger", "repository", &current)?.into_bytes()),
            ));
            Ok(store::Update {
                kind: EventKind::Integrated,
                task: Some(task_id),
                data: json!({"source":source,"parents":current.pending_parents}),
                writes,
            })
        },
    )
}

/// CI supplies a trusted base SHA. Historical records must be exact prefixes;
/// rewriting a ledger, approval, or receipt in the proposed tree is rejected.
pub fn check_base(root: &Path, base: &str) -> Result<(), TelosError> {
    require_clean(root)?;
    let baseline = inventory::at_commit(root, base)?;
    let bytes = inventory::git_output(root, &["show", &format!("{base}:{PATH}")])?;
    let old = decode(&bytes)?;
    let current = read(root)?;
    if old.baseline != current.baseline
        || old.initial_identities != current.initial_identities
        || old.bootstrap_plan != current.bootstrap_plan
        || old.observed_at != current.observed_at
        || !current.receipts.starts_with(&old.receipts)
        || old.current != baseline
    {
        return Err(invalid(
            "the trusted base inventory or history was rewritten",
        ));
    }
    let names = inventory::git_output(
        root,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            "-z",
            base,
            "--",
            "telos/plans",
            "telos/history",
        ],
    )?;
    let fs = RepoFs::open(root)?;
    for raw in names.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let path = std::str::from_utf8(raw).map_err(|e| invalid(e.to_string()))?;
        let old = inventory::git_output(root, &["show", &format!("{base}:{path}")])?;
        let new = fs.read(&RepoPath::parse(path)?)?;
        if path.starts_with("telos/history/") {
            if old != new {
                return Err(invalid(format!(
                    "historical receipt `{path}` was rewritten"
                )));
            }
        } else if let Some(id) = path
            .strip_prefix("telos/plans/")
            .and_then(|s| s.strip_suffix(".tel"))
        {
            let old = store::decode(id, &old)?;
            let new = store::decode(id, &new)?;
            if !new.revisions.starts_with(&old.revisions)
                || !new.events.starts_with(&old.events)
                || old.created_at != new.created_at
            {
                return Err(invalid(format!("plan history `{id}` was rewritten")));
            }
        }
    }
    Ok(())
}

fn unplanned(files: &[FileChange]) -> TelosError {
    TelosError::new(
        ErrorCode::TelosUnplannedChange,
        format!(
            "unplanned repository changes: {}",
            files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
    .hint("restore unrelated changes or approve a plan revision covering these paths")
}
fn invalid(message: impl Into<String>) -> TelosError {
    TelosError::new(ErrorCode::TelosHistoryConflict, message)
}
