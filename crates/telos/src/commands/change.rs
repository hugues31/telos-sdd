//! `telos change <open|list|abandon|diff|approve|reconcile>`: the whole
//! lifecycle of a staged transaction, from the empty change to the seal that
//! closes it.
//!
//! Two rules shape every function here:
//!
//! - **The store writes, never this module.** A change file's bytes come
//!   from [`write_change`] (hence from `emit_change`), its content from
//!   [`read_change`] (hence from `parse_change_file`), and its deletion from
//!   [`delete_change`]. Nothing here formats or decodes a change itself.
//! - **`open` and `approve` are gated, `diff` is not (D17).** Opening a
//!   change or freezing its digest both stage a review against the sealed
//!   base, so both need that base to still be the sealed one; `list`,
//!   `abandon` and `diff` read, or clean up, and a drifted project is
//!   exactly when a caller needs them most.

use clap::Subcommand;
use serde_json::{Value, json};

use telos_core::changes::{delete_change, open_change_infos, read_change, write_change};
use telos_core::counters::write_counters;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::git::GitRepo;
use telos_core::ids::ChangeId;
use telos_core::model::{Change, ChangeStatus, StagedOp};
use telos_core::overlay::{apply_config_ops, op_before_after, parse_base};
use telos_core::reconcile::{reconcile_change, reconcile_full};
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, allocator, diagnostics_to_error, project, require_no_unclaimed_drift};
use crate::envelope::{CmdResult, Outcome};

/// The six verbs of `change`: `open`, `list`, `abandon`, `diff`, `approve`
/// (T8) and `reconcile` (T10) -- the whole change lifecycle, from allocating
/// an id to the transaction that spends it.
#[derive(Debug, Clone, Subcommand)]
pub enum ChangeCommand {
    /// Open a new, empty change and allocate its id.
    Open {
        /// Why this change exists, in one sentence.
        motivation: String,
    },
    /// List every change the project currently holds.
    List,
    /// Abandon a change, deleting its file.
    Abandon {
        /// The change to abandon (`CHG-0001`).
        id: String,
    },
    /// Report a change's staged ops against the sealed base, one before/
    /// after pair per op.
    Diff {
        /// The change to inspect (`CHG-0001`).
        id: String,
    },
    /// Freeze a change's ops digest, approving it for reconcile.
    Approve {
        /// The change to approve (`CHG-0001`).
        id: String,
    },
    /// Apply an approved change (write its spec files, reseal, close it),
    /// or reseal the whole project with `--full`.
    Reconcile {
        /// The change to reconcile (`CHG-0001`).
        #[arg(required_unless_present = "full", conflicts_with = "full")]
        id: Option<String>,
        /// Re-prove the whole project and reseal it, ignoring the current
        /// lock. Takes no change id.
        #[arg(long)]
        full: bool,
    },
}

pub fn run(ctx: &Ctx, command: &ChangeCommand) -> CmdResult {
    match command {
        ChangeCommand::Open { motivation } => open(ctx, motivation),
        ChangeCommand::List => list(ctx),
        ChangeCommand::Abandon { id } => abandon(ctx, id),
        ChangeCommand::Diff { id } => diff(ctx, id),
        ChangeCommand::Approve { id } => approve(ctx, id),
        // `conflicts_with` and `required_unless_present` leave exactly two
        // shapes: an id alone, or `--full` alone.
        ChangeCommand::Reconcile { id: Some(id), .. } => reconcile(ctx, id),
        ChangeCommand::Reconcile { .. } => reconcile_full_project(ctx),
    }
}

// --- change open ------------------------------------------------------------

/// Allocates the next change id, writes the empty change, persists the
/// counters.
///
/// The order of the two writes is the reconcile order of D6 in miniature --
/// the change file first, `counters.toml` last -- and it is the safe one:
/// should the process die between them, the next allocation rescans the
/// floors, sees `CHG-0001` on disk, and starts past it anyway (D4). The
/// reverse order would leave a counter claiming an id that no file backs,
/// which is harmless too, but only by luck rather than by design.
fn open(ctx: &Ctx, motivation: &str) -> CmdResult {
    let project = project(ctx)?;
    require_no_unclaimed_drift(&project)?;

    let mut alloc = allocator(&project.ws, &project.lock)?;
    let id = alloc.next_change();

    let change = Change {
        id,
        motivation: motivation.to_string(),
        status: ChangeStatus::Open,
        approved_digest: None,
        ops: Vec::new(),
        journal: Vec::new(),
    };
    write_change(&project.ws, &change)?;
    write_counters(&project.ws, &alloc.counters())?;

    Ok(Outcome {
        result: json!({ "id": id, "status": ChangeStatus::Open.as_str() }),
        human: format!("opened {id}"),
        next_actions: vec![format!("telos add intent --change {id}")],
    })
}

