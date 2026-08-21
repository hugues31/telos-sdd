//! `telos add|edit|remove <notion|intent|constraint>`: staging one operation
//! into an open change.
//!
//! The three verbs are one command with three shapes. Each of them appends
//! exactly one [`StagedOp`] to a change and writes nothing else -- no `.tel`
//! file of the spec is touched until `reconcile`. What makes that safe
//! is the order the flow below is written in, which is the whole design of
//! this module:
//!
//! 1. **State first.** [`project`] discovers, locks and computes state;
//!    [`require_no_unclaimed_drift`] refuses to stage on top of a base
//!    nobody reviewed.
//! 2. **The change must exist**, and is read through the store.
//! 3. **The op is built against the overlay**, not against the sealed tree:
//!    the base is the sealed spec with this change's own earlier ops already
//!    applied, so `add intent` can name a notion the same change added two
//!    ops ago, and `edit` patches the entity as this change last left it.
//! 4. **One file, one change**: the op's target path must not be
//!    claimed by another open change.
//! 5. **The new op is checked against the overlay** ([`apply_ops`] --
//!    referential deletion safety and whether the target exists at all), **then the whole spec
//!    the delta describes is validated** ([`validate_ops_idempotent`]) --
//!    every reference, every literal.
//! 6. **Only then does anything reach the disk**, change file first,
//!    `counters.toml` second.
//!
//! Step 6 is why steps 1-5 allocate ids freely: an id burnt by a payload
//! that turns out to be invalid is never persisted, because
//! [`write_counters`] runs *after* [`write_change`] and both run only on the
//! success path. A refused mutation leaves the change byte-for-byte as it
//! was.
//!
//! **No status gate.** Staging is allowed on `open` (which becomes
//! `drafted`), on `drafted`, and on an already-approved change too. The last
//! case is deliberate: nothing is lost -- the approval's digest stays,
//! [`Change::is_stale`] turns true, `change diff` reports `stale: true`, and
//! `reconcile` refuses with `TELOS_APPROVAL_STALE`. Refusing here
//! instead would only move the same conversation earlier while making the
//! natural "review, adjust, re-approve" loop impossible.

use clap::ValueEnum;
use serde_json::{Value, json};

use telos_core::changes::{read_change, write_change};
use telos_core::counters::{Alloc, write_counters};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::{ChangeId, ConstraintId, IntentId, NotionName, RepoPath, ScenarioId};
use telos_core::model::{
    Change, ChangeStatus, Constraint, Intent, Notion, StagedOp, TelFile, constraint_path,
    intent_path, notion_path,
};
use telos_core::overlay::{
    apply_ops, apply_ops_idempotent, find_file, notions_of, parse_base, unknown_entity,
    validate_ops_idempotent,
};
use telos_core::payload::{
    constraint_from_json, intent_from_json, notion_from_json, patch_constraint, patch_intent,
    patch_notion,
};

use crate::commands::change::parse_change_id;
use crate::commands::{
    Ctx, Project, allocator, diagnostics_to_error, project, require_no_unclaimed_drift,
};
use crate::envelope::{CmdResult, Outcome};

/// Which kind of entity a staging verb acts on. The three words are the
/// command's own vocabulary (`telos add notion`), and the same three
/// [`StagedOp::entity`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EntityKind {
    Notion,
    Intent,
    Constraint,
}

// --- the three verbs --------------------------------------------------------

/// `telos add <kind> --change CHG-NNNN`, payload on stdin.
///
/// An `add` payload never carries an id: `intent` and `constraint`
/// get theirs from the allocator, and a notion's identity is its `name`.
pub fn add(ctx: &Ctx, kind: EntityKind, change: &str, payload: &str) -> CmdResult {
    let mut staging = Staging::begin(ctx, change)?;
    let payload = payload_json(payload)?;

    let (op, scenario_ids) = match kind {
        EntityKind::Notion => (StagedOp::AddNotion(notion_from_json(&payload)?), Vec::new()),
        EntityKind::Intent => {
            let notions = notions_of(&staging.base);
            let (intent, ids) = intent_from_json(&payload, &notions, staging.alloc()?)?;
            (StagedOp::AddIntent(intent), ids)
        }
        EntityKind::Constraint => (
            StagedOp::AddConstraint(constraint_from_json(&payload, staging.alloc()?)?),
            Vec::new(),
        ),
    };

    staging.finish(op, Some(scenario_ids))
}

