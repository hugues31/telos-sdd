//! `telos change <open|list|abandon>`: the lifecycle of a staged
//! transaction, minus the verbs that need a delta (`diff`/`approve` land in
//! T8, `reconcile` in T10 -- absent from the enum below rather than stubbed,
//! so `telos change approve` is a clap usage error today, not a command that
//! answers with something meaningless).
//!
//! Two rules shape every function here:
//!
//! - **The store writes, never this module.** A change file's bytes come
//!   from [`write_change`] (hence from `emit_change`), its content from
//!   [`read_change`] (hence from `parse_change_file`), and its deletion from
//!   [`delete_change`]. Nothing here formats or decodes a change itself.
//! - **Only `open` is gated (D17).** Opening a change stages new spec on top
//!   of the sealed base, so it needs that base to still be the sealed one;
//!   `list` and `abandon` read, or clean up, and a drifted project is
//!   exactly when a caller needs them most.

use clap::Subcommand;
use serde_json::{Value, json};

use telos_core::changes::{
    delete_change, list_change_ids, open_change_infos, read_change, write_change,
};
use telos_core::counters::{Alloc, floors, read_counters, write_counters};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::ChangeId;
use telos_core::lock::Lock;
use telos_core::model::{Change, ChangeStatus};
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, diagnostics_to_error, project, require_no_unclaimed_drift};
use crate::envelope::{CmdResult, Outcome};

/// The three verbs M2's T5 exposes.
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
}

pub fn run(ctx: &Ctx, command: &ChangeCommand) -> CmdResult {
    match command {
        ChangeCommand::Open { motivation } => open(ctx, motivation),
        ChangeCommand::List => list(ctx),
        ChangeCommand::Abandon { id } => abandon(ctx, id),
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
    };
    write_change(&project.ws, &change)?;
    write_counters(&project.ws, &alloc.counters())?;

    Ok(Outcome {
        result: json!({ "id": id, "status": ChangeStatus::Open.as_str() }),
        human: format!("opened {id}"),
        next_actions: vec![format!("telos add intent --change {id}")],
    })
}

/// The allocator for a fresh id: `max(persisted counters, scanned floors)`
/// (D4).
///
/// The floor scan needs the *sealed model* (its highest intent, scenario and
/// constraint ids), every open change's ops, and the change that produced
/// the current seal. Two details are worth spelling out:
///
/// - A change file that does not parse contributes no op to
///   [`floors`] -- there is nothing trustworthy to scan -- but its **id**
///   still has to hold the change counter down, or abandoning a corrupted
///   file would let the next `open` reissue its id. Hence the explicit
///   `max` over [`list_change_ids`], which reads only file names.
/// - The model is required to parse. `open` is a mutation: allocating an id
///   against a spec whose highest ids cannot be read is how ids get reused.
///   A spec that fails to parse but has not drifted is rare (it was sealed
///   broken) and is a `check` problem, reported here as such.
fn allocator(ws: &Workspace, lock: &Lock) -> Result<Alloc, TelosError> {
    let model = ws.load_model().map_err(diagnostics_to_error)?;
    let ids = list_change_ids(ws)?;
    let parsed: Vec<Change> = ids
        .iter()
        .filter_map(|id| read_change(ws, *id).ok())
        .collect();

    let mut floor = floors(&model, &parsed, lock.sealed_by);
    for id in &ids {
        floor.change = floor.change.max(id.0);
    }

    Ok(Alloc::new(read_counters(ws)?, floor))
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

/// A `CHG-NNNN` argument, or a `TELOS_REFERENCE_UNKNOWN` naming what was
/// expected.
///
/// A domain error rather than a clap `value_parser` (which would exit 2
/// with a usage message): a mistyped id is the same class of mistake as an
/// id that does not exist, and an agent reading the envelope should find
/// both under one code.
fn parse_change_id(id: &str) -> Result<ChangeId, TelosError> {
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
