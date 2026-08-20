//! The overlay: the sealed spec with a change's staged ops applied on top,
//! in memory, and the rules that decide whether that is even legal.
//!
//! A change never writes a `.tel` file while it is open -- it writes *ops*.
//! So the only way to answer "would this delta leave the spec coherent?" is
//! to build the spec the delta describes and check that one, which is what
//! this module is: [`parse_base`] reads the tree, one of the two `apply`
//! functions below replays a change's ops over it, and [`build_model`] runs
//! the semantic pass on the result. Nothing here touches the filesystem
//! beyond that one read, and nothing here ever writes.
//!
//! # Two ways to apply the same ops, and who uses which
//!
//! | | base is | refuses | callers |
//! |---|---|---|---|
//! | [`apply_ops`] | the tree as the sealed spec | an op that contradicts it | `telos add\|edit\|remove`, on the **one op being staged now** |
//! | [`apply_ops_idempotent`] (+ [`validate_ops_idempotent`]) | a tree that may already show part of the delta | nothing | `adopt` and `reconcile`, on a **whole change** |
//!
//! The split is D7's doing. `adopt` turns drift into ops describing a tree
//! that already shows them -- `add` of a file that is already there, `remove`
//! of one that is already gone -- so the staging preconditions would refuse
//! the very state they exist to protect. [`apply_ops_idempotent`]'s own doc
//! carries the full argument; the short version is that those refusals are
//! about *the op*, and only the caller staging it is in a position to act on
//! them.
//!
//! Three properties are load-bearing:
//!
//! - **Order is data** (D1). The ops replay sequentially against one
//!   another's output, so `add X` then `remove X` is a different transaction
//!   from `remove X` then `add X`, and rule 2 below is judged at the point
//!   in the sequence where the removal happens. What order does *not* change
//!   is the verdict of the semantic pass: [`build_model`] runs once, on the
//!   state the last op leaves behind.
//! - **A path is an identity** ([`StagedOp::target_path`]). The base is
//!   keyed by repo-relative path, and an entity's path is a function of its
//!   kind and its id, so two ops on one entity always meet on one slot.
//! - **Rule 2 of §3.3 is enforced twice, differently.** [`apply_ops`] refuses
//!   a removal *naming the referrer* (`cannot remove intent INT-0017:
//!   INT-0042 requires it`) -- the good message, produced where the mistake
//!   was just made. [`apply_ops_idempotent`] cannot check it (the referrer
//!   may itself be part of the drift), so on that path the same violation
//!   surfaces from [`build_model`] as an unresolvable reference. Nothing
//!   escapes: every referrer `first_referrer` scans -- a notion's `rel`, an
//!   intent's `refines`/`requires`/`excludes`, a constraint's `scope`, a
//!   binding's `implements`/`proves` -- is a reference the semantic pass
//!   resolves too, and `crates/telos-core/tests/overlay.rs` pins that.
//!
//! The rules this module does *not* enforce are the ones that need more than
//! the spec tree: no-code-without-telos (rule 5) needs `telos.toml`'s globs
//! and the working tree, and the red-witness discipline needs test runs.
//! Both belong to reconcile.

use std::collections::BTreeMap;

use crate::emit::{emit_constraint, emit_intent, emit_notion};
use crate::error::{Diagnostic, ErrorCode, TelosError};
use crate::ids::{NotionName, RepoPath};
use crate::model::{Binding, Notion, Rule, Scope, StagedOp, TelFile, TelosModel};
use crate::semantic::{build_model, expr_notions, scenario_notions, statement_notions};
use crate::suggest::closest;
use crate::workspace::Workspace;

/// Reads and parses the sealed spec tree, without building a model from it.
///
/// The distinction matters: [`Workspace::load_model`] would reject a base
/// that does not resolve on its own, but the whole job of a change is to
/// carry the spec from one coherent state to another, and the ops are what
/// close the gap. So the base is only required to *parse*; whether it holds
/// together is decided once the ops are on top of it -- by [`build_model`],
/// through [`validate_ops_idempotent`] or through the staging caller's own
/// pass.
pub fn parse_base(ws: &Workspace) -> Result<Vec<(RepoPath, TelFile)>, Vec<Diagnostic>> {
    ws.parse_spec_files()
}