// --- change list ------------------------------------------------------------

/// Every change the store holds, with the motivation that only the file
/// itself carries.
///
/// `id`, `status` and `obligations` come from [`open_change_infos`], the
/// same best-effort scan `status` reports from, so the two commands can
/// never disagree about what is open. `motivation` is not part of that scan
/// (no claim-aware caller needs it), so it is read here, best-effort in the
/// same spirit: an unparseable file keeps its entry, its `open` status and
/// its repair obligation, and reports an empty motivation rather than
/// inventing one or taking the whole listing down (D15).
fn list(ctx: &Ctx) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let infos = open_change_infos(&ws)?;

    let mut changes = Vec::new();
    let mut lines = Vec::new();
    for info in &infos {
        let motivation = read_change(&ws, info.id)
            .map(|change| change.motivation)
            .unwrap_or_default();
        lines.push(format!("{} {} {motivation}", info.id, info.status.as_str()));
        changes.push(json!({
            "id": info.id,
            "status": info.status.as_str(),
            "motivation": motivation,
            "obligations": info.obligations,
        }));
    }

    let human = if lines.is_empty() {
        "no open changes".to_string()
    } else {
        lines.join("\n")
    };

    Ok(Outcome {
        result: json!({ "changes": changes }),
        human,
        next_actions: Vec::new(),
    })
}

// --- change abandon ---------------------------------------------------------

/// Deletes a change's file, after reading it once.
///
/// The read is not decoration: it is what turns an id the store does not
/// hold into [`read_change`]'s «unknown change `CHG-9999`» (with its
/// nearest-id hint) *before* anything is deleted, and what refuses to
/// silently drop a file whose id was mistyped. Not gated on drift (D17): a
/// change is abandonable whatever the working tree looks like -- it is one
/// of the two ways out of a mess, not more mutation of the spec.
fn abandon(ctx: &Ctx, id: &str) -> CmdResult {
    // The argument is validated before anything is discovered, the same
    // order `show` uses: a malformed id is the caller's mistake, and saying
    // so does not require a workspace to exist.
    let id = parse_change_id(id)?;
    let ws = Workspace::discover(&ctx.cwd)?;

    read_change(&ws, id)?;
    delete_change(&ws, id)?;

    Ok(Outcome {
        result: json!({ "id": id, "status": ChangeStatus::Abandoned.as_str() }),
        human: format!("abandoned {id}"),
        next_actions: Vec::new(),
    })
}

// --- change diff -------------------------------------------------------------

/// Reports a change's staged ops against the sealed base: one before/after
/// pair per op, the current ops digest, the frozen `approved_digest` (if
/// any), and whether the two disagree (`stale`, [`Change::is_stale`]).
///
/// Read-only and never gated on drift (D17): a change's own delta is judged
/// against `telos/`'s spec files as they parse right now, whatever state
/// the rest of the project is in -- exactly the moment a caller most needs
/// to see it. [`parse_base`] only requires the base to *parse*, not to
/// build a coherent model on its own (that is the overlay's job, run at
/// staging time and at reconcile), so a base with an unrelated hole still
/// answers here.
fn diff(ctx: &Ctx, id: &str) -> CmdResult {
    let id = parse_change_id(id)?;
    let ws = Workspace::discover(&ctx.cwd)?;
    let change = read_change(&ws, id)?;
    let base = parse_base(&ws).map_err(diagnostics_to_error)?;

    let digest = change.ops_digest();
    let stale = change.is_stale();

    let mut ops = Vec::with_capacity(change.ops.len());
    let mut human_ops = Vec::with_capacity(change.ops.len());
    for (i, op) in change.ops.iter().enumerate() {
        let (before, after) = match op {
            StagedOp::EditConfig(config) => (
                Some(telos_core::emit::emit_config(&apply_config_ops(
                    &ws.config,
                    &change.ops[..i],
                ))?),
                Some(telos_core::emit::emit_config(config)?),
            ),
            _ => op_before_after(&base, &change.ops, i),
        };
        ops.push(json!({
            "n": i + 1,
            "op": op.verb(),
            "entity": op.entity(),
            "key": op.key(),
            "before": before,
            "after": after,
        }));
        human_ops.push(op_human(i + 1, op, &before, &after));
    }

    let mut human = vec![format!(
        "{} {} digest={digest} approved={} stale={stale}",
        change.id,
        change.status.as_str(),
        change.approved_digest.as_deref().unwrap_or("none"),
    )];
    human.extend(human_ops);

    Ok(Outcome {
        result: json!({
            "id": change.id,
            "status": change.status.as_str(),
            "digest": digest,
            "approved_digest": change.approved_digest,
            "stale": stale,
            "ops": ops,
        }),
        human: human.join("\n\n"),
        next_actions: diff_next_actions(&change, stale),
    })
}

