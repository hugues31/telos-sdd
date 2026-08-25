//! The two exits from drift: capture the current bytes or restore the seal.
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

use crate::error::{Diagnostic, ErrorCode, TelosError};
use crate::git::GitRepo;
use crate::ids::{CapabilityRef, ConstraintId, ContextId, IntentId, NotionName, Owner, RepoPath};
use crate::lock::Lock;
use crate::model::StagedOp;
use crate::model::change::{
    capability_path, context_path, owned_constraint_path, owned_intent_path, owned_notion_path,
};
use crate::repo_fs::RepoFs;
use crate::state::{DriftEntry, DriftKind};
use crate::syntax::{
    parse_capability_file, parse_context_file, parse_context_map_file, parse_owned_constraint_file,
    parse_owned_intent_file, parse_owned_notion_file,
};
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
/// reports.
#[derive(Debug)]
pub struct AdoptPlan {
    pub ops: Vec<StagedOp>,
    /// Sorted, so the result is stable regardless of how the drift was
    /// discovered.
    pub paths: Vec<RepoPath>,
}

/// Turns drift into staged ops.
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
        .filter(|e| classify_slot(&e.path).is_none() && e.kind != DriftKind::Missing)
        .map(|e| e.path.clone())
        .collect();
    let oids = git.blob_oids(&opaque)?;

    let mut candidates = Vec::with_capacity(entries.len());
    let mut paths = Vec::with_capacity(entries.len());

    for entry in entries {
        let op = match classify_slot(&entry.path) {
            Some(slot) => entity_op(ws, &entry.path, slot, entry.kind)?,
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
        candidates.push((entry.path.clone(), op));
        paths.push(entry.path.clone());
    }

    let ops = pair_moves(candidates)?;
    Ok(AdoptPlan { ops, paths })
}

/// Collapses the two drift entries of a physical relocation into one
/// identity-preserving move. Numeric intent/constraint ids are global;
/// notion identity also includes its context, so a notion can only move
/// between the shared area and capabilities of the same context.
fn pair_moves(candidates: Vec<(RepoPath, StagedOp)>) -> Result<Vec<StagedOp>, TelosError> {
    let mut used = vec![false; candidates.len()];
    let mut paired_at = vec![None; candidates.len()];

    for index in 0..candidates.len() {
        let (_, candidate) = &candidates[index];
        if is_remove(candidate) {
            let matches: Vec<usize> = (0..candidates.len())
                .filter(|other| *other != index && !used[*other])
                .filter(|other| move_from(candidate, &candidates[*other].1).is_some())
                .collect();
            if matches.len() > 1 {
                return Err(TelosError::new(
                    ErrorCode::TelosIntegrityViolation,
                    format!(
                        "cannot adopt move from `{}`: multiple destinations declare the same identity",
                        candidates[index].0
                    ),
                ));
            }
            if let Some(other) = matches.first().copied() {
                used[index] = true;
                used[other] = true;
                paired_at[index.min(other)] =
                    Some(move_from(candidate, &candidates[other].1).expect("matched above"));
            }
        }
    }
    let mut ops = Vec::new();
    for (index, (_, candidate)) in candidates.iter().enumerate() {
        if let Some(operation) = paired_at[index].take() {
            ops.push(operation);
        } else if !used[index] {
            ops.push(candidate.clone());
        }
    }
    Ok(ops)
}

fn is_remove(op: &StagedOp) -> bool {
    matches!(
        op,
        StagedOp::RemoveOwnedNotion { .. }
            | StagedOp::RemoveOwnedIntent { .. }
            | StagedOp::RemoveOwnedConstraint { .. }
    )
}

fn move_from(remove: &StagedOp, add: &StagedOp) -> Option<StagedOp> {
    match (remove, add) {
        (
            StagedOp::RemoveOwnedNotion { owner: from, name },
            StagedOp::AddOwnedNotion { owner: to, notion },
        ) if name == &notion.name && from.context == to.context && from != to => {
            Some(StagedOp::MoveNotion {
                from: from.clone(),
                to: to.clone(),
                notion: notion.clone(),
            })
        }
        (
            StagedOp::RemoveOwnedIntent { owner: from, id },
            StagedOp::AddOwnedIntent { owner: to, intent },
        ) if id == &intent.id && from != to => Some(StagedOp::MoveIntent {
            from: from.clone(),
            to: to.clone(),
            intent: intent.clone(),
        }),
        (
            StagedOp::RemoveOwnedConstraint { owner: from, id },
            StagedOp::AddOwnedConstraint {
                owner: to,
                constraint,
            },
        ) if id == &constraint.id && from != to => Some(StagedOp::MoveConstraint {
            from: from.clone(),
            to: to.clone(),
            constraint: constraint.clone(),
        }),
        _ => None,
    }
}