/// Every notion of a base, by name -- the map
/// [`crate::payload::intent_from_json`] types a scenario's `fields`
/// against, which for a staged op must include the notions this very change
/// added earlier.
pub fn notions_of(base: &[(RepoPath, TelFile)]) -> BTreeMap<NotionName, Notion> {
    base.iter()
        .filter_map(|(_, file)| match file {
            TelFile::Notion(notion) => Some((notion.name.clone(), notion.clone())),
            _ => None,
        })
        .collect()
}

/// The entity a base holds at `path`, if any.
pub fn find_file<'a>(base: &'a [(RepoPath, TelFile)], path: &RepoPath) -> Option<&'a TelFile> {
    base.iter().find(|(p, _)| p == path).map(|(_, file)| file)
}

/// « unknown intent `INT-9999` », with the `closest is …` suffix when one of
/// the base's own entities of that kind is close enough.
///
/// `entity` is a [`StagedOp::entity`] word (`"notion"`, `"intent"`,
/// `"constraint"`). Public because the CLI resolves an `edit`/`remove`
/// target itself -- it needs the base entity to patch -- and must report a
/// miss exactly as [`apply_ops`] would.
pub fn unknown_entity(base: &[(RepoPath, TelFile)], entity: &str, key: &str) -> TelosError {
    let known: Vec<String> = base
        .iter()
        .filter_map(|(_, file)| match (entity, file) {
            ("notion", TelFile::Notion(n)) => Some(n.name.to_string()),
            ("intent", TelFile::Intent(i)) => Some(i.id.to_string()),
            ("constraint", TelFile::Constraint(c)) => Some(c.id.to_string()),
            _ => None,
        })
        .collect();
    let candidates: Vec<&str> = known.iter().map(String::as_str).collect();
    let message = format!("unknown {entity} `{key}`");
    let message = match closest(key, candidates.iter().copied()) {
        Some(known) => format!("{message}; closest is `{known}`"),
        None => message,
    };
    TelosError::new(ErrorCode::TelosReferenceUnknown, message)
}

/// Replays `ops` over `base`, in staged order, and hands back the spec they
/// describe -- reading `base` as the spec those ops were written against.
///
/// This is the **staging** application: `telos add|edit|remove` runs it on
/// the single op it is about to stage, over the overlay the change already
/// describes. For a whole change, whose base may already show part of the
/// delta, see [`apply_ops_idempotent`].
///
/// The three refusals, each with its own dedicated message:
///
/// - **add** of something the base already holds --
///   `` notion `Invoice` already exists `` (`TELOS_INTEGRITY_VIOLATION`).
/// - **edit**/**remove** of something it does not --
///   `` unknown intent `INT-9999` `` (`TELOS_REFERENCE_UNKNOWN`).
/// - **remove** of something still referenced (rule 2) --
///   `cannot remove intent INT-0017: INT-0042 requires it`
///   (`TELOS_INTEGRITY_VIOLATION`), see [`first_referrer`].
///
/// An `accept` op is inert: it seals the current bytes of a path the model
/// does not represent (`telos/telos.toml`, a code file), which is a
/// reconcile-time concern, not an overlay one.
///
/// Everything else the spec must satisfy -- that references resolve, that an
/// active intent has a scenario, that literals match their attribute types
/// -- is [`build_model`]'s, and every caller runs it on the overlay this
/// returns. Splitting it that way is deliberate: the three rules above are
/// about the *op* (it contradicts what the base holds), and their messages
/// name the op's own target, while a semantic diagnostic is about the *spec*
/// and names a file.
pub fn apply_ops(
    mut base: Vec<(RepoPath, TelFile)>,
    ops: &[StagedOp],
) -> Result<Vec<(RepoPath, TelFile)>, TelosError> {
    for op in ops {
        apply_one(&mut base, op)?;
    }
    Ok(base)
}