/// One `#N verb entity key` section, terse and readable: the header line,
/// then a `before:`/`after:` block each, `(none)` where [`op_before_after`]
/// answered `None`.
fn op_human(n: usize, op: &StagedOp, before: &Option<String>, after: &Option<String>) -> String {
    let header = format!("#{n} {} {} {}", op.verb(), op.entity(), op.key());
    let block = |label: &str, text: &Option<String>| match text {
        Some(text) => format!("{label}:\n{text}"),
        None => format!("{label}: (none)"),
    };
    format!(
        "{header}\n{}\n{}",
        block("before", before),
        block("after", after)
    )
}

/// `change diff`'s `next_actions`: what to do about the state it just
/// reported.
///
/// `open`/`drafted` both still need a review (D16 -- `open` cannot reach
/// here with staged ops, but the case costs nothing to cover uniformly). An
/// `approved`/`implementing` change whose digest still matches is ready for
/// `reconcile`; one that has gone stale (staged into after approval, D3)
/// needs a fresh `approve` before anything else, which is also what
/// re-approving does (idempotent, D16).
fn diff_next_actions(change: &Change, stale: bool) -> Vec<String> {
    match change.status {
        ChangeStatus::Open | ChangeStatus::Drafted => {
            vec![format!("telos change approve {}", change.id)]
        }
        ChangeStatus::Approved | ChangeStatus::Implementing if stale => {
            vec![format!("telos change approve {}", change.id)]
        }
        ChangeStatus::Approved | ChangeStatus::Implementing => {
            vec![format!("telos change reconcile {}", change.id)]
        }
        // Unreachable in practice -- an abandoned change's file is gone, so
        // `read_change` above would already have refused with `unknown
        // change`. Covered for exhaustiveness, not for a real caller.
        ChangeStatus::Abandoned => Vec::new(),
    }
}

// --- change approve ----------------------------------------------------------

/// Freezes `change`'s ops digest (D3): the review a `reconcile` will later
/// check its base against.
///
/// Gated on drift (D17), like `open`: approving is a judgement about the
/// staged delta *against the sealed base*, and that judgement is void if
/// the base is no longer the sealed one. Requires at least one staged op --
/// there is nothing to approve otherwise, and an `open` change (zero ops)
/// can never pass this, so `approve` only ever moves a change out of
/// `drafted` or re-confirms one already `approved`/`implementing` (D16:
/// idempotent, recalculating the digest every time).
///
/// D13: an `implementing` change re-approved *stays* `implementing`. Two
/// reasons, and either alone would settle it. The grammar's: a change with a
/// journal must be `implementing` (`parse_change_file`), so writing
/// `approved` over one would produce a file nothing can read back. The
/// protocol's: the journal is evidence that implementation has begun, and
/// re-reviewing the delta does not un-begin it. What the re-approval does
/// move is the digest -- the freshly staged ops are what was just reviewed
/// -- while the witnesses already recorded stay judged by the reconcile's
/// own per-scenario, per-oid gate, never by the digest.
fn approve(ctx: &Ctx, id: &str) -> CmdResult {
    let id = parse_change_id(id)?;
    let project = project(ctx)?;
    require_no_unclaimed_drift(&project)?;

    let mut change = read_change(&project.ws, id)?;
    if change.ops.is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosChangeStateInvalid,
            format!("change {id} has no staged operations"),
        )
        .hint("stage operations with telos add|edit|remove first"));
    }

    let status = match change.status {
        ChangeStatus::Implementing => ChangeStatus::Implementing,
        _ => ChangeStatus::Approved,
    };
    change.status = status;
    change.approved_digest = Some(change.ops_digest());
    write_change(&project.ws, &change)?;

    let digest = change.approved_digest.expect("just set above");
    Ok(Outcome {
        result: json!({ "id": change.id, "digest": digest, "status": status.as_str() }),
        human: format!("{id}: approved, digest {digest}"),
        next_actions: vec![format!("telos change reconcile {id}")],
    })
}

// --- change reconcile --------------------------------------------------------