/// `telos edit <kind> <key> --change CHG-NNNN`, payload on stdin.
///
/// The op carries the complete post-state, not the patch, so the
/// entity is read from the overlay, patched in memory, and staged whole.
pub fn edit(ctx: &Ctx, kind: EntityKind, key: &str, change: &str, payload: &str) -> CmdResult {
    let mut staging = Staging::begin(ctx, change)?;
    let payload = payload_json(payload)?;

    let (op, scenario_ids) = match kind {
        EntityKind::Notion => {
            let name = parse_notion_name(key)?;
            let base = staging.notion(&name)?.clone();
            let patched = patch_notion(&base, &payload)?;
            if patched.name != name {
                return Err(rename_refused("notion", key, patched.name.as_str()));
            }
            (StagedOp::EditNotion(patched), Vec::new())
        }
        EntityKind::Intent => {
            let id = parse_id::<IntentId>(key, "an intent id")?;
            let base = staging.intent(id)?.clone();
            let notions = notions_of(&staging.base);
            let (intent, ids) = patch_intent(&base, &payload, &notions, staging.alloc()?)?;
            (StagedOp::EditIntent(intent), ids)
        }
        EntityKind::Constraint => {
            let id = parse_id::<ConstraintId>(key, "a constraint id")?;
            let base = staging.constraint(id)?.clone();
            (
                StagedOp::EditConstraint(patch_constraint(&base, &payload)?),
                Vec::new(),
            )
        }
    };

    staging.finish(op, Some(scenario_ids))
}

/// `telos remove <kind> <key> --change CHG-NNNN`. No payload.
///
/// Whether the target exists, and whether anything still references it
/// (the referential-deletion check), are both the overlay's answers -- staged here, decided
/// by [`apply_ops`] in [`Staging::finish`], which is where the removal meets
/// the spec it would leave behind.
pub fn remove(ctx: &Ctx, kind: EntityKind, key: &str, change: &str) -> CmdResult {
    let staging = Staging::begin(ctx, change)?;

    let op = match kind {
        EntityKind::Notion => StagedOp::RemoveNotion(parse_notion_name(key)?),
        EntityKind::Intent => StagedOp::RemoveIntent(parse_id::<IntentId>(key, "an intent id")?),
        EntityKind::Constraint => {
            StagedOp::RemoveConstraint(parse_id::<ConstraintId>(key, "a constraint id")?)
        }
    };

    staging.finish(op, None)
}

// --- the shared flow --------------------------------------------------------

/// Everything the three verbs share: the project, the change being staged
/// into, the overlay that change already describes, and -- for the ops that
/// need one -- the allocator.
struct Staging {
    project: Project,
    change: Change,
    /// The sealed spec with `change`'s ops already applied -- what a new op
    /// is built and validated against.
    base: Vec<(RepoPath, TelFile)>,
    /// Built on first use by [`Staging::alloc`], never eagerly -- see there.
    alloc: Option<Alloc>,
}

impl Staging {
    /// Steps 1-3 of the module's flow, in that order: state, then the
    /// change, then the overlay it describes.
    fn begin(ctx: &Ctx, change: &str) -> Result<Staging, TelosError> {
        // The argument is validated before anything is discovered, the same
        // order `change abandon` uses: a malformed id is the caller's
        // mistake, and saying so does not require a workspace.
        let id = parse_change_id(change)?;
        let project = project(ctx)?;
        require_no_unclaimed_drift(&project)?;

        let change = read_change(&project.ws, id)?;
        let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;
        // The change's *own* earlier ops are replayed idempotently: a change
        // `adopt` produced describes a tree that already shows them,
        // and refusing to build a base for the next op because of that would
        // make an adopted change the one kind nobody can add to.
        let base = apply_ops_idempotent(base, &change.ops);

        Ok(Staging {
            project,
            change,
            base,
            alloc: None,
        })
    }

    /// The id allocator, built on the first call and reused after.
    ///
    /// Lazy because [`allocator`] is a *gate*, not just a computation: it
    /// calls `load_model` on the sealed tree, so it refuses whenever the
    /// sealed spec does not resolve on its own -- which is stricter than
    /// anything staging otherwise requires (the overlay only needs the base
    /// to *parse*, and only the post-state to resolve). Only four of the
    /// nine verb/kind combinations mint an id -- `add intent`,
    /// `add constraint`, and the `edit intent` that grows a scenario --
    /// so making the other five pay that gate would refuse, for instance,
    /// the `remove` that was about to close the very hole the model has.
    fn alloc(&mut self) -> Result<&mut Alloc, TelosError> {
        if self.alloc.is_none() {
            self.alloc = Some(allocator(&self.project.ws, &self.project.lock)?);
        }
        Ok(self.alloc.as_mut().expect("just built above"))
    }

    fn notion(&self, name: &NotionName) -> Result<&Notion, TelosError> {
        match find_file(&self.base, &notion_path(name)) {
            Some(TelFile::Notion(notion)) => Ok(notion),
            _ => Err(unknown_entity(&self.base, "notion", name.as_str())),
        }
    }

    fn intent(&self, id: IntentId) -> Result<&Intent, TelosError> {
        match find_file(&self.base, &intent_path(id)) {
            Some(TelFile::Intent(intent)) => Ok(intent),
            _ => Err(unknown_entity(&self.base, "intent", &id.to_string())),
        }
    }

    fn constraint(&self, id: ConstraintId) -> Result<&Constraint, TelosError> {
        match find_file(&self.base, &constraint_path(id)) {
            Some(TelFile::Constraint(constraint)) => Ok(constraint),
            _ => Err(unknown_entity(&self.base, "constraint", &id.to_string())),
        }
    }

