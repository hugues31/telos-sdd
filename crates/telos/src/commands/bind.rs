//! `telos bind <path> <INT-id>`: record that a code file implements an
//! intent, journalled into the change that owns the intent.
//!
//! The command is *mutating* -- it appends a `bind` line to a change file --
//! so, like `telos test`, it runs its gates in a frozen order and
//! writes nothing until every one of them has passed:
//!
//! 1. the preamble ([`project`]: workspace, lock, repository, one store
//!    scan, state);
//! 2. the intent argument, parsed and resolved against every intent id the
//!    spec or any open change declares;
//! 3. the path argument, parsed as a repo-relative path that does not name
//!    anything under `telos/`;
//! 4. the **owner**: the open change whose delta adds or edits the
//!    intent -- a binding belongs to the transaction that introduced what
//!    it implements, never to the project at large, and never to a
//!    transaction that is *removing* it (see [`owner_of`]);
//! 5. that owner's status: `approved` or `implementing`, never a delta
//!    nobody has reviewed;
//! 6. the bound path must exist, at the bytes the working tree holds right
//!    now;
//! 7. the **drift gate with its carve-out** -- see
//!    [`require_no_foreign_drift`];
//! 8. the journal line, and the `approved` → `implementing` transition,
//!    written together -- or nothing at all, when an identical line is
//!    already there (idempotence).
//!
//! Two rulings are worth spelling out, because neither is visible in the
//! code that implements them:
//!
//! - **`TELOS_FILE_CLAIMED` does not apply to journal writes.** Exactly the
//!   ruling used by `telos test`, and for the same reason: a journal
//!   claim exists to make the drift of the bound file admissible,
//!   not to lock the file against other changes. So nothing here calls
//!   `require_unclaimed`.
//! - **A path under `telos/` is refused with the grammar's own wording.**
//!   `parse_change_file` would reject a hand-written `bind` line naming one
//!   anyway (`code_path_outside_the_spec_tree`) -- a context `bindings.tel`
//!   above all, since every reconcile rewrites it wholesale and journal
//!   paths count as change claims. Answering
//!   with the identical message here, before anything is written, means a
//!   caller sees one wording for one rule regardless of which layer catches
//!   the mistake.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use telos_core::changes::write_change;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::{ChangeId, IntentId, RepoPath};
use telos_core::model::{Change, ChangeStatus, JournalEntry, StagedOp, TelFile};
use telos_core::overlay::parse_base;

use crate::commands::{
    Ctx, Project, diagnostics_to_error, nearest_id, project, require_approved,
    require_no_foreign_drift, unknown,
};
use crate::envelope::{CmdResult, Outcome};

/// `telos bind <path> <INT-id>`, the eight steps of the module doc, once.
pub fn run(ctx: &Ctx, path: &str, intent: &str) -> CmdResult {
    let project = project(ctx)?;

    let intent = parse_intent_id(intent)?;
    require_known(&project, intent)?;
    let path = parse_repo_path(path)?;

    let owner = owner_of(&project.parsed, intent).ok_or_else(|| no_owner(intent))?;
    require_approved(owner)?;

    require_exists(&project, &path)?;
    require_no_foreign_drift(&project, std::slice::from_ref(&path))?;

    let mut change = owner.clone();
    let report = journal_bind(&project, &mut change, path, intent)?;

    Ok(Outcome {
        result: bind_result(&report),
        human: human_line(&report),
        next_actions: vec![format!("telos change reconcile {}", report.change)],
    })
}

// --- the gates ---------------------------------------------------------------

/// An `INT-NNNN` argument, or `TELOS_REFERENCE_UNKNOWN` naming what was
/// expected -- the same policy as every other typed id argument (`show`,
/// `change abandon`, the staging verbs, `telos test`'s scenario argument).
fn parse_intent_id(arg: &str) -> Result<IntentId, TelosError> {
    arg.parse::<IntentId>().map_err(|_| {
        TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("cannot parse `{arg}` as an intent id"),
        )
    })
}

