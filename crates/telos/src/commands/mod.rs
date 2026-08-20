//! One module per command, each exposing a `run` that returns a
//! [`CmdResult`] and prints nothing -- rendering belongs to
//! [`crate::render`], and having exactly one place that writes to a stream is
//! what keeps every command's human and JSON output consistent.

pub mod change;
pub mod check;
pub mod impact;
pub mod init;
pub mod list;
pub mod mutate;
pub mod query;
pub mod show;
pub mod status;

use std::path::PathBuf;

use serde_json::json;

use telos_core::changes::{OpenChangeInfo, list_change_ids, open_change_infos, read_change};
use telos_core::counters::{Alloc, floors, read_counters};
use telos_core::error::{Diagnostic, ErrorCode, TelosError};
use telos_core::git::GitRepo;
use telos_core::graph::NodeRef;
use telos_core::ids::{ConstraintId, EntityRef, IntentId, NotionName, ScenarioId};
use telos_core::lock::Lock;
use telos_core::model::{Change, TelosModel};
use telos_core::state::{DRIFT_HINT, ProjectStateKind, StateReport, compute_state};
use telos_core::suggest;
use telos_core::workspace::Workspace;

use crate::envelope::{CmdResult, Outcome};

/// What a command knows about where it runs. Discovery (git repository,
/// workspace) starts from `cwd`.
pub struct Ctx {
    pub cwd: PathBuf,
}

/// Reads `ws`'s `telos.lock`, turning a missing file into a domain error.
///
/// A discovered workspace (`telos/telos.toml` exists) without a
/// `telos.lock` is not simply "unsealed" -- `telos init` always seals, so
/// there is no code path that leaves a real project in that state on
/// purpose. It is an abnormal state distinct from "no `telos/` at all"
/// ([`Workspace::discover`]'s own `TelosNotInitialized`, with its own
/// hint), so it gets its own message and hint here rather than reusing
/// `discover`'s.
fn require_lock(ws: &Workspace) -> Result<Lock, TelosError> {
    Lock::read(&ws.lock_path())?.ok_or_else(|| {
        TelosError::new(ErrorCode::TelosNotInitialized, "telos.lock is missing").hint(
            "the project was never sealed; run `telos init` in a fresh repository or restore telos.lock from git",
        )
    })
}

// --- the state-aware commands' shared preamble and gates --------------------

/// A discovered, sealed project and the state it is in: what `status`,
/// `check --sealed` and `change open` (and, from T7 on, every staging
/// command) all start from, computed once, the same way, in the same order.
///
/// Holding the workspace and its [`StateReport`] together is what keeps the
/// gates below cheap and honest: a command that has a `Project` has already
/// paid for its state, so asking «may I mutate?» costs nothing more and can
/// never be answered from a second, independently computed state that has
/// since moved.
pub(crate) struct Project {
    pub ws: Workspace,
    pub lock: Lock,
    /// The repository the workspace lives in, kept rather than dropped
    /// because `change reconcile` (T10) needs it to re-hash the tree it is
    /// about to seal -- and re-discovering it there would be a second,
    /// independent answer to "which repository is this".
    pub git: GitRepo,
    /// Every open change, best-effort per D15 -- an unparseable change file
    /// is reported here, never an error. Kept alongside the state it was
    /// computed from because it carries what the state does not: each
    /// change's `claims`, which is what the D5 gate reads.
    pub changes: Vec<OpenChangeInfo>,
    pub state: StateReport,
}

/// Discovers the workspace and the git repository from `ctx.cwd`, requires
/// a lock, scans the open changes, and computes the project's state.
///
/// The order is the one `docs/contracts.md` fixes for `check --sealed` and
/// is shared by every caller so that the *first* thing to go wrong is
/// always reported the same way: no `telos/` (`TELOS_NOT_INITIALIZED` from
/// [`Workspace::discover`]), then no `telos.lock` ([`require_lock`]), then
/// no git repository, then the state itself.
pub(crate) fn project(ctx: &Ctx) -> Result<Project, TelosError> {
    let ws = Workspace::discover(&ctx.cwd)?;
    let lock = require_lock(&ws)?;
    let git = GitRepo::discover(&ctx.cwd)?;
    let changes = open_change_infos(&ws)?;
    let state = compute_state(&ws, &lock, &git, &changes)?;
    Ok(Project {
        ws,
        lock,
        git,
        changes,
        state,
    })
}

