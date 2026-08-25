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
//! The split exists because `adopt` turns drift into ops describing a tree
//! that already shows them -- `add` of a file that is already there, `remove`
//! of one that is already gone -- so the staging preconditions would refuse
//! the very state they exist to protect. [`apply_ops_idempotent`]'s own doc
//! carries the full argument; the short version is that those refusals are
//! about *the op*, and only the caller staging it is in a position to act on
//! them.
//!
//! Three properties are load-bearing:
//!
//! - **Order is data**. The ops replay sequentially against one
//!   another's output, so `add X` then `remove X` is a different transaction
//!   from `remove X` then `add X`, and the referential-deletion check below is judged at the point
//!   in the sequence where the removal happens. What order does *not* change
//!   is the verdict of the semantic pass: [`build_model`] runs once, on the
//!   state the last op leaves behind.
//! - **A path is an identity** ([`StagedOp::target_path`]). The base is
//!   keyed by repo-relative path, and an entity's path is a function of its
//!   kind and its id, so two ops on one entity always meet on one slot.
//! - **Referential deletion safety is enforced twice, differently.** [`apply_ops`] refuses
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
//! # The third layer: the journal
//!
//! A change carries a *journal* as well as ops -- the runs `telos
//! test` sealed and the binds `telos bind` recorded -- and those lines are
//! bindings the change asserts just as surely as its ops are entities it
//! asserts. [`fold_journal_bindings`] puts them on top of the applied ops,
//! into `telos/bindings.tel`, and it is deliberately a separate step: ops
//! are replayed in order against one another, while the journal is folded
//! once, at the end, into the one file no entity owns. Every caller that
//! needs the spec a change describes runs both, in that order, before
//! [`build_model`] -- which is what makes a journal's `implements`/`proves`
//! resolve, and be checked, exactly like a binding a human wrote.
//!
//! The rules this module does *not* enforce are the ones that need more than
//! the spec tree: no-code-without-telos (the unbound-code gate) needs `telos.toml`'s globs
//! and the working tree, and the red-witness discipline needs test runs and
//! the current blob oids to judge them against. Both belong to reconcile.

use std::collections::BTreeMap;

use crate::config::Config;
use crate::emit::{emit_constraint, emit_intent, emit_notion};
use crate::error::{Diagnostic, ErrorCode, TelosError};
use crate::ids::{ContextId, IntentId, NotionName, RepoPath, ScenarioId};
use crate::model::change::context_bindings_path;
use crate::model::{Binding, Change, Notion, Rule, Scope, StagedOp, TelFile, TelosModel};
use crate::semantic::{build_model, expr_notions, scenario_notions, statement_notions};
use crate::suggest::closest;
use crate::workspace::{BINDINGS_PATH, Workspace};

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

/// Applies configuration operations. They are intentionally not model ops.
pub fn apply_config_ops(base: &Config, ops: &[StagedOp]) -> Config {
    let mut config = base.clone();
    for op in ops {
        if let StagedOp::EditConfig(next) = op {
            config = next.clone();
        }
    }
    config.normalize();
    config
}