/// Refuses an intent id nothing declares, with the nearest one that is.
///
/// "Declared" spans both worlds an intent can live in, exactly as
/// `telos test`'s scenario lookup does: the spec files on disk, and the
/// deltas of the open changes -- an intent `add intent` allocated a moment
/// ago is precisely the one `telos bind` is about to be pointed at.
fn require_known(project: &Project, intent: IntentId) -> Result<(), TelosError> {
    let known = known_intents(project)?;
    if known.contains(&intent) {
        return Ok(());
    }
    Err(unknown(
        "intent",
        intent,
        nearest_id(intent.0, known.iter().map(|id| id.0), |n| {
            IntentId(n).to_string()
        }),
    ))
}

/// Every intent id the sealed spec or any open change's overlay declares.
///
/// `add`/`edit intent` insert the id, `remove intent` withdraws it -- the
/// post-state an op carries whole, applied in staged order so a
/// change that adds then removes the same intent within its own delta
/// leaves it unknown, as it should.
fn known_intents(project: &Project) -> Result<BTreeSet<IntentId>, TelosError> {
    let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;

    let mut known: BTreeSet<IntentId> = BTreeSet::new();
    for (_, file) in &base {
        match file {
            TelFile::OwnedIntent { intent, .. } | TelFile::Intent(intent) => {
                known.insert(intent.id);
            }
            _ => {}
        }
    }
    for change in &project.parsed {
        for op in &change.ops {
            match op {
                StagedOp::AddOwnedIntent { intent: i, .. }
                | StagedOp::EditOwnedIntent { intent: i, .. }
                | StagedOp::MoveIntent { intent: i, .. }
                | StagedOp::AddIntent(i)
                | StagedOp::EditIntent(i) => {
                    known.insert(i.id);
                }
                StagedOp::RemoveOwnedIntent { id, .. } | StagedOp::RemoveIntent(id) => {
                    known.remove(id);
                }
                _ => {}
            }
        }
    }
    Ok(known)
}

/// A repo-relative path argument: relative (no leading `/`, not absolute on
/// any platform), with no `..` component escaping the tree, and not naming
/// anything under `telos/`.
fn parse_repo_path(arg: &str) -> Result<RepoPath, TelosError> {
    RepoPath::parse_outside_telos(arg).map_err(|error| {
        if error.message == "a journal line cannot name a path under telos/" {
            return error;
        }
        TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("cannot parse `{arg}` as a repository-relative path"),
        )
    })
}

/// The open change whose delta adds or edits the
/// intent -- the same predicate `telos test`'s `owner_of` applies to a
/// scenario, one level up. Only `AddIntent`/`EditIntent` can make a change
/// the intent's *implementer*: `Change::claims()` maps `target_path` over
/// every op including `RemoveIntent` (and `intent_path(id)` is that op's
/// target path too), so testing `claims()` instead would hand ownership of
/// an intent to the one change that is *deleting* it -- exactly backwards.
///
/// Takes the parsed changes directly, rather than a whole [`Project`], so it
/// is a pure function of the data it actually reads -- and so the
/// `RemoveIntent` regression below can construct a `Vec<Change>` and call it
/// without also having to fabricate a workspace, a lock and a git
/// repository. Ascending by id (the order [`Project::parsed`] is already
/// in), so two changes that somehow both staged the same intent (impossible
/// in practice: staging's one-file-one-change gate would have refused the
/// second) resolve to the lower one, deterministically.
fn owner_of(changes: &[Change], intent: IntentId) -> Option<&Change> {
    changes.iter().find(|change| {
        change.ops.iter().any(|op| match op {
            StagedOp::AddOwnedIntent { intent: i, .. }
            | StagedOp::EditOwnedIntent { intent: i, .. }
            | StagedOp::MoveIntent { intent: i, .. }
            | StagedOp::AddIntent(i)
            | StagedOp::EditIntent(i) => i.id == intent,
            _ => false,
        })
    })
}

/// The stable error for an intent no open change claims; `telos test` uses
/// the same shape for an unowned scenario.
fn no_owner(intent: IntentId) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("no open change is implementing {intent}"),
    )
    .hint("stage it into a change and approve it first")
}