/// The allocator for a fresh id: `max(persisted counters, scanned floors)`
/// (D4).
///
/// The floor scan needs the *sealed model* (its highest intent, scenario and
/// constraint ids), every open change's ops, and the change that produced
/// the current seal. Two details are worth spelling out:
///
/// - A change file that does not parse contributes no op to [`floors`] --
///   there is nothing trustworthy to scan -- but its **id** still has to
///   hold the change counter down, or abandoning a corrupted file would let
///   the next `open` reissue its id. Hence the explicit `max` over
///   [`list_change_ids`], which reads only file names.
/// - The model is required to parse. Allocating an id against a spec whose
///   highest ids cannot be read is how ids get reused. A spec that fails to
///   parse but has not drifted is rare (it was sealed broken) and is a
///   `check` problem, reported here as such.
///
/// Shared by `change open` and by the three staging verbs: one definition of
/// "where do the next ids start", so two commands can never disagree.
pub(crate) fn allocator(ws: &Workspace, lock: &Lock) -> Result<Alloc, TelosError> {
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

/// D17's gate: refuses to go on while drift nobody claimed is on disk.
///
/// "Unclaimed" is the whole subtlety, and it is [`compute_state`] that
/// resolves it: a path an open change claims is that change in progress,
/// not damage (D5), so it never reaches `state.drift` and never trips this.
/// What trips it is a sealed file edited outside the protocol -- staging
/// more spec on top of that would seal a base nobody reviewed.
///
/// Used by `change open` here, and by `add`/`edit`/`remove` (T7),
/// `change approve` (T8) and `change reconcile` without `--full` (T10).
/// `change diff|list|abandon`, `status`, `check` and `show` deliberately do
/// not call it: they read, or they clean up, and a drifted project is
/// exactly when a caller most needs them.
pub(crate) fn require_no_unclaimed_drift(project: &Project) -> Result<(), TelosError> {
    if project.state.state == ProjectStateKind::Drifted {
        return Err(TelosError::new(
            ErrorCode::TelosDriftDetected,
            "the project has drifted from its seal",
        )
        .hint(DRIFT_HINT));
    }
    Ok(())
}

/// D15's addition to `check --sealed`: "sealed and unmodified" cannot be
/// true while a change is still open, so `changing` is refused with its own
/// code rather than folded into `TELOS_DRIFT_DETECTED` -- the two states
/// have different remedies, and only one of them is damage.
///
/// Called *after* [`require_no_unclaimed_drift`], which mirrors the state
/// priority of D15: unclaimed drift outranks an open change, so a project
/// that is both reports the drift.
pub(crate) fn require_no_open_changes(project: &Project) -> Result<(), TelosError> {
    if project.state.state == ProjectStateKind::Changing {
        return Err(TelosError::new(
            ErrorCode::TelosChangeStateInvalid,
            "open changes; reconcile or abandon them",
        )
        .hint("run `telos change list`"));
    }
    Ok(())
}

/// `telos version` -- the same version `--version` prints, in the envelope.
///
/// Small enough to live here rather than in its own module: it reads nothing
/// and touches nothing.
pub fn version() -> CmdResult {
    Ok(Outcome {
        result: json!({ "version": telos_core::VERSION }),
        human: format!("telos {}", telos_core::VERSION),
        next_actions: Vec::new(),
    })
}

/// Collapses a full diagnostics list into the single [`TelosError`] the
/// envelope carries.
///
/// `code` and `hint` are the first diagnostic's -- the frozen error body
/// has room for exactly one of each, so a command that can find several
/// problems in one pass (`check`, but also `show`/`list` loading the same
/// model) surfaces the first (Annex B). The *message* stays multi-line when
/// there is more than one diagnostic: every diagnostic gets its own `file:
/// message` line (via the same `From<Diagnostic>` conversion, applied to
/// each), so a human reading stderr sees everything the load found in this
/// run. In `--json` mode this means `error.message` can itself carry more
/// than one line -- an agent that only reads the first line still gets the
/// primary diagnosis; this M1 limitation (no `result.diagnostics` array on
/// failure) is documented in `docs/contracts.md`.
pub(crate) fn diagnostics_to_error(diagnostics: Vec<Diagnostic>) -> TelosError {
    let mut iter = diagnostics.into_iter();
    let first = iter
        .next()
        .expect("`load_model` reports at least one diagnostic on `Err`");
    let mut error: TelosError = first.into();
    for diagnostic in iter {
        let extra: TelosError = diagnostic.into();
        error.message.push('\n');
        error.message.push_str(&extra.message);
    }
    error
}

// --- shared `show`/`query`/`impact` suggestion helpers ----------------------
//
// A typed id (`INT-9999`) and a bare notion name (`Rogue`) that resolve to
// nothing get different -- and deliberately different -- suggestion
// algorithms: numeric distance for an id (two ids that look nothing alike as
// text can still be numerically close, once the argument's prefix has
// already picked the entity's type), edit distance for a name (`suggest::
// closest`, the same one the parser and semantic checker use). `show` is
// where both were first written; `query`'s `--using`/`--triggered-by` and
// `impact`'s argument resolution reuse them rather than reimplementing
// either.

/// «cannot parse `foo-bar` as an id or notion name» -- the diagnosis for an
/// argument `EntityRef::from_str` rejects outright, replacing whatever error
/// the fallback `NotionName::new` produced (a parse error about a notion
/// name specifically, since a bare word is the fallback every unprefixed
/// argument takes) with one that names what the command -- not the notion
/// grammar -- actually expected.
pub(crate) fn unparsable(target: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosReferenceUnknown,
        format!("cannot parse `{target}` as an id or notion name"),
    )
}

