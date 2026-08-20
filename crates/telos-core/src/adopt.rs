//! The two exits from drift (spec §6, D7): capture it, or throw it away.
//!
//! Drift is the one thing this system treats as illegitimate -- a sealed file
//! edited outside the protocol -- and it is also unavoidable: an editor, a
//! merge, a script, a human in a hurry. Refusing to go forward is only half
//! an answer; the other half is that *nothing is ever lost to a refusal*, and
//! this module is that half.
//!
//! - [`plan_adopt`] reads the drift as a *diff, sealed -> current*, and
//!   expresses it in the same [`StagedOp`]s `telos add|edit|remove` produce.
//!   The bytes are kept; what changes is that they now go through review
//!   (`change diff`, `change approve`) and through the reconcile's gates like
//!   any other delta.
//! - [`revert`] does the opposite: the seal is right, the tree is wrong, so
//!   every sealed path is restored from the blob its OID names and everything
//!   unsealed is deleted.
//!
//! # Why one of them needs a committed repository and the other does not
//!
//! [`plan_adopt`] only ever *hashes* the tree ([`GitRepo::blob_oids`], no
//! `-w`), so it works on a project that was sealed but never committed.
//! [`revert`] restores *content*, and content only exists in the object store
//! once something wrote it there -- a commit, almost always. A seal is
//! therefore not a backup on its own, and the refusal
//! [`GitRepo::cat_blob`] raises says so in as many words.
//!
//! # What `adopt` refuses
//!
//! Three things, and each of them hands the caller a next step rather than
//! guessing at one:
//!
//! - A drifted `.tel` file that no longer parses. There is no entity to stage
//!   -- `TELOS_PARSE_ERROR` with [`ADOPT_PARSE_HINT`].
//! - The deletion of a file that carries no entity: a bound code file, or
//!   `telos/bindings.tel`. An entity file's identity survives its deletion
//!   (it is in the path), so a `remove` op can still be written; an opaque
//!   file's content does not, so there is neither an identity to remove nor
//!   bytes to accept. `revert` is what handles that ([`undeletable`]).
//! - A `.tel` file whose declared entity belongs at another path
//!   ([`require_identity`]): adopting it would stage an op claiming a file
//!   *other* than the drifted one, leaving the drift uncaptured after an
//!   `adopt` that reported success.
//!
//! A fourth refusal is not this module's, but reaches the caller through it
//! all the same: the delta `plan_adopt` returns still has to describe a spec
//! that resolves. Drift that deletes a notion three intents still name is
//! planned without complaint here and refused by
//! [`crate::overlay::validate_ops_idempotent`] at the CLI layer, with the
//! semantic pass' own diagnostics.

use std::fs;

use crate::error::{Diagnostic, ErrorCode, TelosError};
use crate::git::GitRepo;
use crate::ids::{ConstraintId, IntentId, NotionName, RepoPath};
use crate::lock::Lock;
use crate::model::{StagedOp, constraint_path, intent_path, notion_path};
use crate::state::{DriftEntry, DriftKind};
use crate::syntax::{parse_constraint_file, parse_intent_file, parse_notion_file};
use crate::workspace::Workspace;

/// The frozen hint of the `TELOS_PARSE_ERROR` `adopt` raises on a drifted
/// file it cannot read: the two ways out, in the order a caller should try
/// them.
pub const ADOPT_PARSE_HINT: &str = "fix the file or run `telos revert`";

/// What `adopt` would stage: one op per drifted path, and the paths
/// themselves.
///
/// The two vectors are parallel by construction -- every drift entry yields
/// exactly one op -- but they answer different questions, so both are
/// carried: `ops` is what goes into the change, `paths` is what the caller
/// reports (Annex E).
#[derive(Debug)]
pub struct AdoptPlan {
    pub ops: Vec<StagedOp>,
    /// Sorted, so the result is stable regardless of how the drift was
    /// discovered.
    pub paths: Vec<RepoPath>,
}