/// The bound path must exist, at the bytes the working tree holds right
/// now.
///
/// `blob_oids` silently drops a path `std::fs::metadata` cannot see (the
/// same filter `seal` and `telos test`'s oid lookup both rely on), so a
/// missing file is exactly the paths that come back empty -- no separate
/// existence check duplicates that logic. The message is `seal`'s own,
/// word for word: the two are the same integrity problem, a
/// binding naming a file that is not there, whichever pass discovers it.
fn require_exists(project: &Project, path: &RepoPath) -> Result<(), TelosError> {
    let oids = project.git.blob_oids(std::slice::from_ref(path))?;
    if oids.contains_key(path) {
        return Ok(());
    }
    Err(TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("binding references `{path}`, which does not exist"),
    ))
}

// --- journalling ---------------------------------------------------------------

/// What one recorded bind reports, in the result and the human line.
struct BindReport {
    path: RepoPath,
    intent: IntentId,
    change: ChangeId,
}

/// Journals `path -> intent` into `change`; when an identical line is already
/// there, writes nothing and answers with the same result anyway.
///
/// That dedup is `bind`'s own addition on top of `telos test`'s journalling
/// shape: a rerun of the exact same command -- a retried script, two agents
/// racing -- must be idempotent, so the check comes first and the
/// append happens only on a genuine miss. `Change::journal_bindings` folds
/// duplicate lines too, but only at reconcile; this is what keeps the
/// journal itself, the append-only evidence both commands write, from
/// growing a line per identical retry in the meantime.
///
/// The `approved` → `implementing` transition rides along with the write,
/// exactly as it does for a run: the grammar requires a journalled
/// change to be `implementing`, so the two can never be written apart.
/// Nothing moves when the dedup finds nothing new -- including the status,
/// which by then is already whatever the first bind left it as -- and the
/// frozen approval digest is untouched either way because journal entries
/// are digest-inert.
fn journal_bind(
    project: &Project,
    change: &mut Change,
    path: RepoPath,
    intent: IntentId,
) -> Result<BindReport, TelosError> {
    let already_bound = change.journal.iter().any(|entry| {
        matches!(
            entry,
            JournalEntry::Bind { path: p, intent: i } if *p == path && *i == intent
        )
    });

    if !already_bound {
        change.journal.push(JournalEntry::Bind {
            path: path.clone(),
            intent,
        });
        if change.status == ChangeStatus::Approved {
            change.status = ChangeStatus::Implementing;
        }
        write_change(&project.ws, change)?;
    }

    Ok(BindReport {
        path,
        intent,
        change: change.id,
    })
}

/// The bind's result object.
fn bind_result(report: &BindReport) -> Value {
    json!({
        "change": report.change,
        "path": report.path,
        "intent": report.intent,
    })
}

fn human_line(report: &BindReport) -> String {
    format!(
        "{} implements {} (recorded in {})",
        report.path, report.intent, report.change
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change_with(ops: Vec<StagedOp>) -> Change {
        Change {
            id: ChangeId(1),
            motivation: "x".to_string(),
            status: ChangeStatus::Approved,
            approved_digest: None,
            ops,
            journal: Vec::new(),
        }
    }

    /// The regression the review caught: `Change::claims()` maps
    /// `target_path` over *every* op, `RemoveIntent` included, so an
    /// ownership check built on `claims()` would treat the one change
    /// *deleting* an intent as the change *implementing* it. This asserts
    /// the fix directly against `owner_of`, independent of `require_known`
    /// (which happens to withdraw a removed id from the known set today,
    /// but is a separate gate that could change without this one noticing
    /// -- see `bind_to_an_intent_a_change_only_removes_is_unknown_not_owned`
    /// in `tests/test_bind.rs` for that layer, documented as the current,
    /// coincidental one).
    ///
    /// A second, unrelated `RemoveIntent` shares the change too, so the
    /// assertion cannot pass by accident of the change having only one op.
    #[test]
    fn owner_of_never_selects_a_change_that_only_removes_the_intent() {
        let intent = IntentId(17);
        let changes = vec![change_with(vec![
            StagedOp::RemoveIntent(intent),
            StagedOp::RemoveIntent(IntentId(42)),
        ])];

        assert!(owner_of(&changes, intent).is_none());
    }
}