/// «unknown intent `INT-9999`» and friends, with `hint` attached only when
/// one was found.
pub(crate) fn unknown(noun: &str, id: impl std::fmt::Display, hint: Option<String>) -> TelosError {
    let error = TelosError::new(
        ErrorCode::TelosReferenceUnknown,
        format!("unknown {noun} `{id}`"),
    );
    match hint {
        Some(hint) => error.hint(hint),
        None => error,
    }
}

/// The nearest existing id to `target` by numeric distance, rendered and
/// wrapped as a hint.
pub(crate) fn nearest_id(
    target: u32,
    existing: impl Iterator<Item = u32>,
    render: impl Fn(u32) -> String,
) -> Option<String> {
    existing
        .min_by_key(|&n| target.abs_diff(n))
        .map(|n| format!("closest is {}", render(n)))
}

/// Turns a `show`/`impact` argument into the graph node it names, or the
/// same «unknown ...» error (with the appropriate suggestion) `show` reports
/// for the same argument -- `impact` needs only the node, not the entity's
/// full data `show` also prints, so it resolves through this rather than
/// through `show`'s own per-type lookups.
pub(crate) fn resolve_or_hint(model: &TelosModel, r: &EntityRef) -> Result<NodeRef, TelosError> {
    if let Some(node) = model.resolve(r) {
        return Ok(node);
    }
    match r {
        EntityRef::Notion(name) => {
            let known: Vec<&str> = model.notions.keys().map(NotionName::as_str).collect();
            let hint = suggest::closest(name.as_str(), known.iter().copied())
                .map(|c| format!("closest is `{c}`"));
            Err(unknown("notion", name, hint))
        }
        EntityRef::Intent(id) => {
            let hint = nearest_id(id.0, model.intents.keys().map(|i| i.0), |n| {
                IntentId(n).to_string()
            });
            Err(unknown("intent", id, hint))
        }
        EntityRef::Scenario(id) => {
            let hint = nearest_id(id.0, model.scenario_owner.keys().map(|s| s.0), |n| {
                ScenarioId(n).to_string()
            });
            Err(unknown("scenario", id, hint))
        }
        EntityRef::Constraint(id) => {
            let hint = nearest_id(id.0, model.constraints.keys().map(|c| c.0), |n| {
                ConstraintId(n).to_string()
            });
            Err(unknown("constraint", id, hint))
        }
        // A change is a transaction record, not a node of the spec graph:
        // there is no edge to walk from it, so `impact` -- the one caller
        // of this helper -- has nothing to answer. `show CHG-…` does
        // resolve one, against the change store rather than the model.
        EntityRef::Change(_) => Err(TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            "`impact` does not apply to changes",
        )),
    }
}