/// What `revert` did.
#[derive(Debug)]
pub struct RevertOutcome {
    /// Sealed paths written back from their sealed blob, sorted.
    pub restored: Vec<RepoPath>,
    /// Unsealed paths removed from the working tree, sorted.
    pub deleted: Vec<RepoPath>,
}

/// Restores the sealed state of every drifted path.
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
    let repo_fs = RepoFs::open(&ws.repo_root)?;

    for entry in drift {
        entry.path.validate()?;
        match entry.kind {
            DriftKind::Modified | DriftKind::Missing => {
                let oid = lock
                    .spec
                    .get(&entry.path)
                    .or_else(|| lock.code.get(&entry.path))
                    .ok_or_else(|| unsealed(&entry.path))?;
                let bytes = git.cat_blob(oid)?;
                repo_fs.write(&entry.path, &bytes)?;
                restored.push(entry.path.clone());
            }
            DriftKind::Untracked => {
                repo_fs.remove_file(&entry.path)?;
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
#[derive(Debug, Clone, PartialEq, Eq)]
enum Slot {
    Context(ContextId),
    Capability(CapabilityRef),
    Notion(Owner, NotionName),
    Intent(Owner, IntentId),
    Constraint(Option<Owner>, ConstraintId),
    ContextMap,
}

/// The entity slot a path names, with the file's stem -- the entity's
/// identity as the *path* spells it.
///
/// `None` for everything else: `telos/telos.toml`, `telos/bindings.tel`, and
/// every code file. Those are the paths the model holds no entity for, and
/// the ones `accept` exists for.
fn classify_slot(path: &RepoPath) -> Option<Slot> {
    let parts: Vec<&str> = path.as_str().split('/').collect();
    match parts.as_slice() {
        ["telos", "context-map.tel"] => Some(Slot::ContextMap),
        ["telos", "constraints", file] => {
            Some(Slot::Constraint(None, tel_stem(file)?.parse().ok()?))
        }
        ["telos", "contexts", context, "context.tel"] => {
            Some(Slot::Context(ContextId::new(*context).ok()?))
        }
        ["telos", "contexts", context, "notions", file] => Some(Slot::Notion(
            Owner::context(ContextId::new(*context).ok()?),
            NotionName::new(tel_stem(file)?).ok()?,
        )),
        ["telos", "contexts", context, "constraints", file] => Some(Slot::Constraint(
            Some(Owner::context(ContextId::new(*context).ok()?)),
            tel_stem(file)?.parse().ok()?,
        )),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "capability.tel",
        ] => Some(Slot::Capability(CapabilityRef::new(
            ContextId::new(*context).ok()?,
            crate::ids::CapabilityId::new(*capability).ok()?,
        ))),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "notions",
            file,
        ] => Some(Slot::Notion(
            Owner::capability(
                format!("{context}/{capability}")
                    .parse::<CapabilityRef>()
                    .ok()?,
            ),
            NotionName::new(tel_stem(file)?).ok()?,
        )),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "intents",
            file,
        ] => Some(Slot::Intent(
            Owner::capability(
                format!("{context}/{capability}")
                    .parse::<CapabilityRef>()
                    .ok()?,
            ),
            tel_stem(file)?.parse().ok()?,
        )),
        [
            "telos",
            "contexts",
            context,
            "capabilities",
            capability,
            "constraints",
            file,
        ] => Some(Slot::Constraint(
            Some(Owner::capability(
                format!("{context}/{capability}")
                    .parse::<CapabilityRef>()
                    .ok()?,
            )),
            tel_stem(file)?.parse().ok()?,
        )),
        _ => None,
    }
}

fn tel_stem(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".tel")?;
    (!stem.is_empty()).then_some(stem)
}