/// Applies an approved change: [`reconcile_change`] runs every gate and,
/// only if all of them pass, writes the spec files, the new seal, and the
/// change file's deletion (D6).
///
/// Deliberately *not* wrapped in [`require_no_unclaimed_drift`], unlike
/// `open` and `approve`. The drift gate is the engine's own first gate here
/// (D5): it admits the drift of the paths this very change claims -- which
/// it is about to overwrite -- and names the offending paths when it
/// refuses, which the shared CLI gate cannot do. Running both would only
/// mean the same verdict reported twice, the less informative one first.
///
/// `result.full` is `false` here by construction: clap refuses an id and
/// `--full` together, so this function is only ever reached without it.
fn reconcile(ctx: &Ctx, id: &str) -> CmdResult {
    let id = parse_change_id(id)?;
    let project = project(ctx)?;
    let change = read_change(&project.ws, id)?;

    let outcome = reconcile_change(
        &project.ws,
        &project.git,
        &project.lock,
        &change,
        &project.changes,
    )?;

    let human = format!(
        "reconciled {id}: {} op(s) applied, {} check(s), {} test(s)",
        outcome.ops_applied, outcome.checks_run, outcome.tests_run
    );
    Ok(Outcome {
        result: json!({
            "id": id,
            "full": false,
            "ops_applied": outcome.ops_applied,
            "checks_run": outcome.checks_run,
            "tests_run": outcome.tests_run,
            "witness_warnings": outcome.witness_warnings,
        }),
        human: with_warnings(human, &outcome.witness_warnings),
        next_actions: vec!["telos status".to_string()],
    })
}

/// Appends one `warning: …` line per advisory witness verdict to a
/// reconcile's human output (D7).
///
/// Human mode is where an `advisory` project's TDD debt has to be visible at
/// all: `witness_warnings` carries it in `--json`, and a reconcile that
/// printed only its counters would let the same debt accumulate silently
/// run after run. Empty in every other case, which is the ordinary one, and
/// then the line is exactly what M2 printed.
fn with_warnings(mut human: String, warnings: &[String]) -> String {
    for warning in warnings {
        human.push_str("\nwarning: ");
        human.push_str(warning);
    }
    human
}

/// `telos change reconcile --full`: re-prove the whole project and reseal
/// it (D12), whatever the current `telos.lock` says -- or fails to say.
///
/// Deliberately does *not* go through [`project`]: that preamble requires a
/// readable lock and computes a state against it, and this command exists
/// precisely for the projects where neither is possible -- a lock left
/// conflicted by a merge, or a spec tree that was never sealed at all. The
/// workspace and the repository are all it needs, and it re-proves
/// everything it seals.
///
/// `result.id` is `null` rather than absent: the envelope's `result` shape
/// is one shape per command (Annex E), and a caller reading `id` should
/// find the key with nothing in it rather than have to know that this one
/// invocation omits it.
fn reconcile_full_project(ctx: &Ctx) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let git = GitRepo::discover(&ctx.cwd)?;

    let outcome = reconcile_full(&ws, &git)?;

    Ok(Outcome {
        result: json!({
            "id": Value::Null,
            "full": true,
            "ops_applied": outcome.ops_applied,
            "checks_run": outcome.checks_run,
            "tests_run": outcome.tests_run,
            // Always `[]` here: a full reseal belongs to no change, so there
            // is no journal to judge (D7). Present anyway -- one command,
            // one result shape.
            "witness_warnings": outcome.witness_warnings,
        }),
        human: format!(
            "resealed the project: {} check(s), {} test(s)",
            outcome.checks_run, outcome.tests_run
        ),
        next_actions: vec!["telos status".to_string()],
    })
}

/// A `CHG-NNNN` argument, or a `TELOS_REFERENCE_UNKNOWN` naming what was
/// expected.
///
/// A domain error rather than a clap `value_parser` (which would exit 2
/// with a usage message): a mistyped id is the same class of mistake as an
/// id that does not exist, and an agent reading the envelope should find
/// both under one code. Shared with `add`/`edit`/`remove`, whose `--change`
/// argument is the same thing.
pub(crate) fn parse_change_id(id: &str) -> Result<ChangeId, TelosError> {
    id.parse::<ChangeId>().map_err(|_| {
        TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("cannot parse `{id}` as a change id"),
        )
    })
}

/// The `ops` array of Annex E's `show CHG-…`: one `{n, op, entity, key}`
/// descriptor per staged op, `n` counting from 1 in staged order (the order
/// *is* the transaction -- D1).
///
/// Lives here rather than in `show` because it describes a change, and
/// `change diff` (T8) reports the same descriptors with `before`/`after`
/// added.
pub(crate) fn op_descriptors(change: &Change) -> Vec<Value> {
    change
        .ops
        .iter()
        .enumerate()
        .map(|(i, op)| {
            json!({
                "n": i + 1,
                "op": op.verb(),
                "entity": op.entity(),
                "key": op.key(),
            })
        })
        .collect()
}