/// Turns drift into staged ops (D7).
///
/// One entry, one op, decided by *where the path is* and *how it drifted*:
///
/// | | `Modified` | `Untracked` | `Missing` |
/// |---|---|---|---|
/// | `telos/{notions,intents,constraints}/*.tel` | `edit` | `add` | `remove` |
/// | anything else (`telos.toml`, `bindings.tel`, code) | `accept` | `accept` | refused |
///
/// The `edit`/`add` ops re-parse the file's *current* content, which is what
/// makes `adopt` canonicalizing: the op carries the parsed entity, so the
/// reconcile writes the canonical bytes back and whatever whitespace the
/// out-of-protocol edit introduced never reaches the seal. The `remove` op
/// takes its identity from the path -- there is no content left to read --
/// which is why an entity's location being a function of its identity
/// ([`notion_path`] and friends) is load-bearing here and not just tidy.
///
/// Costs exactly one `git hash-object` child process, for every path that
/// needs a current OID, however wide the drift.
///
/// `lock` is read only to phrase the refusal above: whether a deleted opaque
/// file was bound code or an unbound spec file changes what the caller has
/// to do about it.
pub fn plan_adopt(
    ws: &Workspace,
    git: &GitRepo,
    lock: &Lock,
    drift: &[DriftEntry],
) -> Result<AdoptPlan, TelosError> {
    let mut entries: Vec<&DriftEntry> = drift.iter().collect();
    entries.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    // Every opaque path that still exists needs its current OID; one batch
    // for all of them.
    let opaque: Vec<RepoPath> = entries
        .iter()
        .filter(|e| slot_of(&e.path).is_none() && e.kind != DriftKind::Missing)
        .map(|e| e.path.clone())
        .collect();
    let oids = git.blob_oids(&opaque)?;

    let mut ops = Vec::with_capacity(entries.len());
    let mut paths = Vec::with_capacity(entries.len());

    for entry in entries {
        let op = match slot_of(&entry.path) {
            Some((slot, key)) => entity_op(ws, &entry.path, slot, key, entry.kind)?,
            // A path present in `oids` is one that exists on disk; anything
            // else is a deletion this command cannot express -- including
            // the rare case of a file that vanished between `compute_state`
            // and now, which is the same situation with worse timing.
            None => match oids.get(&entry.path) {
                Some(oid) => StagedOp::Accept {
                    path: entry.path.clone(),
                    oid: oid.clone(),
                },
                None => return Err(undeletable(lock, &entry.path)),
            },
        };
        ops.push(op);
        paths.push(entry.path.clone());
    }

    Ok(AdoptPlan { ops, paths })
}

/// What `revert` did.
#[derive(Debug)]
pub struct RevertOutcome {
    /// Sealed paths written back from their sealed blob, sorted.
    pub restored: Vec<RepoPath>,
    /// Unsealed paths removed from the working tree, sorted.
    pub deleted: Vec<RepoPath>,
}

/// Restores the sealed state of every drifted path (D7).
///
/// `Modified` and `Missing` are one case: both are sealed paths whose bytes
/// must come back, and both come back the same way -- [`GitRepo::cat_blob`]
/// of the OID `lock` records, written over whatever is (or is not) there,
/// creating parent directories as needed. `Untracked` is the other: a file
/// the seal never mentioned, so restoring it means deleting it.
///
/// Not atomic, and deliberately not pretending to be: a failure part-way
/// leaves the paths already restored restored. That is strictly closer to
/// the seal than where it started, and re-running `revert` finishes the job
/// -- which is a better property than a rollback that would put the drift
/// back.
pub fn revert(
    ws: &Workspace,
    git: &GitRepo,
    lock: &Lock,
    drift: &[DriftEntry],
) -> Result<RevertOutcome, TelosError> {
    let mut restored = Vec::new();
    let mut deleted = Vec::new();

    for entry in drift {
        let abs = ws.abs_path(&entry.path);
        match entry.kind {
            DriftKind::Modified | DriftKind::Missing => {
                let oid = lock
                    .spec
                    .get(&entry.path)
                    .or_else(|| lock.code.get(&entry.path))
                    .ok_or_else(|| unsealed(&entry.path))?;
                let bytes = git.cat_blob(oid)?;
                if let Some(parent) = abs.parent() {
                    fs::create_dir_all(parent).map_err(|e| io_error("create", &entry.path, e))?;
                }
                fs::write(&abs, bytes).map_err(|e| io_error("restore", &entry.path, e))?;
                restored.push(entry.path.clone());
            }
            DriftKind::Untracked => {
                match fs::remove_file(&abs) {
                    Ok(()) => {}
                    // Already gone: the outcome this asked for.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_error("delete", &entry.path, e)),
                }
                deleted.push(entry.path.clone());
            }
        }
    }

    restored.sort();
    deleted.sort();
    Ok(RevertOutcome { restored, deleted })
}