/// The op one drifted entity file becomes.
fn entity_op(
    ws: &Workspace,
    path: &RepoPath,
    slot: Slot,
    kind: DriftKind,
) -> Result<StagedOp, TelosError> {
    if kind == DriftKind::Missing {
        return match slot {
            Slot::Context(id) => Ok(StagedOp::RemoveContext(id)),
            Slot::Capability(id) => Ok(StagedOp::RemoveCapability(id)),
            Slot::Notion(owner, name) => Ok(StagedOp::RemoveOwnedNotion { owner, name }),
            Slot::Intent(owner, id) => Ok(StagedOp::RemoveOwnedIntent { owner, id }),
            Slot::Constraint(owner, id) => Ok(StagedOp::RemoveOwnedConstraint { owner, id }),
            Slot::ContextMap => Err(undeletable_map(path)),
        };
    }

    let src = ws.read_to_string(path)?;
    // `Untracked` means the seal never held this path, which is what an
    // `add` says; `Modified` means it did, which is what an `edit` says.
    let adding = kind == DriftKind::Untracked;

    match slot {
        Slot::Context(expected) => {
            let context = parse_context_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &context_path(&context.id))?;
            require_declared(expected == context.id, path)?;
            Ok(if adding {
                StagedOp::AddContext(context)
            } else {
                StagedOp::EditContext(context)
            })
        }
        Slot::Capability(expected) => {
            let capability = parse_capability_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &capability_path(&capability.id))?;
            require_declared(expected == capability.id, path)?;
            Ok(if adding {
                StagedOp::AddCapability(capability)
            } else {
                StagedOp::EditCapability(capability)
            })
        }
        Slot::Notion(expected, _) => {
            let (owner, notion) =
                parse_owned_notion_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &owned_notion_path(&owner, &notion.name))?;
            require_declared(expected == owner, path)?;
            Ok(if adding {
                StagedOp::AddOwnedNotion { owner, notion }
            } else {
                StagedOp::EditOwnedNotion { owner, notion }
            })
        }
        Slot::Intent(expected, _) => {
            let (owner, intent) =
                parse_owned_intent_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &owned_intent_path(&owner, intent.id))?;
            require_declared(expected == owner, path)?;
            Ok(if adding {
                StagedOp::AddOwnedIntent { owner, intent }
            } else {
                StagedOp::EditOwnedIntent { owner, intent }
            })
        }
        Slot::Constraint(expected, _) => {
            let (owner, constraint) =
                parse_owned_constraint_file(path, &src).map_err(|d| unparseable(path, d))?;
            require_identity(path, &owned_constraint_path(owner.as_ref(), constraint.id))?;
            require_declared(expected == owner, path)?;
            Ok(if adding {
                StagedOp::AddOwnedConstraint { owner, constraint }
            } else {
                StagedOp::EditOwnedConstraint { owner, constraint }
            })
        }
        Slot::ContextMap => parse_context_map_file(path, &src)
            .map(StagedOp::EditContextMap)
            .map_err(|d| unparseable(path, d)),
    }
}

fn require_declared(matches: bool, path: &RepoPath) -> Result<(), TelosError> {
    if matches {
        Ok(())
    } else {
        Err(unidentifiable(path))
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

fn undeletable_map(path: &RepoPath) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("cannot adopt: required context map `{path}` was deleted"),
    )
    .hint("restore it with `telos revert`, or recreate it before adopting")
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

#[cfg(test)]
mod tests {
    //! The one decision here that is a pure function of its argument.
    //! Everything else needs a real workspace, a real repository and a real
    //! seal, and is covered end to end in `crates/telos/tests/adopt_revert.rs`.

    use super::*;

    fn slot(path: &str) -> Option<Slot> {
        classify_slot(&RepoPath::new(path))
    }

    #[test]
    fn slot_of_recognizes_canonical_owned_entity_paths() {
        assert_eq!(
            slot("telos/contexts/billing/notions/Invoice.tel"),
            Some(Slot::Notion(
                Owner::context(ContextId::new("billing").unwrap()),
                NotionName::new("Invoice").unwrap(),
            ))
        );
        assert_eq!(
            slot("telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel"),
            Some(Slot::Intent(
                Owner::capability("billing/settlement".parse().unwrap()),
                IntentId(42),
            ))
        );
        assert_eq!(
            slot("telos/constraints/CON-0003.tel"),
            Some(Slot::Constraint(None, ConstraintId(3)))
        );
    }

    #[test]
    fn slot_of_rejects_legacy_and_opaque_paths() {
        assert_eq!(slot("telos/telos.toml"), None);
        assert_eq!(slot("telos/bindings.tel"), None);
        assert_eq!(slot("telos/intents/INT-0042.tel"), None);
        assert_eq!(slot("src/billing/invoice.rs"), None);
        assert_eq!(slot("telos/contexts/billing/bindings.tel"), None);
    }

    #[test]
    fn pair_moves_finds_a_destination_that_sorts_before_the_source() {
        let from = Owner::capability("billing/settlement".parse().unwrap());
        let to = Owner::capability("billing/invoicing".parse().unwrap());
        let source = concat!(
            "intent INT-0042 in billing/invoicing \"Moved intent\" {\n",
            "  status draft\n",
            "  telos  \"Exercise order-independent move pairing.\"\n",
            "  statement ubiquitous {\n",
            "    system shall \"record the move\"\n",
            "  }\n",
            "}\n",
        );
        let (_, intent) = parse_owned_intent_file(
            &RepoPath::new("telos/contexts/billing/capabilities/invoicing/intents/INT-0042.tel"),
            source,
        )
        .unwrap();
        let candidates = vec![
            (
                RepoPath::new("telos/contexts/billing/capabilities/invoicing/intents/INT-0042.tel"),
                StagedOp::AddOwnedIntent {
                    owner: to.clone(),
                    intent: intent.clone(),
                },
            ),
            (
                RepoPath::new(
                    "telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel",
                ),
                StagedOp::RemoveOwnedIntent {
                    owner: from.clone(),
                    id: IntentId(42),
                },
            ),
        ];

        assert_eq!(
            pair_moves(candidates).unwrap(),
            vec![StagedOp::MoveIntent { from, to, intent }]
        );
    }
}