/// Applies one op in place. On `Err` the vector is left in whatever state
/// the failing op reached, which no caller can observe: [`apply_ops`] owns
/// it and drops it.
///
/// The dispatch is over the variants themselves rather than over
/// [`StagedOp::verb`]'s word, so the post-state an `add`/`edit` writes comes
/// out of the match arm that knows it exists -- no `expect` reconstructing
/// what the verb already implied -- and a new variant is a compile error
/// here rather than a silent fall into the `remove` arm.
fn apply_one(base: &mut Vec<(RepoPath, TelFile)>, op: &StagedOp) -> Result<(), TelosError> {
    match op {
        // Inert: an `accept` seals the current bytes of a path the model
        // holds no entity for -- a reconcile-time concern, not an overlay
        // one.
        StagedOp::Accept { .. } => Ok(()),

        StagedOp::AddNotion(n) => add(base, op, TelFile::Notion(n.clone())),
        StagedOp::AddIntent(i) => add(base, op, TelFile::Intent(i.clone())),
        StagedOp::AddConstraint(c) => add(base, op, TelFile::Constraint(c.clone())),

        StagedOp::EditNotion(n) => edit(base, op, TelFile::Notion(n.clone())),
        StagedOp::EditIntent(i) => edit(base, op, TelFile::Intent(i.clone())),
        StagedOp::EditConstraint(c) => edit(base, op, TelFile::Constraint(c.clone())),

        StagedOp::RemoveNotion(_) | StagedOp::RemoveIntent(_) | StagedOp::RemoveConstraint(_) => {
            remove(base, op)
        }
    }
}

/// Inserts `file` at `op`'s target path, refusing if the base already holds
/// something there. The base is kept sorted by path, as [`parse_base`]
/// hands it over.
fn add(
    base: &mut Vec<(RepoPath, TelFile)>,
    op: &StagedOp,
    file: TelFile,
) -> Result<(), TelosError> {
    let path = op.target_path();
    if base.iter().any(|(p, _)| *p == path) {
        return Err(TelosError::new(
            ErrorCode::TelosIntegrityViolation,
            format!("{} already exists", label(op)),
        ));
    }
    base.push((path, file));
    base.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
    Ok(())
}

/// Replaces what the base holds at `op`'s target path, refusing if it holds
/// nothing there.
fn edit(base: &mut [(RepoPath, TelFile)], op: &StagedOp, file: TelFile) -> Result<(), TelosError> {
    match base.iter().position(|(p, _)| *p == op.target_path()) {
        Some(slot) => {
            base[slot].1 = file;
            Ok(())
        }
        None => Err(unknown_entity(base, op.entity(), &op.key())),
    }
}

/// Drops what the base holds at `op`'s target path, then enforces rule 2 of
/// §3.3 against the state the removal leaves behind.
fn remove(base: &mut Vec<(RepoPath, TelFile)>, op: &StagedOp) -> Result<(), TelosError> {
    let Some(slot) = base.iter().position(|(p, _)| *p == op.target_path()) else {
        return Err(unknown_entity(base, op.entity(), &op.key()));
    };
    let (_, removed) = base.remove(slot);
    match first_referrer(base, &removed) {
        Some(referrer) => Err(TelosError::new(
            ErrorCode::TelosIntegrityViolation,
            format!("cannot remove {}: {referrer}", label(op)),
        )),
        None => Ok(()),
    }
}

/// How an op's target is named in a message: a notion by its quoted name, an
/// id-bearing entity by its bare id -- the same two shapes
/// [`crate::semantic`] already uses for a duplicate declaration.
fn label(op: &StagedOp) -> String {
    match op.entity() {
        "notion" => format!("notion `{}`", op.key()),
        entity => format!("{entity} {}", op.key()),
    }
}

/// Rule 2 of §3.3: the first thing in `base` that still points at `removed`,
/// rendered as the tail of the refusal message (`INT-0042 requires it`).
///
/// `base` is the state *after* the removal, so what it finds is exactly what
/// would be left dangling. `removed` is the whole file rather than its key
/// because an intent takes its scenarios with it: a `proves` binding on one
/// of them is broken by the removal just as surely as a `requires` edge.
///
/// Nothing in the model points at a constraint (a constraint's own `scope`
/// points *out*, at intents), so removing one is never refused here.
///
/// The scan is deterministic: `base` is ordered by path, and each file is
/// examined in a fixed order, so the same removal always names the same
/// referrer.
fn first_referrer(base: &[(RepoPath, TelFile)], removed: &TelFile) -> Option<String> {
    match removed {
        TelFile::Notion(notion) => first_notion_referrer(base, &notion.name),
        TelFile::Intent(intent) => {
            let scenarios: Vec<_> = intent.scenarios.iter().map(|s| s.id).collect();
            first_intent_referrer(base, intent.id, &scenarios)
        }
        TelFile::Constraint(_) | TelFile::Bindings(_) => None,
    }
}