// --- classifying a drifted path ---------------------------------------------

/// Which spec directory an entity file lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Notion,
    Intent,
    Constraint,
}

/// The entity slot a path names, with the file's stem -- the entity's
/// identity as the *path* spells it.
///
/// `None` for everything else: `telos/telos.toml`, `telos/bindings.tel`, and
/// every code file. Those are the paths the model holds no entity for, and
/// the ones `accept` exists for.
fn slot_of(path: &RepoPath) -> Option<(Slot, &str)> {
    let s = path.as_str();
    let (slot, rest) = if let Some(rest) = s.strip_prefix("telos/notions/") {
        (Slot::Notion, rest)
    } else if let Some(rest) = s.strip_prefix("telos/intents/") {
        (Slot::Intent, rest)
    } else {
        (Slot::Constraint, s.strip_prefix("telos/constraints/")?)
    };

    let stem = rest.strip_suffix(".tel")?;
    // A nested path under a spec directory is not an entity file --
    // `spec_files()` never lists one, so this can only come from a lock, and
    // treating it as an opaque path is the honest answer.
    if stem.is_empty() || stem.contains('/') {
        return None;
    }
    Some((slot, stem))
}

/// The op one drifted entity file becomes.
fn entity_op(
    ws: &Workspace,
    path: &RepoPath,
    slot: Slot,
    key: &str,
    kind: DriftKind,
) -> Result<StagedOp, TelosError> {
    if kind == DriftKind::Missing {
        return match slot {
            Slot::Notion => NotionName::new(key)
                .map(StagedOp::RemoveNotion)
                .map_err(|_| unidentifiable(path)),
            Slot::Intent => key
                .parse::<IntentId>()
                .map(StagedOp::RemoveIntent)
                .map_err(|_| unidentifiable(path)),
            Slot::Constraint => key
                .parse::<ConstraintId>()
                .map(StagedOp::RemoveConstraint)
                .map_err(|_| unidentifiable(path)),
        };
    }

    let src = fs::read_to_string(ws.abs_path(path)).map_err(|e| io_error("read", path, e))?;
    // `Untracked` means the seal never held this path, which is what an
    // `add` says; `Modified` means it did, which is what an `edit` says.
    let adding = kind == DriftKind::Untracked;

    match slot {
        Slot::Notion => {
            let notion = parse_notion_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &notion_path(&notion.name))?;
            Ok(if adding {
                StagedOp::AddNotion(notion)
            } else {
                StagedOp::EditNotion(notion)
            })
        }
        Slot::Intent => {
            let intent = parse_intent_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &intent_path(intent.id))?;
            Ok(if adding {
                StagedOp::AddIntent(intent)
            } else {
                StagedOp::EditIntent(intent)
            })
        }
        Slot::Constraint => {
            let constraint = parse_constraint_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &constraint_path(constraint.id))?;
            Ok(if adding {
                StagedOp::AddConstraint(constraint)
            } else {
                StagedOp::EditConstraint(constraint)
            })
        }
    }
}

/// Refuses a file whose declared entity belongs somewhere else.
///
/// An op's target path comes from the entity's identity, never from where
/// the file happened to be ([`StagedOp::target_path`]). So adopting
/// `notions/Rogue.tel` that declares `notion Other` would stage an op
/// claiming `notions/Other.tel` -- leaving `Rogue.tel` still drifted, still
/// unclaimed, and the project still not `changing` after an `adopt` that
/// reported success. Refusing is the only outcome that keeps `adopt`'s
/// promise: after it, the drift it named is captured.
fn require_identity(path: &RepoPath, declared: &RepoPath) -> Result<(), TelosError> {
    if path == declared {
        return Ok(());
    }
    Err(TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("`{path}` declares an entity that belongs in `{declared}`"),
    )
    .hint("rename the file to match the entity it declares, or the entity to match the file"))
}