/// Every notion of a base, by name -- the map
/// [`crate::payload::intent_from_json`] types a scenario's `fields`
/// against, which for a staged op must include the notions this very change
/// added earlier.
pub fn notions_of(base: &[(RepoPath, TelFile)]) -> BTreeMap<NotionName, Notion> {
    base.iter()
        .filter_map(|(_, file)| match file {
            TelFile::Notion(notion) | TelFile::OwnedNotion { notion, .. } => {
                Some((notion.name.clone(), notion.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The entity a base holds at `path`, if any.
pub fn find_file<'a>(base: &'a [(RepoPath, TelFile)], path: &RepoPath) -> Option<&'a TelFile> {
    base.iter().find(|(p, _)| p == path).map(|(_, file)| file)
}

/// “ unknown intent `INT-9999` ”, with the `closest is …` suffix when one of
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
            ("context", TelFile::Context(context)) => Some(context.id.to_string()),
            ("capability", TelFile::Capability(capability)) => Some(capability.id.to_string()),
            ("notion", TelFile::Notion(n)) => Some(n.name.to_string()),
            ("notion", TelFile::OwnedNotion { owner, notion }) => {
                Some(format!("{}/{}", owner.context, notion.name))
            }
            ("intent", TelFile::Intent(i)) => Some(i.id.to_string()),
            ("intent", TelFile::OwnedIntent { intent, .. }) => Some(intent.id.to_string()),
            ("constraint", TelFile::Constraint(c)) => Some(c.id.to_string()),
            ("constraint", TelFile::OwnedConstraint { constraint, .. }) => {
                Some(constraint.id.to_string())
            }
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
/// - **remove** of something still referenced (the referential-deletion check) --
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
        StagedOp::Accept { .. } | StagedOp::EditConfig(_) => Ok(()),

        StagedOp::AddContext(context) => add(base, op, TelFile::Context(context.clone())),
        StagedOp::AddCapability(capability) => {
            add(base, op, TelFile::Capability(capability.clone()))
        }
        StagedOp::AddOwnedNotion { owner, notion } => add(
            base,
            op,
            TelFile::OwnedNotion {
                owner: owner.clone(),
                notion: notion.clone(),
            },
        ),
        StagedOp::AddOwnedIntent { owner, intent } => add(
            base,
            op,
            TelFile::OwnedIntent {
                owner: owner.clone(),
                intent: intent.clone(),
            },
        ),
        StagedOp::AddOwnedConstraint { owner, constraint } => add(
            base,
            op,
            TelFile::OwnedConstraint {
                owner: owner.clone(),
                constraint: constraint.clone(),
            },
        ),

        StagedOp::EditContext(context) => edit(base, op, TelFile::Context(context.clone())),
        StagedOp::EditCapability(capability) => {
            edit(base, op, TelFile::Capability(capability.clone()))
        }
        StagedOp::EditOwnedNotion { owner, notion } => edit(
            base,
            op,
            TelFile::OwnedNotion {
                owner: owner.clone(),
                notion: notion.clone(),
            },
        ),
        StagedOp::EditOwnedIntent { owner, intent } => edit(
            base,
            op,
            TelFile::OwnedIntent {
                owner: owner.clone(),
                intent: intent.clone(),
            },
        ),
        StagedOp::EditOwnedConstraint { owner, constraint } => edit(
            base,
            op,
            TelFile::OwnedConstraint {
                owner: owner.clone(),
                constraint: constraint.clone(),
            },
        ),
        StagedOp::EditContextMap(map) => edit(base, op, TelFile::ContextMap(map.clone())),

        StagedOp::MoveNotion { to, notion, .. } => move_entity(
            base,
            op,
            TelFile::OwnedNotion {
                owner: to.clone(),
                notion: notion.clone(),
            },
        ),
        StagedOp::MoveIntent { to, intent, .. } => move_entity(
            base,
            op,
            TelFile::OwnedIntent {
                owner: to.clone(),
                intent: intent.clone(),
            },
        ),
        StagedOp::MoveConstraint { to, constraint, .. } => move_entity(
            base,
            op,
            TelFile::OwnedConstraint {
                owner: to.clone(),
                constraint: constraint.clone(),
            },
        ),

        StagedOp::AddNotion(n) => add(base, op, TelFile::Notion(n.clone())),
        StagedOp::AddIntent(i) => add(base, op, TelFile::Intent(i.clone())),
        StagedOp::AddConstraint(c) => add(base, op, TelFile::Constraint(c.clone())),

        StagedOp::EditNotion(n) => edit(base, op, TelFile::Notion(n.clone())),
        StagedOp::EditIntent(i) => edit(base, op, TelFile::Intent(i.clone())),
        StagedOp::EditConstraint(c) => edit(base, op, TelFile::Constraint(c.clone())),

        StagedOp::RemoveContext(_)
        | StagedOp::RemoveCapability(_)
        | StagedOp::RemoveOwnedNotion { .. }
        | StagedOp::RemoveOwnedIntent { .. }
        | StagedOp::RemoveOwnedConstraint { .. }
        | StagedOp::RemoveNotion(_)
        | StagedOp::RemoveIntent(_)
        | StagedOp::RemoveConstraint(_) => remove(base, op),
    }
}

fn move_entity(
    base: &mut Vec<(RepoPath, TelFile)>,
    op: &StagedOp,
    file: TelFile,
) -> Result<(), TelosError> {
    let source = op.source_path().expect("move has a source path");
    let Some(slot) = base.iter().position(|(path, _)| *path == source) else {
        return Err(unknown_entity(base, op.entity(), &op.key()));
    };
    if base.iter().any(|(path, _)| *path == op.target_path()) {
        return Err(TelosError::new(
            ErrorCode::TelosIntegrityViolation,
            format!("move target `{}` already exists", op.target_path()),
        ));
    }
    base.remove(slot);
    base.push((op.target_path(), file));
    base.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
    Ok(())
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

/// Drops what the base holds at `op`'s target path, then checks the state the
/// removal leaves behind for dangling references.
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

/// The first thing in `base` that still points at `removed`,
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
        TelFile::Notion(notion) | TelFile::OwnedNotion { notion, .. } => {
            first_notion_referrer(base, &notion.name)
        }
        TelFile::Intent(intent) | TelFile::OwnedIntent { intent, .. } => {
            let scenarios: Vec<_> = intent.scenarios.iter().map(|s| s.id).collect();
            first_intent_referrer(base, intent.id, &scenarios)
        }
        TelFile::Constraint(_)
        | TelFile::OwnedConstraint { .. }
        | TelFile::Bindings(_)
        | TelFile::ContextBindings { .. }
        | TelFile::Context(_)
        | TelFile::Capability(_)
        | TelFile::ContextMap(_) => None,
    }
}

fn first_notion_referrer(base: &[(RepoPath, TelFile)], name: &NotionName) -> Option<String> {
    for (_, file) in base {
        match file {
            TelFile::Notion(other) | TelFile::OwnedNotion { notion: other, .. } => {
                let targeted = other.rels.iter().any(|rel| rel.target.node == *name)
                    || other.attrs.iter().any(|attr| match &attr.ty {
                        crate::model::AttrType::Ref(target) => target == name,
                        _ => false,
                    });
                if targeted {
                    return Some(format!("notion `{}` references it", other.name));
                }
            }
            TelFile::Intent(intent) | TelFile::OwnedIntent { intent, .. } => {
                if statement_notions(&intent.statement).contains(name) {
                    return Some(format!("{} uses it", intent.id));
                }
                for scenario in &intent.scenarios {
                    if scenario_notions(scenario).contains(name) {
                        return Some(format!("{} uses it", scenario.id));
                    }
                }
            }
            TelFile::Constraint(constraint) | TelFile::OwnedConstraint { constraint, .. } => {
                if let Rule::Machine(expr) = &constraint.rule {
                    let mut notions = Default::default();
                    expr_notions(expr, &mut notions);
                    if notions.contains(name) {
                        return Some(format!("{} uses it", constraint.id));
                    }
                }
            }
            TelFile::Bindings(_)
            | TelFile::ContextBindings { .. }
            | TelFile::Context(_)
            | TelFile::Capability(_)
            | TelFile::ContextMap(_) => {}
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
            TelFile::Intent(other) | TelFile::OwnedIntent { intent: other, .. } => {
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
            TelFile::Constraint(constraint) | TelFile::OwnedConstraint { constraint, .. } => {
                if let Scope::Intents(ids) = &constraint.scope
                    && ids.iter().any(|scoped| scoped.node == id)
                {
                    return Some(format!("{} constrains it", constraint.id));
                }
            }
            TelFile::Bindings(bindings) | TelFile::ContextBindings { bindings, .. } => {
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
            TelFile::Notion(_)
            | TelFile::OwnedNotion { .. }
            | TelFile::Context(_)
            | TelFile::Capability(_)
            | TelFile::ContextMap(_) => {}
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
/// # Why this exists
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
/// referential deletion safety is still enforced at reconcile -- an entity removed while
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
            StagedOp::Accept { .. } | StagedOp::EditConfig(_) => continue,
            StagedOp::AddContext(context) | StagedOp::EditContext(context) => {
                Some(TelFile::Context(context.clone()))
            }
            StagedOp::AddCapability(capability) | StagedOp::EditCapability(capability) => {
                Some(TelFile::Capability(capability.clone()))
            }
            StagedOp::AddOwnedNotion { owner, notion }
            | StagedOp::EditOwnedNotion { owner, notion } => Some(TelFile::OwnedNotion {
                owner: owner.clone(),
                notion: notion.clone(),
            }),
            StagedOp::AddOwnedIntent { owner, intent }
            | StagedOp::EditOwnedIntent { owner, intent } => Some(TelFile::OwnedIntent {
                owner: owner.clone(),
                intent: intent.clone(),
            }),
            StagedOp::AddOwnedConstraint { owner, constraint }
            | StagedOp::EditOwnedConstraint { owner, constraint } => {
                Some(TelFile::OwnedConstraint {
                    owner: owner.clone(),
                    constraint: constraint.clone(),
                })
            }
            StagedOp::EditContextMap(map) => Some(TelFile::ContextMap(map.clone())),
            StagedOp::MoveNotion { to, notion, .. } => Some(TelFile::OwnedNotion {
                owner: to.clone(),
                notion: notion.clone(),
            }),
            StagedOp::MoveIntent { to, intent, .. } => Some(TelFile::OwnedIntent {
                owner: to.clone(),
                intent: intent.clone(),
            }),
            StagedOp::MoveConstraint { to, constraint, .. } => Some(TelFile::OwnedConstraint {
                owner: to.clone(),
                constraint: constraint.clone(),
            }),
            StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => Some(TelFile::Notion(n.clone())),
            StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => Some(TelFile::Intent(i.clone())),
            StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => {
                Some(TelFile::Constraint(c.clone()))
            }
            StagedOp::RemoveContext(_)
            | StagedOp::RemoveCapability(_)
            | StagedOp::RemoveOwnedNotion { .. }
            | StagedOp::RemoveOwnedIntent { .. }
            | StagedOp::RemoveOwnedConstraint { .. }
            | StagedOp::RemoveNotion(_)
            | StagedOp::RemoveIntent(_)
            | StagedOp::RemoveConstraint(_) => None,
        };

        if let Some(source) = op.source_path()
            && let Some(slot) = base.iter().position(|(path, _)| *path == source)
        {
            base.remove(slot);
        }

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

/// Folds a change's journal into the spec state as bindings, extending
/// or creating the bindings file of the context that owns the referenced
/// intent or scenario.
///
/// This is what makes `bindings.tel` a *derived* file: `telos bind` and the
/// first green run of `telos test` write journal lines, never bindings, and
/// the sealed `bindings.tel` on disk stays untouched until a reconcile folds
/// the journal into it. Every caller that must see the spec a change
/// describes -- reconcile's gates, `telos context`'s pack -- folds before
/// building a model, so a journal's `implements`/`proves` resolve, and are
/// resolved, exactly like a binding a human wrote.
///
/// Deduplication is against what the base already holds and is deliberately
/// **span-insensitive**: a binding parsed from `bindings.tel` carries the
/// span it was read at, one folded from a journal line carries the zero span
/// ([`Change::journal_bindings`]), so structural equality would keep both and
/// the emitter would write the line twice. Two bindings are the same line
/// here when they are the same kind, over the same path (or the same test
/// locator, `name` included) and the same id -- which is exactly what
/// [`crate::emit::emit_bindings`] would render identically.
///
/// Order follows the file: what the base held stays first, in its own order,
/// and the journal's additions are appended in journal order. Nothing is
/// sorted, because sorting bindings is the emitter's job and its alone.
pub fn fold_journal_bindings(
    mut files: Vec<(RepoPath, TelFile)>,
    change: &Change,
) -> Vec<(RepoPath, TelFile)> {
    let folded = change.journal_bindings();
    if folded.is_empty() {
        return files;
    }

    let (intent_contexts, scenario_contexts) = binding_owners(&files);
    for binding in folded {
        let context = match &binding {
            Binding::Implements { intent, .. } => intent_contexts.get(&intent.node),
            Binding::Proves { scenario, .. } => scenario_contexts.get(&scenario.node),
        };
        // Unowned forms only exist in isolated legacy unit fixtures. A real
        // workspace rejects their paths before reaching the overlay.
        let path = context
            .map(context_bindings_path)
            .unwrap_or_else(|| RepoPath::new(BINDINGS_PATH));
        let slot = files.iter().position(|(candidate, file)| {
            *candidate == path
                && matches!(
                    (context, file),
                    (Some(_), TelFile::ContextBindings { .. }) | (None, TelFile::Bindings(_))
                )
        });
        let mut bindings = match slot {
            Some(slot) => match files.remove(slot).1 {
                TelFile::ContextBindings { bindings, .. } | TelFile::Bindings(bindings) => bindings,
                other => unreachable!("the slot was matched as bindings, got {other:?}"),
            },
            None => Vec::new(),
        };
        if !bindings.iter().any(|held| same_binding(held, &binding)) {
            bindings.push(binding);
        }
        let file = match context {
            Some(context) => TelFile::ContextBindings {
                context: context.clone(),
                bindings,
            },
            None => TelFile::Bindings(bindings),
        };
        files.push((path, file));
    }
    files.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
    files
}

fn binding_owners(
    files: &[(RepoPath, TelFile)],
) -> (
    BTreeMap<IntentId, ContextId>,
    BTreeMap<ScenarioId, ContextId>,
) {
    let mut intents = BTreeMap::new();
    let mut scenarios = BTreeMap::new();
    for (_, file) in files {
        if let TelFile::OwnedIntent { owner, intent } = file {
            intents.insert(intent.id, owner.context.clone());
            for scenario in &intent.scenarios {
                scenarios.insert(scenario.id, owner.context.clone());
            }
        }
    }
    (intents, scenarios)
}

/// Whether two bindings are the same line, ignoring the spans they were
/// built with -- see [`fold_journal_bindings`].
fn same_binding(a: &Binding, b: &Binding) -> bool {
    match (a, b) {
        (
            Binding::Implements { path, intent },
            Binding::Implements {
                path: other_path,
                intent: other_intent,
            },
        ) => path == other_path && intent.node == other_intent.node,
        (
            Binding::Proves { test, scenario },
            Binding::Proves {
                test: other_test,
                scenario: other_scenario,
            },
        ) => test == other_test && scenario.node == other_scenario.node,
        _ => false,
    }
}

/// The canonical text of op `idx`'s target, before and after that op --
/// `None` on the side where the entity does not exist.
///
/// The ops before `idx` are replayed first, so an `edit` of something an
/// earlier op of the same change added reports that op's output as its
/// `before`: what a reviewer sees is the delta the op itself introduces, not
/// the delta against the sealed tree.
///
/// Total by construction, because `change diff` must be able to
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

    let before_path = op.source_path().unwrap_or_else(|| op.target_path());
    let before = find_file(&pre, &before_path).map(emit);
    let after = match op {
        StagedOp::AddContext(context) | StagedOp::EditContext(context) => {
            Some(crate::emit::emit_context(context))
        }
        StagedOp::AddCapability(capability) | StagedOp::EditCapability(capability) => {
            Some(crate::emit::emit_capability(capability))
        }
        StagedOp::AddOwnedNotion { owner, notion }
        | StagedOp::EditOwnedNotion { owner, notion } => {
            Some(crate::emit::emit_owned_notion(owner, notion))
        }
        StagedOp::AddOwnedIntent { owner, intent }
        | StagedOp::EditOwnedIntent { owner, intent } => {
            Some(crate::emit::emit_owned_intent(owner, intent))
        }
        StagedOp::AddOwnedConstraint { owner, constraint }
        | StagedOp::EditOwnedConstraint { owner, constraint } => Some(
            crate::emit::emit_owned_constraint(owner.as_ref(), constraint),
        ),
        StagedOp::EditContextMap(map) => Some(crate::emit::emit_context_map(map)),
        StagedOp::MoveNotion { to, notion, .. } => Some(crate::emit::emit_owned_notion(to, notion)),
        StagedOp::MoveIntent { to, intent, .. } => Some(crate::emit::emit_owned_intent(to, intent)),
        StagedOp::MoveConstraint { to, constraint, .. } => {
            Some(crate::emit::emit_owned_constraint(to.as_ref(), constraint))
        }
        StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => Some(emit_notion(n)),
        StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => Some(emit_intent(i)),
        StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => Some(emit_constraint(c)),
        StagedOp::RemoveContext(_)
        | StagedOp::RemoveCapability(_)
        | StagedOp::RemoveOwnedNotion { .. }
        | StagedOp::RemoveOwnedIntent { .. }
        | StagedOp::RemoveOwnedConstraint { .. }
        | StagedOp::RemoveNotion(_)
        | StagedOp::RemoveIntent(_)
        | StagedOp::RemoveConstraint(_)
        | StagedOp::Accept { .. } => None,
        StagedOp::EditConfig(_) => None,
    };
    (before, after)
}

fn emit(file: &TelFile) -> String {
    crate::emit::emit_file(file)
}

#[cfg(test)]
mod tests {
    //! [`fold_journal_bindings`] only -- everything else in this module is
    //! covered against a real spec tree in `crates/telos-core/tests/
    //! overlay.rs`, while the fold is a pure function of a base and a
    //! change, and its interesting cases are all about what the base
    //! already holds.

    use super::*;
    use crate::ids::{IntentId, Owner, ScenarioId};
    use crate::model::change::fixtures::{implementing_change, int_0017, invoice};
    use crate::model::{Binding, TestRef};
    use crate::span::{Sp, Span};

    fn bindings_of(files: &[(RepoPath, TelFile)]) -> Vec<Binding> {
        files
            .iter()
            .find(|(path, _)| path.as_str() == BINDINGS_PATH)
            .map(|(_, file)| match file {
                TelFile::Bindings(bindings) => bindings.clone(),
                other => panic!("`{BINDINGS_PATH}` holds {other:?}"),
            })
            .unwrap_or_else(|| panic!("no `{BINDINGS_PATH}` entry in {files:?}"))
    }

    /// `implements "src/billing.rs" -> INT-0001`, at whatever span -- the
    /// span is what the deduplication must *not* look at.
    fn implements(span: Span) -> Binding {
        Binding::Implements {
            path: RepoPath::new("src/billing.rs"),
            intent: Sp {
                node: IntentId(1),
                span,
            },
        }
    }

    /// The `proves` the example's green run folds to.
    fn proves() -> Binding {
        Binding::Proves {
            test: TestRef {
                path: RepoPath::new("tests/billing.rs"),
                name: Some("scn_0001_a_full_payment_settles_the_invoice".to_string()),
            },
            scenario: Sp {
                node: ScenarioId(1),
                span: Span::default(),
            },
        }
    }

    fn notion_entry() -> (RepoPath, TelFile) {
        (
            RepoPath::new("telos/notions/Invoice.tel"),
            TelFile::Notion(invoice()),
        )
    }

    #[test]
    fn journal_bindings_are_folded_into_the_referenced_intents_context() {
        let mut intent = int_0017();
        intent.id = IntentId(1);
        intent.scenarios[0].id = ScenarioId(1);
        let context = ContextId::new("billing").unwrap();
        let owner = Owner::capability("billing/invoicing".parse().unwrap());
        let base = vec![(
            crate::model::change::owned_intent_path(&owner, intent.id),
            TelFile::OwnedIntent { owner, intent },
        )];

        let files = fold_journal_bindings(base, &implementing_change());
        let path = context_bindings_path(&context);
        let bindings = files
            .iter()
            .find_map(|(candidate, file)| match file {
                TelFile::ContextBindings {
                    context: held,
                    bindings,
                } if candidate == &path && held == &context => Some(bindings),
                _ => None,
            })
            .expect("the context owns a derived bindings file");

        assert_eq!(bindings, &vec![proves(), implements(Span::default())]);
        assert!(
            files
                .iter()
                .all(|(candidate, _)| candidate.as_str() != BINDINGS_PATH)
        );
    }

    #[test]
    fn a_base_with_no_bindings_file_gains_one() {
        let files = fold_journal_bindings(vec![notion_entry()], &implementing_change());

        assert_eq!(
            bindings_of(&files),
            vec![proves(), implements(Span::default())]
        );
        // And the entry lands in path order, like every other base entry.
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec![BINDINGS_PATH, "telos/notions/Invoice.tel"]);
    }

    #[test]
    fn an_existing_bindings_file_is_extended_not_replaced() {
        let held = Binding::Proves {
            test: TestRef {
                path: RepoPath::new("tests/other.rs"),
                name: None,
            },
            scenario: Sp {
                node: ScenarioId(107),
                span: Span { start: 12, end: 20 },
            },
        };
        let base = vec![(
            RepoPath::new(BINDINGS_PATH),
            TelFile::Bindings(vec![held.clone()]),
        )];

        let files = fold_journal_bindings(base, &implementing_change());

        assert_eq!(
            bindings_of(&files),
            vec![held, proves(), implements(Span::default())],
            "what the file held stays, first, and the journal's lines follow"
        );
        assert_eq!(files.len(), 1, "one bindings entry, not two");
    }

    #[test]
    fn a_binding_the_file_already_holds_is_not_folded_twice_whatever_its_span() {
        // `telos bind` may journal a pair that `bindings.tel` already carries
        // (a re-bind, or a second change
        // touching the same file). The parsed binding carries the span it
        // was read at, the folded one carries the zero span -- structural
        // equality would keep both and the emitter would write the line
        // twice.
        let sealed = implements(Span { start: 40, end: 48 });
        let base = vec![(
            RepoPath::new(BINDINGS_PATH),
            TelFile::Bindings(vec![sealed.clone()]),
        )];

        let files = fold_journal_bindings(base, &implementing_change());

        assert_eq!(bindings_of(&files), vec![sealed, proves()]);
    }

    #[test]
    fn a_change_with_no_journal_leaves_the_base_exactly_as_it_was() {
        // Including the case that matters most: a project with no
        // `bindings.tel` at all must not grow one because a change with an
        // empty journal reconciled.
        let mut change = implementing_change();
        change.journal = Vec::new();
        let base = vec![notion_entry()];

        assert_eq!(fold_journal_bindings(base.clone(), &change), base);
    }
}