fn first_notion_referrer(base: &[(RepoPath, TelFile)], name: &NotionName) -> Option<String> {
    for (_, file) in base {
        match file {
            TelFile::Notion(other) => {
                let targeted = other.rels.iter().any(|rel| rel.target.node == *name)
                    || other.attrs.iter().any(|attr| match &attr.ty {
                        crate::model::AttrType::Ref(target) => target == name,
                        _ => false,
                    });
                if targeted {
                    return Some(format!("notion `{}` references it", other.name));
                }
            }
            TelFile::Intent(intent) => {
                if statement_notions(&intent.statement).contains(name) {
                    return Some(format!("{} uses it", intent.id));
                }
                for scenario in &intent.scenarios {
                    if scenario_notions(scenario).contains(name) {
                        return Some(format!("{} uses it", scenario.id));
                    }
                }
            }
            TelFile::Constraint(constraint) => {
                if let Rule::Machine(expr) = &constraint.rule {
                    let mut notions = Default::default();
                    expr_notions(expr, &mut notions);
                    if notions.contains(name) {
                        return Some(format!("{} uses it", constraint.id));
                    }
                }
            }
            TelFile::Bindings(_) => {}
        }
    }
    None
}

fn first_intent_referrer(
    base: &[(RepoPath, TelFile)],
    id: crate::ids::IntentId,
    scenarios: &[crate::ids::ScenarioId],
) -> Option<String> {
    for (_, file) in base {
        match file {
            TelFile::Intent(other) => {
                let relations = [
                    ("refines", &other.refines),
                    ("requires", &other.requires),
                    ("excludes", &other.excludes),
                ];
                for (verb, ids) in relations {
                    if ids.iter().any(|other_id| other_id.node == id) {
                        return Some(format!("{} {verb} it", other.id));
                    }
                }
            }
            TelFile::Constraint(constraint) => {
                if let Scope::Intents(ids) = &constraint.scope
                    && ids.iter().any(|scoped| scoped.node == id)
                {
                    return Some(format!("{} constrains it", constraint.id));
                }
            }
            TelFile::Bindings(bindings) => {
                for binding in bindings {
                    match binding {
                        Binding::Implements { path, intent } if intent.node == id => {
                            return Some(format!("`{path}` implements it"));
                        }
                        Binding::Proves { test, scenario }
                            if scenarios.contains(&scenario.node) =>
                        {
                            return Some(format!("`{test}` proves its scenario {}", scenario.node));
                        }
                        _ => {}
                    }
                }
            }
            TelFile::Notion(_) => {}
        }
    }
    None
}