// --- the refusals -----------------------------------------------------------

/// A drifted `.tel` file the parser cannot read.
///
/// The first diagnostic carries the position and the diagnosis, which is
/// what makes the message actionable; the code is forced to
/// `TELOS_PARSE_ERROR` and the hint replaced with [`ADOPT_PARSE_HINT`],
/// because from the caller's side this is one situation with two exits, not
/// a parse report.
fn unparseable(path: &RepoPath, diagnostics: Vec<Diagnostic>) -> TelosError {
    let mut error = match diagnostics.into_iter().next() {
        Some(diagnostic) => TelosError::from(diagnostic),
        None => TelosError::new(
            ErrorCode::TelosParseError,
            format!("{path}: cannot be parsed"),
        ),
    };
    error.code = ErrorCode::TelosParseError;
    error.hint(ADOPT_PARSE_HINT)
}

/// A deleted file that carries no entity, so no op can express its deletion.
///
/// Bound code and an unbound spec file get the same message shape and
/// different hints: dropping a binding is a real next step for the first and
/// meaningless for the second. `lock` is what tells them apart -- a path in
/// `lock.code` is there because a binding put it there.
fn undeletable(lock: &Lock, path: &RepoPath) -> TelosError {
    let (what, hint) = if lock.code.contains_key(path) {
        (
            "bound file ",
            "restore it with `telos revert`, or remove its binding",
        )
    } else {
        ("", "restore it with `telos revert`")
    };
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("cannot adopt: {what}`{path}` was deleted"),
    )
    .hint(hint)
}

/// An entity file whose *name* is not a valid identity -- so not even its
/// deletion can be expressed. Only reachable from a lock that sealed such a
/// path in the first place.
fn unidentifiable(path: &RepoPath) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("cannot read an entity identity from `{path}`"),
    )
    .hint(format!("restore `{path}` with `telos revert`"))
}

/// A drifted path the lock does not seal, which [`crate::state::compute_state`]
/// cannot produce for a `Modified` or `Missing` entry -- covered rather than
/// assumed away.
fn unsealed(path: &RepoPath) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("`{path}` is not sealed; there is nothing to restore it from"),
    )
    .hint("run `telos change reconcile --full` to reseal the project")
}

fn io_error(verb: &str, path: &RepoPath, e: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {path}: {e}"),
    )
}

#[cfg(test)]
mod tests {
    //! The one decision here that is a pure function of its argument.
    //! Everything else needs a real workspace, a real repository and a real
    //! seal, and is covered end to end in `crates/telos/tests/adopt_revert.rs`.

    use super::*;

    /// [`slot_of`] borrows from its argument, so the answer is copied out
    /// before the `RepoPath` it borrows from is dropped.
    fn slot(path: &str) -> Option<(Slot, String)> {
        let path = RepoPath::new(path);
        slot_of(&path).map(|(slot, key)| (slot, key.to_string()))
    }

    #[test]
    fn slot_of_recognizes_the_three_entity_directories() {
        assert_eq!(
            slot("telos/notions/Invoice.tel"),
            Some((Slot::Notion, "Invoice".to_string()))
        );
        assert_eq!(
            slot("telos/intents/INT-0042.tel"),
            Some((Slot::Intent, "INT-0042".to_string()))
        );
        assert_eq!(
            slot("telos/constraints/CON-0003.tel"),
            Some((Slot::Constraint, "CON-0003".to_string()))
        );
    }

    #[test]
    fn slot_of_rejects_everything_the_model_holds_no_entity_for() {
        assert_eq!(slot("telos/telos.toml"), None);
        assert_eq!(slot("telos/bindings.tel"), None);
        assert_eq!(slot("src/billing/invoice.rs"), None);
        // Not a `.tel` file, no stem, or nested: none of them is a file
        // `spec_files()` would ever list as an entity.
        assert_eq!(slot("telos/notions/README.md"), None);
        assert_eq!(slot("telos/notions/.tel"), None);
        assert_eq!(slot("telos/notions/nested/Invoice.tel"), None);
    }
}
