//! `telos bind <path> <INT-id>`: record that a code file implements an
//! intent, journalled into the change that owns the intent (D5, D6).
//!
//! The command is *mutating* -- it appends a `bind` line to a change file --
//! so, like `telos test` (T3), it runs its gates in a frozen order and
//! writes nothing until every one of them has passed:
//!
//! 1. the preamble ([`project`]: workspace, lock, repository, one store
//!    scan, state);
//! 2. the intent argument, parsed and resolved against every intent id the
//!    spec or any open change declares;
//! 3. the path argument, parsed as a repo-relative path that does not name
//!    anything under `telos/`;
//! 4. the **owner** (D5): the open change whose overlay claims the intent --
//!    a binding belongs to the transaction that introduced what it
//!    implements, never to the project at large;
//! 5. that owner's status: `approved` or `implementing`, never a delta
//!    nobody has reviewed;
//! 6. the bound path must exist, at the bytes the working tree holds right
//!    now;
//! 7. the **drift gate with its carve-out** (D6) -- see
//!    [`require_no_foreign_drift`];
//! 8. the journal line, and the `approved` → `implementing` transition,
//!    written together -- or nothing at all, when an identical line is
//!    already there (idempotence).
//!
//! Two rulings are worth spelling out, because neither is visible in the
//! code that implements them:
//!
//! - **`TELOS_FILE_CLAIMED` does not apply to journal writes.** Exactly the
//!   ruling T3 makes for `telos test`, and for the same reason: a journal
//!   claim exists to make the drift of the bound file admissible (D3/D6),
//!   not to lock the file against other changes. So nothing here calls
//!   `require_unclaimed`.
//! - **A path under `telos/` is refused with the grammar's own wording.**
//!   `parse_change_file` would reject a hand-written `bind` line naming one
//!   anyway (`code_path_outside_the_spec_tree`) -- `telos/bindings.tel`
//!   above all, since D2 rewrites it wholesale at every reconcile and D3's
//!   claim guarantee depends on no journal line ever naming it. Answering
//!   with the identical message here, before anything is written, means a
//!   caller sees one wording for one rule regardless of which layer catches
//!   the mistake.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use telos_core::changes::write_change;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::{ChangeId, IntentId, RepoPath};
use telos_core::model::{Change, ChangeStatus, JournalEntry, StagedOp, TelFile, intent_path};
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

    let owner = owner_of(&project, intent).ok_or_else(|| no_owner(intent))?;
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
/// post-state an op carries whole (Annex C), applied in staged order so a
/// change that adds then removes the same intent within its own delta
/// leaves it unknown, as it should.
fn known_intents(project: &Project) -> Result<BTreeSet<IntentId>, TelosError> {
    let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;

    let mut known: BTreeSet<IntentId> = BTreeSet::new();
    for (_, file) in &base {
        if let TelFile::Intent(intent) = file {
            known.insert(intent.id);
        }
    }
    for change in &project.parsed {
        for op in &change.ops {
            match op {
                StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => {
                    known.insert(i.id);
                }
                StagedOp::RemoveIntent(id) => {
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
    if !is_repo_relative(arg) {
        return Err(TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("cannot parse `{arg}` as a repository-relative path"),
        ));
    }
    if arg.starts_with("telos/") {
        // The grammar refuses this too, and for the same reason (D2, D3,
        // D9) -- see the module doc's second ruling. Same message, whether
        // it is this check or `parse_change_file` reading the line back
        // afterwards that catches it.
        return Err(TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            "a journal line cannot name a path under telos/",
        ));
    }
    Ok(RepoPath::new(arg))
}

/// Whether `arg` is usable as a repo-relative path (see
/// [`parse_repo_path`]): not empty, not absolute, and free of any `..`
/// component.
fn is_repo_relative(arg: &str) -> bool {
    if arg.is_empty() || arg.starts_with('/') {
        return false;
    }
    let path = std::path::Path::new(arg);
    if path.is_absolute() {
        return false;
    }
    !path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// D5's ownership rule: the open change whose overlay claims the intent.
///
/// An `add`/`edit intent` op's target path is `intent_path(id)` (Annex C),
/// and that path is exactly what becomes a claim of the change that stages
/// it (`Change::claims`) -- so "the change whose ops add or edit this
/// intent" and "the change whose claims contain `intent_path(intent)`" are
/// one and the same set, and testing the claim is enough. `Project::parsed`
/// is ascending by id, so two changes that somehow both claimed the same
/// intent (impossible in practice: staging's one-file-one-change gate would
/// have refused the second) resolve to the lower one, deterministically.
fn owner_of(project: &Project, intent: IntentId) -> Option<&Change> {
    let path = intent_path(intent);
    project
        .parsed
        .iter()
        .find(|change| change.claims().contains(&path))
}

/// The frozen wording for an intent no open change claims (Annex F, the
/// same shape `telos test` answers with for an unowned scenario).
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
/// word for word (M2, Annex F): the two are the same integrity problem, a
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

/// Journals `path -> intent` into `change` (D1, D5) -- or, when an identical
/// line is already there, writes nothing and answers with the same result
/// anyway.
///
/// That dedup is `bind`'s own addition on top of `telos test`'s journalling
/// shape: a rerun of the exact same command -- a retried script, two agents
/// racing -- must be idempotent (Annex C), so the check comes first and the
/// append happens only on a genuine miss. `Change::journal_bindings` folds
/// duplicate lines too (D2), but only at reconcile; this is what keeps the
/// journal itself, the append-only evidence both commands write, from
/// growing a line per identical retry in the meantime.
///
/// The `approved` → `implementing` transition rides along with the write,
/// exactly as it does for a run (D5): the grammar requires a journalled
/// change to be `implementing`, so the two can never be written apart.
/// Nothing moves when the dedup finds nothing new -- including the status,
/// which by then is already whatever the first bind left it as -- and the
/// frozen approval digest is untouched either way (the journal is
/// digest-inert, D1).
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

/// The bind's result object (Annex C).
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