/// [`apply_ops`] with the staging preconditions dropped: each op simply puts
/// its post-state at its target path, and is therefore a no-op against a
/// base that already shows it.
///
/// Infallible, because with the preconditions gone there is nothing left to
/// refuse: an `add` whose slot is taken *sets* it (the same thing an `edit`
/// does), a `remove` whose slot is already empty leaves it empty, an
/// `accept` is inert. The verdict on the result is [`build_model`]'s alone.
///
/// # Why this exists (D7)
///
/// [`apply_ops`]' three refusals -- add-what-exists, edit/remove-what-does-
/// not, remove-what-is-referenced -- read the base as *the sealed tree*, and
/// they are exactly right there: `telos add notion Invoice` on a project
/// that already has one is a mistake, and saying so early is the point.
///
/// A whole change validated at reconcile time is a different question, and
/// `adopt` is what makes the difference visible. Adopting a hand-deleted
/// `CON-0003.tel` stages `remove constraint CON-0003` -- against a base read
/// from a disk where the file is *already gone*. Adopting a hand-created
/// `Rogue.tel` stages `add notion Rogue` -- against a base that already
/// holds it. Under [`apply_ops`] both would be refused as contradictions,
/// when in fact both describe precisely the state on disk. The base is no
/// longer the sealed tree, so the preconditions no longer mean what they
/// were written to mean.
///
/// What survives untouched is the part that is about the *spec* rather than
/// about the op: [`build_model`] still rejects every dangling reference, so
/// rule 2 of §3.3 is still enforced at reconcile -- an entity removed while
/// something still points at it fails as an unresolvable reference instead
/// of as a named referrer. Only the message differs, and the good message is
/// kept where it does the most good: at staging time, where the mistake was
/// just made. That claim is not left to inspection: `overlay.rs`'s
/// integration tests pin it for a removal [`first_referrer`] would have
/// caught, and `adopt_revert.rs` pins it end to end for a drift that deletes
/// a referenced notion.
pub fn apply_ops_idempotent(
    mut base: Vec<(RepoPath, TelFile)>,
    ops: &[StagedOp],
) -> Vec<(RepoPath, TelFile)> {
    for op in ops {
        // `Accept` seals bytes the model holds no entity for -- inert here,
        // exactly as in `apply_ops`.
        let post = match op {
            StagedOp::Accept { .. } => continue,
            StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => Some(TelFile::Notion(n.clone())),
            StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => Some(TelFile::Intent(i.clone())),
            StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => {
                Some(TelFile::Constraint(c.clone()))
            }
            StagedOp::RemoveNotion(_)
            | StagedOp::RemoveIntent(_)
            | StagedOp::RemoveConstraint(_) => None,
        };

        let path = op.target_path();
        let slot = base.iter().position(|(p, _)| *p == path);
        match (slot, post) {
            (Some(slot), Some(file)) => base[slot].1 = file,
            (Some(slot), None) => {
                base.remove(slot);
            }
            (None, Some(file)) => {
                base.push((path, file));
                base.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
            }
            (None, None) => {}
        }
    }
    base
}

/// The full check a *whole change* must pass: the tree parses,
/// [`apply_ops_idempotent`] puts the delta's post-state on top, and
/// [`build_model`] proves the spec that describes -- whether or not the
/// working tree already shows part of that delta.
///
/// This is what `adopt` validates a plan with, what `reconcile` runs as its
/// fifth gate, and what `telos add|edit|remove` finishes with once the op it
/// is staging has passed [`apply_ops`]'s own preconditions. See
/// [`apply_ops_idempotent`] for why those preconditions cannot be re-run
/// over a complete change.
pub fn validate_ops_idempotent(
    ws: &Workspace,
    ops: &[StagedOp],
) -> Result<TelosModel, Vec<Diagnostic>> {
    let base = parse_base(ws)?;
    build_model(apply_ops_idempotent(base, ops))
}

/// The canonical text of op `idx`'s target, before and after that op --
/// `None` on the side where the entity does not exist.
///
/// The ops before `idx` are replayed first, so an `edit` of something an
/// earlier op of the same change added reports that op's output as its
/// `before`: what a reviewer sees is the delta the op itself introduces, not
/// the delta against the sealed tree.
///
/// Total by construction, because `change diff` (T8) must be able to
/// describe a change file a human hand-edited into something the overlay
/// would refuse: an index past the end, an `accept` op (whose target is a
/// path the model holds no entity for), and a prefix that fails to apply all
/// answer with what is knowable rather than with an error -- the last of
/// those by falling back to the unmodified base.
pub fn op_before_after(
    base: &[(RepoPath, TelFile)],
    ops: &[StagedOp],
    idx: usize,
) -> (Option<String>, Option<String>) {
    let Some(op) = ops.get(idx) else {
        return (None, None);
    };
    let pre = apply_ops(base.to_vec(), &ops[..idx]).unwrap_or_else(|_| base.to_vec());

    let before = find_file(&pre, &op.target_path()).map(emit);
    let after = match op {
        StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => Some(emit_notion(n)),
        StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => Some(emit_intent(i)),
        StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => Some(emit_constraint(c)),
        StagedOp::RemoveNotion(_)
        | StagedOp::RemoveIntent(_)
        | StagedOp::RemoveConstraint(_)
        | StagedOp::Accept { .. } => None,
    };
    (before, after)
}

fn emit(file: &TelFile) -> String {
    crate::emit::emit_file(file)
}