    /// Steps 4-6: the claim gate, the full validation, then the two writes.
    ///
    /// `scenario_ids` is `Some` for `add`/`edit` (whose result reports the
    /// ids allocated along the way, `[]` when none were) and `None` for
    /// `remove`, whose result is the shorter shape of the result schema.
    ///
    /// `claims` in the result is the *change's* claim set with the new op
    /// counted, not this op's one path: what a caller needs to know is which
    /// files this change now owns, which is also what a competing `add`
    /// would collide with.
    fn finish(mut self, op: StagedOp, scenario_ids: Option<Vec<ScenarioId>>) -> CmdResult {
        require_unclaimed(&self.project, self.change.id, &op.target_path())?;

        // Step 5 in two halves, because the two halves judge different
        // things. The op's own preconditions -- add of what already exists,
        // edit/remove of what does not, referential deletion safety -- are judged
        // strictly, against the overlay this change already describes: this
        // op is being staged *now*, and its mistakes are the caller's to fix
        // now. The spec as a whole is judged after, over the full delta,
        // idempotently: an op an earlier `adopt` staged describes a tree that
        // already shows it, and re-judging its preconditions here would
        // refuse a change that is perfectly applicable.
        apply_ops(self.base.clone(), std::slice::from_ref(&op))?;

        let entity = op.entity();
        let key = op.key();
        let verb = op.verb();
        self.change.ops.push(op);
        // the change lifecycle: the first staged op is what takes a change out of `open`.
        // Every other status is left exactly as it was -- see the module
        // docs on why staging into an approved change is allowed.
        if self.change.status == ChangeStatus::Open {
            self.change.status = ChangeStatus::Drafted;
        }

        validate_ops_idempotent(&self.project.ws, &self.change.ops)
            .map_err(diagnostics_to_error)?;

        write_change(&self.project.ws, &self.change)?;
        // Only an op that minted an id has a counter to persist; the others
        // never built an allocator, and re-persisting an unchanged
        // `counters.toml` would be a write for nothing. Nothing is lost by
        // skipping it: the next allocation rescans the floors anyway.
        if let Some(alloc) = &self.alloc {
            write_counters(&self.project.ws, &alloc.counters())?;
        }

        let id = self.change.id;
        let mut result = json!({ "change": id, "entity": entity, "id": key });
        if let Some(scenario_ids) = scenario_ids {
            result["scenario_ids"] = json!(scenario_ids);
            result["claims"] = json!(self.change.claims());
        }

        Ok(Outcome {
            result,
            human: format!("{id}: {verb} {entity} {key}"),
            next_actions: vec![format!("telos change diff {id}")],
        })
    }
}

/// Enforces one file, one change.
///
/// The scan is over every *other* open change -- a change may of course
/// stage the same file twice, a claim keeps others out, it is not a lock
/// against its owner. `Project::changes` is ordered by id, so a path two
/// changes somehow claimed names the lower one, deterministically.
///
/// Shared with `adopt`, which stages several ops at once and runs this
/// on each of them: one definition of what a claim collision is, and one
/// message for it.
pub(crate) fn require_unclaimed(
    project: &Project,
    mine: ChangeId,
    path: &RepoPath,
) -> Result<(), TelosError> {
    for info in &project.changes {
        if info.id != mine && info.claims.contains(path) {
            return Err(TelosError::new(
                ErrorCode::TelosFileClaimed,
                format!("{path} is already claimed by {}", info.id),
            )
            .hint(format!(
                "reconcile or abandon {} first, or work within it",
                info.id
            )));
        }
    }
    Ok(())
}

// --- arguments and payload --------------------------------------------------

/// The one frozen message for a payload that is not a JSON object: absent,
/// empty, malformed, or a JSON value of any other kind. They are one
/// mistake from the caller's side -- "nothing usable arrived on stdin" --
/// and telling them apart would only invite an agent to branch on which.
fn payload_json(raw: &str) -> Result<Value, TelosError> {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) if value.is_object() => Ok(value),
        _ => Err(TelosError::new(
            ErrorCode::TelosParseError,
            "payload: expected a JSON object on stdin",
        )),
    }
}

/// A typed id argument, or `TELOS_REFERENCE_UNKNOWN` naming what was
/// expected -- the same policy as `change abandon`'s `--change`.
fn parse_id<T: std::str::FromStr>(key: &str, expected: &str) -> Result<T, TelosError> {
    key.parse::<T>().map_err(|_| {
        TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("cannot parse `{key}` as {expected}"),
        )
    })
}

fn parse_notion_name(key: &str) -> Result<NotionName, TelosError> {
    NotionName::new(key).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("cannot parse `{key}` as a notion name"),
        )
    })
}

/// An `edit` payload that changes the entity's identity is refused rather
/// than silently staged.
///
/// Only a notion can reach this: an intent's and a constraint's id come from
/// the base entity, never from the payload (`patch_intent` /
/// `patch_constraint`), while a notion's identity *is* a payload field. A
/// staged rename would claim the new name's file and leave the old one
/// behind untouched -- two operations wearing one op's clothes.
fn rename_refused(entity: &str, from: &str, to: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("cannot rename {entity} `{from}` to `{to}`"),
    )
    .hint(format!(
        "stage `remove {entity} {from}` and an `add` of the new one instead"
    ))
}
