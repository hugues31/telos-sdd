//! `reconcile`: the transaction that turns an approved delta into spec files
//! on disk and a fresh seal.
//!
//! Everything the rest of the engine builds -- the change store, the
//! overlay, the globs, the shell, the lock -- exists so that this module can
//! do one thing safely: decide, *entirely in memory*, whether a staged
//! transaction may be applied, and only then write.
//!
//! # The gate order is frozen
//!
//! [`reconcile_change`] runs eleven gates, in this order, and the order is
//! contract rather than implementation detail -- an agent that fixes what a
//! reconcile complains about must converge, and it only converges if the
//! complaint it gets is the *first* thing wrong rather than an arbitrary one:
//!
//! 1. **Drift**. Anything on disk that no longer matches the seal
//!    and that no open change claims stops everything. A path *this* change
//!    claims is the change in progress, not damage; a path *another* open
//!    change claims is admissible too -- and is exactly what the carry-over
//!    below exists for.
//! 2. **Status**. `approved` or `implementing`; nothing else can be
//!    reconciled.
//! 3. **Digest**. The delta must still be the one that was approved.
//! 4. **Accept OIDs**. Each `accept` op sealed a specific blob; the
//!    file must still hash to it.
//! 5. **The overlay** ([`crate::overlay::apply_ops_idempotent`], plus the
//!    journal folded on top). The spec the delta describes must parse and
//!    resolve references, validate active intents and events, type-check
//!    literals, and preserve referential integrity. The *idempotent*
//!    application is what a whole
//!    change needs rather than the staging one: a delta `adopt` produced
//!    describes a working tree that already shows it, so the staging
//!    preconditions -- add-what-exists, remove-what-does-not -- would refuse
//!    the very state they are meant to protect. The journal is folded in
//!    ([`crate::overlay::fold_journal_bindings`]) *before* the model is
//!    built, so the `implements` a `bind` line asserts and the `proves` a
//!    green run asserts are resolved here like any other binding -- and
//!    every gate below judges the model they are part of.
//! 6. **No unbound code**, evaluated over the *post* model --
//!    the folded one, so a file bound by a journal line is covered. A path
//!    some open change's journal has declared in flight -- this change's
//!    own included -- is exempt: see [`require_no_orphan_code`].
//! 7. **The sealed code coverage**. Every path the *previous* seal held
//!    in its `code` table must still be covered by a binding of the folded
//!    model, unless this delta stages `telos/bindings.tel` itself -- in
//!    which case the shrink is what was reviewed. See
//!    [`require_no_coverage_shrink`].
//! 8. **The sealed red witness**, under `policy.tdd = "strict"`: every
//!    scenario this delta makes new or different owes an intact red/green
//!    pair on the bytes its test file has right now. Under `advisory` the
//!    same verdicts come back as [`ReconcileOutcome::witness_warnings`]
//!    instead of refusing. See [`check_witnesses`].
//! 9. **Sealable structure**. Every scenario of every active
//!    intent has at least one `proves`, and active obligations have a runner.
//!    This is deliberately stricter than spec-only model loading/rebuild.
//! 10. **Constraint checks**, for the constraints this delta puts in
//!     scope.
//! 11. **Tests**, one run per distinct `proves` target of the impacted
//!     scenarios -- the folded model's, so a scenario the journal just proved
//!     is actually run and `tests_run` is honest.
//!
//! [`reconcile_full`] is the same transaction with the delta taken out: no
//! change, so no drift, status, digest or accept gate, no journal
//! to fold and therefore no witness gate, no previous lock and therefore no
//! coverage gate, and no ops to apply -- only the five gates that prove a
//! spec on its own (5, 6, 9, 10, 11) and the seal they earn. It is the exit from
//! a `telos.lock` merge conflict, and the way a preexisting spec tree gets
//! its first seal.
//!
//! # The carry-over: what the seal must not launder
//!
//! Gate 1 admits drift claimed by another open change, and it has to: an
//! implementing change drifts its own code files for its whole life,
//! and a concurrent reconcile of an unrelated change cannot be held hostage
//! to it. But the seal that reconcile writes is computed by re-hashing the
//! *whole* tree from disk, so left alone it would fold those foreign bytes
//! -- never reviewed, never approved -- into a fresh lock, and abandoning
//! the change that claimed them would leave the project `coherent` with an
//! out-of-protocol edit permanently sealed. That is precisely the invariant
//! behind CLI-only specification mutations and drift detection.
//!
//! So the drift of a path *another* open change claims is admissible for
//! this transaction without being sealable by it. Every such path is
//! **carried over**: the fresh lock records for it exactly what the
//! previous lock recorded -- the same OID, in the same table, or the same
//! absence if the previous lock never held it (an adopted but unreconciled
//! `Rogue.tel` stays untracked rather than becoming sealed). The drift
//! therefore survives the reconcile: [`crate::state::compute_state`] still
//! sees it, the claiming change still covers it, and the moment that change
//! is abandoned the path resurfaces as `drifted`, exactly as it would have
//! had the concurrent reconcile never run. A change's *own* claims are not
//! carried over: it is reconciling them, its ops have just rewritten them,
//! and re-hashing them from disk is the whole point.
//!
//! The seam is deliberate. [`seal`] is left alone -- it means, and keeps
//! meaning, “the bytes on disk right now”, one honest sentence rather than
//! one qualified by whoever is calling it. [`reconcile_change`] applies the
//! carry-over to the lock [`seal`] hands back, before writing it, and owns
//! the exception in its own docs. That also keeps [`reconcile_full`]'s
//! contract intact: `--full` re-proves everything from disk and seals disk
//! truth, open adopt-changes included, by design. It is total
//! proof, not a bypass -- a `--full` run while a change holds adopted drift
//! seals that drift, because `--full`'s answer to “is this tree provable?”
//! is computed over the tree, not over anybody's claims.
//!
//! # Atomicity, and what stands in for a rollback
//!
//! Not one byte is written before gate 11 has passed. After that, the write
//! order is fixed: the ops' spec `.tel` files, then `telos/bindings.tel`
//! re-emitted from the folded model (both through the emitter -- reconcile
//! never edits text), then `telos.lock`, then the change file's deletion.
//! Writing `bindings.tel` before the lock ensures the bindings table is never
//! updated while a change is open, so it cannot reference code or an intent
//! that the spec tree does not yet hold.
//! Sealing *after* writing is deliberate: [`seal`] re-hashes the spec tree
//! from disk, so the lock records the bytes that are really there rather
//! than the bytes we believed we wrote -- everywhere except the carried-over
//! paths above, which are the one place where “what is really there” is not
//! this transaction's to record.
//!
//! There is no hand-rolled rollback. If the process dies between two of
//! those writes the working tree is left half-applied, and the recovery
//! mechanism is git: the committed state is the checkpoint, `git checkout`
//! is the undo. Building a journal here would duplicate, worse, a
//! transaction log the project already has.
//!
//! `counters.toml` is not touched: every id this transaction spends was
//! allocated and persisted when the op was staged, so there is nothing
//! left to bump -- and the floors keep holding those ids down after the
//! change file is gone, because the entities themselves are now in the spec.

use std::collections::{BTreeMap, BTreeSet};

use crate::changes::{OpenChangeInfo, delete_change, diagnostics_to_error};
use crate::config::{Config, TddPolicy};
use crate::emit::emit_file;
use crate::error::{ErrorCode, TelosError};
use crate::exec::{run_shell, run_shell_with_filter, substitute_filter};
use crate::git::{GitRepo, Oid};
use crate::globs::{glob_matches, orphan_code};
use crate::graph::NodeRef;
use crate::ids::{ConstraintId, IntentId, RepoPath, ScenarioId};
use crate::lock::Lock;
use crate::model::{
    Binding, Change, ChangeStatus, Constraint, IntentStatus, JournalEntry, Scope, StagedOp,
    TelFile, TelosModel, TestRef,
};
use crate::overlay::{apply_config_ops, apply_ops_idempotent, fold_journal_bindings, parse_base};
use crate::repo_fs::RepoFs;
use crate::semantic::build_model;
use crate::state::{DRIFT_HINT, compute_state};
use crate::witness::{WitnessVerdict, required_witnesses, witness_verdict};
use crate::workspace::{BINDINGS_PATH, Workspace};

/// What one reconcile did.
#[derive(Debug)]
pub struct ReconcileOutcome {
    /// The staged ops applied -- every op of the change, since a reconcile
    /// is all-or-nothing.
    pub ops_applied: u32,
    /// Constraint `check` commands run.
    pub checks_run: u32,
    /// `[test] cmd` invocations run. Zero when no active obligation
    /// requires a runner and no impacted proof target is runnable.
    pub tests_run: u32,
    /// The witness verdicts that would have refused this reconcile under
    /// `policy.tdd = "strict"`, using the stable wording from the error contract.
    ///
    /// Always empty except on an `advisory` project that owes a witness it
    /// does not have: under `strict` the first such verdict is an error
    /// rather than a warning, so a successful strict reconcile has nothing
    /// to report, and a `--full` reseal belongs to no change and therefore
    /// to no journal.
    pub witness_warnings: Vec<String>,
    /// The seal this reconcile wrote.
    pub lock: Lock,
}

/// Applies one approved change: the eleven gates in the module docs, followed by
/// an atomic write phase.
///
/// `others` is what the change store currently reports open. It is only read
/// for its `claims`, to decide which drift is somebody's business and which
/// is nobody's (gate 1), and which unbound code file is somebody's declared
/// work in flight (gate 6) -- whether it happens to include `change` itself
/// makes no difference, since `change`'s own claims are folded in either
/// way and an entry for `change` is filtered back out of the *foreign*
/// claims.
///
/// The `lock` it is handed is the seal this transaction supersedes, and it
/// is read three times: by gate 1, to know what has drifted; by gate 7, to
/// know what code coverage it recorded; and at the end, to carry over the
/// paths whose drift another open change claims -- see the module docs.
///
/// On `Ok` the spec tree, `telos.lock` and `telos/changes/` have all moved.
/// On `Err` nothing has been written at all: every gate runs before the
/// first byte.
pub fn reconcile_change(
    ws: &Workspace,
    git: &GitRepo,
    lock: &Lock,
    change: &Change,
    others: &[OpenChangeInfo],
) -> Result<ReconcileOutcome, TelosError> {
    let carried = classify_drift(ws, git, lock, change, others)?;
    require_approved(change)?;
    require_fresh_approval(change)?;
    require_accepted_bytes(git, change)?;

    let effective_ws = Workspace {
        repo_root: ws.repo_root.clone(),
        telos_dir: ws.telos_dir.clone(),
        config: apply_config_ops(&ws.config, &change.ops),
    };
    Config::validate_transition(&ws.config, &effective_ws.config)?;
    let base = parse_base(ws).map_err(diagnostics_to_error)?;
    let folded = fold_journal_bindings(apply_ops_idempotent(base.clone(), &change.ops), change);
    let model = build_model(folded).map_err(diagnostics_to_error)?;
    require_no_orphan_code(&effective_ws, &model, &in_flight_paths(change, others))?;
    require_no_coverage_shrink(lock, &model, &change.ops)?;

    let witness_warnings = check_witnesses(&effective_ws, git, &base, &model, change)?;
    require_sealable_structure(&effective_ws, &model)?;

    let proven = capture_seal_snapshot(ws, &model, git)?;
    let impacted = impacted_nodes(ws, &model, &change.ops);
    let checks_run = run_constraint_checks(&effective_ws, &model, Some(&impacted))?;
    let tests_run = run_tests(&effective_ws, &model, &impacted)?;
    require_unchanged_snapshot(ws, git, &proven)?;

    // --- write phase: everything above passed, so and only so, write. ---
    for op in &change.ops {
        apply_op(ws, op)?;
    }
    write_bindings(ws, &model)?;
    let spec_paths = ws.spec_files()?;
    let spec = git.blob_oids(&spec_paths)?;
    require_complete_map("specification", &spec_paths, &spec)?;
    require_code_snapshot(git, &proven.code)?;
    let publication_spec = spec.clone();
    let mut fresh = lock_from_maps(spec, proven.code.clone(), Some(change.id));
    carry_over(&mut fresh, lock, &carried);
    require_publication_snapshot(ws, git, &publication_spec, &proven.code)?;
    fresh.write_to_workspace(ws)?;
    require_publication_snapshot(ws, git, &publication_spec, &proven.code)?;
    delete_change(ws, change.id)?;

    Ok(ReconcileOutcome {
        ops_applied: change.ops.len() as u32,
        checks_run,
        tests_run,
        witness_warnings,
        lock: fresh,
    })
}

/// Re-proves the whole project from the files on disk and reseals it.
///
/// This is the exit from a lock merge conflict, and the one legitimate way
/// to seal a spec tree that exists but was never sealed -- the two
/// situations in which the current `telos.lock` is worthless. So it never
/// reads one: absent, conflicted or corrupt are all the same input here,
/// and re-proving everything is exactly what makes ignoring it safe.
///
/// Four gates of [`reconcile_change`] are structurally absent rather than
/// skipped. **Drift** (gate 1) cannot apply: drift is a disagreement with a
/// lock this function does not consult, and re-proving the tree is a
/// stronger answer than comparing it to a seal nobody trusts. **Status,
/// digest and accept OIDs** (gates 2-4) are properties of a change, and
/// there is none: open changes are tolerated and left exactly as they are,
/// files untouched and still open -- a full reseal is about the spec on
/// disk, not about anybody's staged delta. **The sealed code coverage**
/// (gate 7) is a comparison against the previous seal, so it goes with the
/// lock this function does not read. Whether the tree is provable is computed
/// from the tree itself; what an older, possibly
/// worthless lock happened to cover is no part of it.
///
/// That last one is an escape hatch, not a proof, and it is worth naming as
/// such: `--full` seals disk truth, so it *is* the one remaining way a
/// `bindings.tel` that shrank outside the protocol reaches a fresh lock
/// with nobody having approved the shrink. The unbound-code gate does not
/// close the hole -- it judges only the files the *current* globs match,
/// and the same hand edit that drops an `implements` line can empty the
/// `[code]` globs that would have caught the file it covered (the e2e
/// `a_full_reseal_is_exempt_from_the_coverage_gate` is exactly that tree).
/// What keeps this honest is that `--full` is a deliberate, human-invoked
/// act with no delta behind it: the same property that makes it the exit
/// from a `telos.lock` merge conflict makes it the operator's
/// responsibility, which is where full reconciliation puts it.
///
/// **The witness** (gate 8) is a property of a change's
/// *journal*, so it goes the same way, and `witness_warnings` comes back
/// empty -- and with no change to claim anything in flight, the unbound-code gate exempts
/// nothing here either, so a `--full` run while some change holds a
/// red-only journal refuses on that change's test file. `--full` is
/// stricter than a per-change reconcile there, deliberately: it proves the
/// disk, and an unreconciled journal is not on it. **The ops** are likewise
/// absent,
/// hence `ops_applied: 0` and a `sealed_by: None` seal: no transaction
/// produced this state, it was simply found -- and with no ops there is no
/// journal to fold either, so `bindings.tel` is read as it stands rather
/// than rewritten.
///
/// What remains is what proves a spec on its own: a fully parsed and resolved
/// model, the unbound-code gate, sealable proof/runner structure, every
/// constraint that has a `check` (there is no delta to narrow the scope),
/// and, when the model has active obligations, one run of `[test] cmd` with
/// an empty `{filter}` for the whole suite.
pub fn reconcile_full(ws: &Workspace, git: &GitRepo) -> Result<ReconcileOutcome, TelosError> {
    // [`seal`] checks this too, but only after every check and test has
    // run: paying for it upfront turns "you invoked this from the wrong
    // repository" into an immediate answer rather than one that arrives
    // after a full test suite.
    git.ensure_matches_workspace_root(&ws.repo_root)?;
    ws.config.validate_self()?;

    let model = ws.load_model().map_err(diagnostics_to_error)?;
    require_no_orphan_code(ws, &model, &BTreeSet::new())?;
    require_sealable_structure(ws, &model)?;

    let proven = capture_seal_snapshot(ws, &model, git)?;
    let checks_run = run_constraint_checks(ws, &model, None)?;
    let tests_run = if model
        .intents
        .values()
        .any(|intent| intent.status == IntentStatus::Active)
    {
        run_full_tests(ws)?
    } else {
        0
    };

    require_unchanged_snapshot(ws, git, &proven)?;
    let lock = lock_from_maps(proven.spec.clone(), proven.code.clone(), None);
    require_publication_snapshot(ws, git, &lock.spec, &lock.code)?;
    lock.write_to_workspace(ws)?;
    require_publication_snapshot(ws, git, &lock.spec, &lock.code)?;

    Ok(ReconcileOutcome {
        ops_applied: 0,
        checks_run,
        tests_run,
        // No change, so no journal, so nothing to say about witnesses: the
        // field is present and empty rather than absent, because the shape
        // of a reconcile result is one shape.
        witness_warnings: Vec::new(),
        lock,
    })
}

#[derive(Debug, Clone)]
struct SealSnapshot {
    spec: BTreeMap<RepoPath, Oid>,
    code: BTreeMap<RepoPath, Oid>,
}

/// Captures exactly the spec and bound-code bytes about to be exercised.
/// `blob_oids` itself rejects a path that changes while Git filters it.
fn capture_seal_snapshot(
    ws: &Workspace,
    model: &TelosModel,
    git: &GitRepo,
) -> Result<SealSnapshot, TelosError> {
    git.ensure_matches_workspace_root(&ws.repo_root)?;
    let spec_paths = ws.spec_files()?;
    let spec = git.blob_oids(&spec_paths)?;
    require_complete_map("specification", &spec_paths, &spec)?;

    let code_paths = model
        .bindings
        .iter()
        .map(|binding| binding.code_path().clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let code = git.blob_oids(&code_paths)?;
    require_complete_map("binding", &code_paths, &code)?;
    Ok(SealSnapshot { spec, code })
}

fn require_complete_map(
    kind: &str,
    paths: &[RepoPath],
    oids: &BTreeMap<RepoPath, Oid>,
) -> Result<(), TelosError> {
    if let Some(path) = paths.iter().find(|path| !oids.contains_key(*path)) {
        return Err(TelosError::new(
            ErrorCode::TelosIntegrityViolation,
            format!("{kind} path `{path}` disappeared before it could be proven"),
        ));
    }
    Ok(())
}

fn require_unchanged_snapshot(
    ws: &Workspace,
    git: &GitRepo,
    snapshot: &SealSnapshot,
) -> Result<(), TelosError> {
    let current_spec_paths = ws.spec_files()?;
    let expected_spec_paths = snapshot.spec.keys().cloned().collect::<Vec<_>>();
    if current_spec_paths != expected_spec_paths {
        return Err(snapshot_changed("the specification tree"));
    }
    require_exact_oids(git, &snapshot.spec, "the specification tree")?;
    require_exact_oids(git, &snapshot.code, "bound code or proof files")
}

fn require_code_snapshot(
    git: &GitRepo,
    expected: &BTreeMap<RepoPath, Oid>,
) -> Result<(), TelosError> {
    require_exact_oids(git, expected, "bound code or proof files")
}

fn require_publication_snapshot(
    ws: &Workspace,
    git: &GitRepo,
    spec: &BTreeMap<RepoPath, Oid>,
    code: &BTreeMap<RepoPath, Oid>,
) -> Result<(), TelosError> {
    let current_spec_paths = ws.spec_files()?;
    let expected_spec_paths = spec.keys().cloned().collect::<Vec<_>>();
    if current_spec_paths != expected_spec_paths {
        return Err(snapshot_changed("the specification tree"));
    }
    require_exact_oids(git, spec, "the specification tree")?;
    require_exact_oids(git, code, "bound code or proof files")
}

fn require_exact_oids(
    git: &GitRepo,
    expected: &BTreeMap<RepoPath, Oid>,
    subject: &str,
) -> Result<(), TelosError> {
    let paths = expected.keys().cloned().collect::<Vec<_>>();
    let current = git.blob_oids(&paths)?;
    if &current != expected {
        return Err(snapshot_changed(subject));
    }
    Ok(())
}

fn snapshot_changed(subject: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("{subject} changed while checks or tests were running"),
    )
    .hint("restore the intended bytes and reconcile again")
}

fn lock_from_maps(
    spec: BTreeMap<RepoPath, Oid>,
    code: BTreeMap<RepoPath, Oid>,
    sealed_by: Option<crate::ids::ChangeId>,
) -> Lock {
    Lock {
        version: 1,
        tool: format!("telos {}", crate::VERSION),
        sealed_by,
        spec_digest: Lock::compute_digest(&spec),
        spec,
        code,
    }
}

/// The write-time sealability check for active intents.
///
/// [`crate::semantic::build_model`] deliberately accepts a spec-only active
/// intent whose scenarios do not yet have proof bindings: rebuild planning
/// must remain usable while reconstruction is at 0/N. A seal is a stronger
/// claim. Before checks, tests, or writes, every scenario owned by an active
/// intent must have at least one distinct `proves` target, and a project with
/// any such obligations must have a runnable `[test] cmd`.
///
/// The first missing scenario is selected by id, independently of declaration
/// order, so every caller gets one stable and corrective error. This function
/// is public because sealed-state consumers use the same predicate to reject
/// locks produced by older Telos versions without making live/spec-only model
/// loading stricter.
pub fn require_sealable_structure(ws: &Workspace, model: &TelosModel) -> Result<(), TelosError> {
    let active_scenarios: BTreeSet<ScenarioId> = model
        .intents
        .values()
        .filter(|intent| intent.status == IntentStatus::Active)
        .flat_map(|intent| intent.scenarios.iter().map(|scenario| scenario.id))
        .collect();

    if active_scenarios.is_empty() {
        return Ok(());
    }

    let proved: BTreeSet<ScenarioId> = model
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Proves { scenario, .. } => Some(scenario.node),
            Binding::Implements { .. } => None,
        })
        .collect();
    if let Some(scenario) = active_scenarios
        .iter()
        .find(|scenario| !proved.contains(scenario))
    {
        return Err(TelosError::new(
            ErrorCode::TelosIntegrityViolation,
            format!("active scenario {scenario} has no `proves` binding"),
        )
        .hint(format!(
            "record a green proof for {scenario} through an approved change before reconciling"
        )));
    }

    if ws.config.test.cmd.trim().is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosTestNotFound,
            "no `[test] cmd` is configured in telos/telos.toml",
        )
        .hint("set [test] cmd, e.g. `cargo test {filter}`"));
    }

    Ok(())
}

// --- gate 1: drift, and the carry-over it decides ----------------------------

/// Sorts the project's drift into the three kinds this transaction has to
/// tell apart, and refuses on the only one that stops it.
///
/// One [`compute_state`] pass -- over *raw* drift, hence the empty
/// `open_changes` argument -- answers all three:
///
/// - **Unclaimed.** Nobody's business, so nobody reviewed it: the reconcile
///   refuses, naming the paths so a caller does not have to run `status` to
///   find out which.
/// - **Claimed by `change`.** The change in progress, not damage -- that is
///   the whole point of an open change, and this very reconcile is about to
///   overwrite those paths with its ops' canonical post-state. Admissible,
///   and sealed from disk like everything else.
/// - **Claimed by another open change.** Admissible too (implementing changes
///   drift their code files for their whole life, and an unrelated
///   reconcile cannot be held hostage to that), but *not* sealable here:
///   these are the paths returned, for [`carry_over`] to keep at their
///   previously sealed state. See the module docs.
///
/// The same verdict comes out whether or not the caller left `change` in
/// `others`: its own claims win over a foreign claim of the same path in
/// the second bullet, and an `others` entry for `change` itself is filtered
/// out of the third.
fn classify_drift(
    ws: &Workspace,
    git: &GitRepo,
    lock: &Lock,
    change: &Change,
    others: &[OpenChangeInfo],
) -> Result<BTreeSet<RepoPath>, TelosError> {
    let report = compute_state(ws, lock, git, &[])?;
    let mine = change.claims();
    let theirs: BTreeSet<&RepoPath> = others
        .iter()
        .filter(|info| info.id != change.id)
        .flat_map(|info| info.claims.iter())
        .collect();

    let mut carried = BTreeSet::new();
    let mut unclaimed = Vec::new();
    for entry in &report.drift {
        if mine.contains(&entry.path) {
            continue;
        }
        if theirs.contains(&entry.path) {
            carried.insert(entry.path.clone());
        } else {
            unclaimed.push(format!("`{}`", entry.path));
        }
    }

    if unclaimed.is_empty() {
        return Ok(carried);
    }
    Err(TelosError::new(
        ErrorCode::TelosDriftDetected,
        format!(
            "the project has drifted from its seal: {}",
            unclaimed.join(", ")
        ),
    )
    .hint(DRIFT_HINT))
}

/// Restores, in the `fresh` seal, exactly what `previous` recorded for every
/// path of `carried` -- the same OID in the same table, or the same absence
/// when `previous` held no record of it at all.
///
/// Both tables are rewritten for each path, rather than the one it was
/// expected in, so that “what the previous lock said about this path” is
/// reproduced whole: a path that was sealed as `code` and would now be
/// re-hashed into `spec` (or the reverse) cannot smuggle its bytes in
/// through the other table.
///
/// `spec_digest` is recomputed afterwards, since it is a digest *of* `spec`
/// ([`Lock::compute_digest`]): a carried-over lock still carrying the digest
/// of the re-hashed map would be internally inconsistent, which is not a
/// thing to write to disk whether or not anything reads it back today.
/// Nothing is recomputed when `carried` is empty, which is the ordinary
/// case: a lock with no foreign claim to carry is [`seal`]'s output
/// untouched, byte for byte.
fn carry_over(fresh: &mut Lock, previous: &Lock, carried: &BTreeSet<RepoPath>) {
    if carried.is_empty() {
        return;
    }
    for path in carried {
        restore(&mut fresh.spec, &previous.spec, path);
        restore(&mut fresh.code, &previous.code, path);
    }
    fresh.spec_digest = Lock::compute_digest(&fresh.spec);
}

fn restore(
    fresh: &mut BTreeMap<RepoPath, Oid>,
    previous: &BTreeMap<RepoPath, Oid>,
    path: &RepoPath,
) {
    match previous.get(path) {
        Some(oid) => fresh.insert(path.clone(), oid.clone()),
        None => fresh.remove(path),
    };
}

// --- gates 2 and 3: status and digest ---------------------------------------

/// Reconcile accepts `approved` and `implementing`. The latter is an approved
/// change in flight, so it
/// carries the same frozen digest and passes the same gate 3.
fn require_approved(change: &Change) -> Result<(), TelosError> {
    match change.status {
        ChangeStatus::Approved | ChangeStatus::Implementing => Ok(()),
        _ => Err(TelosError::new(
            ErrorCode::TelosChangeStateInvalid,
            format!("change {} is not approved; approve it first", change.id),
        )
        .hint(format!(
            "run `telos change diff {id}` then `telos change approve {id}`",
            id = change.id
        ))),
    }
}

/// Approval applies to a specific delta. Staging into an approved change is
/// deliberately allowed, so the digest is what stands
/// between "reviewed" and "reviewed, then quietly changed".
fn require_fresh_approval(change: &Change) -> Result<(), TelosError> {
    if !change.is_stale() {
        return Ok(());
    }
    Err(TelosError::new(
        ErrorCode::TelosApprovalStale,
        "the staged delta changed after approval",
    )
    .hint(format!(
        "re-approve with `telos change approve {}`",
        change.id
    )))
}

// --- gate 4: the accepted bytes ---------------------------------------------

/// An `accept` op means "the bytes at this path, as they were when `adopt`
/// saw them, are the intended bytes". If they moved since, the delta
/// under review is not the delta about to be sealed, so the reconcile stops
/// rather than sealing content nobody looked at.
///
/// One `blob_oids` batch for every accept of the change, so a change that
/// adopted a wide drift costs one child process, not one per path.
fn require_accepted_bytes(git: &GitRepo, change: &Change) -> Result<(), TelosError> {
    let accepts: Vec<(&RepoPath, &Oid)> = change
        .ops
        .iter()
        .filter_map(|op| match op {
            StagedOp::Accept { path, oid } => Some((path, oid)),
            _ => None,
        })
        .collect();
    if accepts.is_empty() {
        return Ok(());
    }

    let paths: Vec<RepoPath> = accepts.iter().map(|(path, _)| (*path).clone()).collect();
    let current = git.blob_oids(&paths)?;

    for (path, accepted) in accepts {
        let message = match current.get(path) {
            Some(now) if now == accepted => continue,
            Some(_) => format!("`{path}` changed since it was accepted"),
            None => format!("`{path}` was accepted but no longer exists"),
        };
        return Err(
            TelosError::new(ErrorCode::TelosIntegrityViolation, message).hint(format!(
                "re-run `telos adopt` to accept the current bytes of `{path}`"
            )),
        );
    }
    Ok(())
}

// --- gate 6: the unbound-code gate, no code without telos ----------------------------------

/// The paths the unbound-code gate must stay silent about because some open change has
/// already declared them in flight.
///
/// Two sources, and the asymmetry between them is the point:
///
/// - **This change's journal.** A `bind` line, or a red run's test file, is
///   a declaration that the path is being worked on *now*. A red run in
///   particular asserts no binding at all -- that is what red means --
///   so its test file is uncovered by construction until the green folds a
///   `proves` in. Left to the unbound-code gate, the only red-only reconcile there is would
///   answer “orphan, run `telos bind`”, which is the wrong next step and
///   does not converge: the honest verdict is gate 8's, one gate later, and
///   it names the green that is missing. The unbound-code gate is not weakened,
///   only deferred -- at the green end state the fold turns the run into a
///   `proves` and the path is covered by the model itself.
/// - **Every *other* open change's claims**, ops included. Implementing
///   changes write unbound code files for their whole life, and an unrelated
///   reconcile cannot be held hostage to that any more than gate 1 can be.
///   The exemption lasts exactly as long as the claim: abandon that change
///   and the file is the unbound-code gate's business again, on the very next reconcile.
///
/// `others` may or may not contain `change` itself; its entry is filtered
/// out either way, so the answer is the same -- exactly as in [`classify_drift`].
fn in_flight_paths(change: &Change, others: &[OpenChangeInfo]) -> BTreeSet<RepoPath> {
    change
        .journal
        .iter()
        .map(|entry| entry.path().clone())
        .chain(
            others
                .iter()
                .filter(|info| info.id != change.id)
                .flat_map(|info| info.claims.iter().cloned()),
        )
        .collect()
}

/// code coverage, over the model the delta describes: a file the `[code]` globs match
/// must be covered by an `implements` binding, one the `[tests]` globs match
/// by a `proves` one, independently.
///
/// [`orphan_code`] answers *which* files are uncovered; the wording is
/// recomputed here because the message has to name which of the two families
/// the file failed -- they have different remedies, and an agent reading
/// “no `implements` binding” knows what to write next.
///
/// `exempt` is what some open change has declared in flight
/// ([`in_flight_paths`]), and it is empty for a `--full` reseal: `--full`
/// proves the tree on disk, and a journal nobody has reconciled yet is not
/// part of that proof.
fn require_no_orphan_code(
    ws: &Workspace,
    model: &TelosModel,
    exempt: &BTreeSet<RepoPath>,
) -> Result<(), TelosError> {
    let orphans = orphan_code(ws, model)?;
    let Some(path) = orphans.iter().find(|path| !exempt.contains(*path)) else {
        return Ok(());
    };

    let implemented: BTreeSet<&RepoPath> = model
        .bindings
        .iter()
        .filter_map(|b| match b {
            Binding::Implements { path, .. } => Some(path),
            Binding::Proves { .. } => None,
        })
        .collect();
    let code_files = glob_matches(&ws.repo_root, &ws.config.code.globs)?;
    let (family, relation) = if code_files.contains(path) && !implemented.contains(path) {
        ("[code]", "implements")
    } else {
        ("[tests]", "proves")
    };

    Err(TelosError::new(
        ErrorCode::TelosOrphanCode,
        format!("`{path}` matches the {family} globs but no `{relation}` binding covers it"),
    )
    .hint(
        "Bind it with `telos bind <path> <INT-id>`, or remove it from the `telos.toml` globs if it isn't spec-governed.",
    ))
}

// --- gate 7: the sealed code coverage -----------------------------------

/// A reconcile may not shrink the code coverage recorded by the previous seal
/// unless that shrink is part of the reviewed delta.
///
/// [`seal`] derives `lock.code` from the model's bindings, so “the previous
/// lock held this path and the folded model binds nothing that reaches it”
/// means exactly one thing: the bindings table lost a line between the two
/// transactions. Nothing writes `bindings.tel` outside a reconcile, and no
/// journal line may even name it, so that line went missing outside the
/// protocol -- a hand edit, a bad merge, a truncation. Sealing over it would
/// launder that loss into a fresh, coherent-looking lock, and the file it
/// used to cover would silently stop being spec-governed. That is the
/// ledger attack, and
/// it is exactly what the CLI-only mutation contract forbids.
///
/// The one exemption is a delta that *stages* `telos/bindings.tel` -- the
/// `accept` op `adopt` produces for it. What that buys is bounded, and the
/// bound belongs in the contract rather than in a reader's optimism: the
/// shrink was **captured** by `adopt` into a change, and somebody approved
/// a delta one of whose ops is “take the current bytes of
/// `telos/bindings.tel`”. It is not a line-level review.
/// [`crate::overlay::op_before_after`] renders an `accept`'s `before` from
/// the tree as it stands -- already shrunk -- and its `after` as `None`, so
/// `change diff` never shows the approver *which* `implements` line went
/// missing. The exemption means “this shrink is owned by an approved
/// delta”, not “somebody read it”, and that is the whole claim.
///
/// Any op targeting the path counts, not just `accept`: what matters is
/// that the reviewed delta is about that file.
///
/// Deliberately local and static: the alternative -- deriving the expected
/// code table from the carried-over lock -- would need `git cat-file` on a
/// seal that may never have been committed, and would make [`seal`] report
/// something other than the bytes on disk. Here the two inputs are a lock
/// already in hand and a model already built, so the gate costs nothing and
/// says one thing.
///
/// The **first** dropped path aborts, in `lock.code`'s own sorted order, for
/// the convergence reason the whole gate order exists for: a caller fixes
/// one named thing and re-runs.
fn require_no_coverage_shrink(
    previous: &Lock,
    model: &TelosModel,
    ops: &[StagedOp],
) -> Result<(), TelosError> {
    // The follow-up this exemption is waiting on: render an `accept`'s
    // `before` from the *seal* rather than from disk, so that `change diff`
    // shows an approver which `implements` line the delta drops. Until then
    // what is exempted here is ownership, not a line-level review -- see the
    // doc above, which says so rather than implying otherwise.
    let bindings_path = RepoPath::new(BINDINGS_PATH);
    if ops.iter().any(|op| op.target_path() == bindings_path) {
        return Ok(());
    }

    let covered: BTreeSet<&RepoPath> = model.bindings.iter().map(Binding::code_path).collect();
    let Some(dropped) = previous.code.keys().find(|path| !covered.contains(path)) else {
        return Ok(());
    };

    Err(TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!(
            "sealing would drop `{dropped}` from the code table: no binding covers it \
             and this change does not stage {BINDINGS_PATH}"
        ),
    )
    .hint(
        "the bindings shrank outside this change; reconcile or abandon the change that \
         claims telos/bindings.tel, or restore them with `telos revert`",
    ))
}

// --- gate 8: the sealed red witness -------------------------------------

/// The witness discipline, as a static gate over the journal: every
/// scenario this delta makes new or different owes an intact red/green pair
/// on the bytes its test file has *right now*.
///
/// [`required_witnesses`] answers who owes one -- the scenarios of the
/// `add`/`edit intent` ops whose post-status is `active` and whose emitted
/// fragment is not already in the base, which is what exempts an intent
/// edited without touching its scenarios. [`witness_verdict`] answers
/// whether the journal pays it, against the current blob oids of the test
/// files the journal itself names: a path no run mentions cannot carry a
/// witness, and a path that vanished is absent from the map, which is
/// exactly how the verdict tells "changed" from "gone".
///
/// Under `strict` the **first** failing scenario aborts, in
/// [`required_witnesses`]' sorted order. That is the convergence property
/// the whole gate order exists for: a caller fixes one named thing, re-runs,
/// and gets the next one -- rather than a list it has to re-derive after
/// every step. Under `advisory` every verdict is collected instead and the
/// reconcile proceeds: the messages are the same frozen wordings, carried
/// out as [`ReconcileOutcome::witness_warnings`].
///
/// # A project with no runner owes no witness
///
/// An empty `[test] cmd` skips this witness-only gate. The following
/// sealable-structure gate still refuses active obligations without a runner;
/// this skip merely avoids suggesting an impossible `telos test` command
/// before that authoritative structural diagnosis.
/// ([`run_tests`] likewise has nothing to execute without a command.)
fn check_witnesses(
    ws: &Workspace,
    git: &GitRepo,
    base: &[(RepoPath, TelFile)],
    model: &TelosModel,
    change: &Change,
) -> Result<Vec<String>, TelosError> {
    if ws.config.test.cmd.trim().is_empty() {
        return Ok(Vec::new());
    }

    let required = required_witnesses(base, model, &change.ops);
    if required.is_empty() {
        return Ok(Vec::new());
    }

    let test_paths: Vec<RepoPath> = change
        .journal
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Run(run) => Some(run.test.path.clone()),
            JournalEntry::Bind { .. } => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let current = git.blob_oids(&test_paths)?;

    let mut warnings = Vec::new();
    for scenario in required {
        let (code, message, hint) = match witness_verdict(&change.journal, scenario, &current) {
            WitnessVerdict::Intact => continue,
            WitnessVerdict::MissingRed => (
                ErrorCode::TelosScenarioRedExpected,
                format!("scenario {scenario} has no sealed red witness"),
                format!("run `telos test {scenario}` to record a red witness before implementing"),
            ),
            WitnessVerdict::MissingGreen => (
                ErrorCode::TelosScenarioRedExpected,
                format!("scenario {scenario} has a red witness but no green run on the same bytes"),
                format!("run `telos test {scenario}` again once the implementation is in place"),
            ),
            // The message is the verdict's own -- it names the test file and
            // distinguishes "changed" from "no longer exists", and
            // only `witness_verdict` knows which red it judged.
            WitnessVerdict::Sealed(message) => (
                ErrorCode::TelosTestSealed,
                message,
                format!(
                    "the red witness is invalid; run `telos test {scenario}` again on the \
                     current bytes before reconciling"
                ),
            ),
        };

        if ws.config.policy.tdd == TddPolicy::Strict {
            return Err(TelosError::new(code, message).hint(hint));
        }
        warnings.push(message);
    }
    Ok(warnings)
}

// --- what this delta impacts --------------------------------------

/// The graph nodes a delta touches, plus everything that depends on them.
///
/// An `add`/`edit` names an entity that exists in the *post* model, so its
/// node and its reverse closure are read there. A `remove` names one that
/// does not, so what depended on it can only be read from the *pre* model --
/// which, because referential deletion safety is already enforced in gate 5,
/// is empty for a
/// legal removal, since nothing may still reference what is being removed.
/// It is computed anyway rather than assumed: the rule and this set are
/// enforced by different code, and the day one of them grows a legitimate
/// exception, the impacted set must not silently miss it.
///
/// An `accept` names a path, not an entity. When that path is bound by
/// `implements`, it is a `Code` node like any other and its dependents
/// re-verify; when it is not (a `telos.toml`, say), the graph knows no such
/// node and it contributes nothing.
///
/// Best-effort on the pre model: a sealed base that does not build on its
/// own -- the very hole this change may be closing -- contributes no
/// closure rather than failing the reconcile. Gate 5 already proved what
/// matters, that the *post* model is coherent.
fn impacted_nodes(ws: &Workspace, model: &TelosModel, ops: &[StagedOp]) -> BTreeSet<NodeRef> {
    // Configuration controls the runner, glob ownership, TDD policy, and the
    // execution environment of every constraint. It is therefore a global
    // mutation: every intent/scenario must be revalidated, not an entity-less
    // op that accidentally produces an empty impact set.
    if ops.iter().any(|op| matches!(op, StagedOp::EditConfig(_))) {
        return model
            .intents
            .keys()
            .copied()
            .map(NodeRef::Intent)
            .chain(model.scenario_owner.keys().copied().map(NodeRef::Scenario))
            .collect();
    }

    let pre = ops
        .iter()
        .any(is_remove)
        .then(|| parse_base(ws).ok().and_then(|base| build_model(base).ok()))
        .flatten();

    let mut nodes = BTreeSet::new();
    for op in ops {
        let (node, source) = match op {
            StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => {
                (NodeRef::Notion(n.name.clone()), Some(model))
            }
            StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => {
                (NodeRef::Intent(i.id), Some(model))
            }
            StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => {
                (NodeRef::Constraint(c.id), Some(model))
            }
            StagedOp::RemoveNotion(name) => (NodeRef::Notion(name.clone()), pre.as_ref()),
            StagedOp::RemoveIntent(id) => (NodeRef::Intent(*id), pre.as_ref()),
            StagedOp::RemoveConstraint(id) => (NodeRef::Constraint(*id), pre.as_ref()),
            StagedOp::EditConfig(_) => continue,
            StagedOp::Accept { path, .. } => (NodeRef::Code(path.clone()), Some(model)),
        };

        let Some(source) = source else { continue };
        for entry in source.graph.reverse_closure(&node) {
            nodes.insert(entry.node);
        }
        // A removed entity is not itself impacted -- it is gone.
        if !is_remove(op) {
            nodes.insert(node);
        }
    }
    nodes
}

fn is_remove(op: &StagedOp) -> bool {
    matches!(
        op,
        StagedOp::RemoveNotion(_) | StagedOp::RemoveIntent(_) | StagedOp::RemoveConstraint(_)
    )
}

/// The intents among `nodes` -- what a constraint's `scope` is matched
/// against.
fn impacted_intents(nodes: &BTreeSet<NodeRef>) -> BTreeSet<IntentId> {
    nodes
        .iter()
        .filter_map(|node| match node {
            NodeRef::Intent(id) => Some(*id),
            _ => None,
        })
        .collect()
}

/// The scenarios among `nodes`, plus every scenario of the intents among
/// them: editing an intent impacts its own scenarios even though no
/// edge points from the intent to them -- `verifies` runs the other way.
fn impacted_scenarios(model: &TelosModel, nodes: &BTreeSet<NodeRef>) -> BTreeSet<ScenarioId> {
    let mut scenarios = BTreeSet::new();
    for node in nodes {
        match node {
            NodeRef::Scenario(id) => {
                scenarios.insert(*id);
            }
            NodeRef::Intent(id) => {
                if let Some(intent) = model.intents.get(id) {
                    scenarios.extend(intent.scenarios.iter().map(|s| s.id));
                }
            }
            _ => {}
        }
    }
    scenarios
}

// --- gate 10: constraint checks ---------------------------------------

/// Runs the `check` of every global constraint and of every scoped one whose
/// scope meets an impacted intent, at the repository root.
///
/// `impacted` is `None` for a full reseal, which runs every constraint check:
/// a `scope` filters constraints against *a delta*, so with no delta to
/// filter against there is nothing to narrow -- every constraint that has a
/// `check` runs.
///
/// A constraint with no `check` is not a failure and is not counted: the
/// `rule` is then prose for a human, which this engine has no way to verify.
/// A command that cannot even be spawned is folded into the same
/// `TELOS_CONSTRAINT_FAILED` as one that exits non-zero -- from the
/// caller's side “the check did not pass” is one outcome with one remedy.
fn run_constraint_checks(
    ws: &Workspace,
    model: &TelosModel,
    impacted: Option<&BTreeSet<NodeRef>>,
) -> Result<u32, TelosError> {
    let intents = impacted.map(impacted_intents);

    let mut checks_run = 0;
    for (id, constraint) in &model.constraints {
        let Some(check) = &constraint.check else {
            continue;
        };
        if let Some(intents) = &intents
            && !in_scope(constraint, intents)
        {
            continue;
        }

        match run_shell(check, &ws.repo_root) {
            Ok(result) if result.status == 0 => checks_run += 1,
            Ok(_) | Err(_) => return Err(constraint_failed(*id, check)),
        }
    }
    Ok(checks_run)
}

fn in_scope(constraint: &Constraint, impacted: &BTreeSet<IntentId>) -> bool {
    match &constraint.scope {
        Scope::Global => true,
        Scope::Intents(ids) => ids.iter().any(|id| impacted.contains(&id.node)),
    }
}

/// The refusal for a failed check names the constraint and command.
///
/// The command's own output is deliberately *not* folded in. It is not
/// reproducible across machines (a git version, a locale, a `$PATH`), so a
/// message carrying it could not be a frozen contract -- and the hint,
/// frozen by `docs/contracts.md`, already says where to get it: run the
/// command.
fn constraint_failed(id: ConstraintId, command: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosConstraintFailed,
        format!("{id} check failed: `{command}`"),
    )
    .hint("Run the constraint's `check` command directly to see its output.")
}

// --- gate 11: tests ----------------------------------------------------

/// Runs `[test] cmd` once per distinct `proves` target of the impacted
/// scenarios, `{filter}` substituted with the target's test name (or, when
/// it names no single test, its path).
///
/// An empty `cmd` skips the whole gate and reports zero runs. Gate 9 already
/// proved that no active obligation requires one; draft-only/spec scaffolding
/// may therefore still reconcile without a runner. A run that fails is the
/// runtime half of the proof check --
/// a scenario the spec claims is proved, whose proof does not currently
/// pass -- and is reported as `TELOS_INTEGRITY_VIOLATION` carrying the
/// substituted command, so the caller can rerun exactly what ran.
fn run_tests(
    ws: &Workspace,
    model: &TelosModel,
    impacted: &BTreeSet<NodeRef>,
) -> Result<u32, TelosError> {
    let cmd = &ws.config.test.cmd;
    if cmd.trim().is_empty() {
        return Ok(0);
    }

    let scenarios = impacted_scenarios(model, impacted);
    // Keyed by the target's rendered form: that both deduplicates two
    // scenarios proved by one test and orders the runs deterministically.
    let mut targets: BTreeMap<String, &TestRef> = BTreeMap::new();
    for binding in &model.bindings {
        if let Binding::Proves { test, scenario } = binding
            && scenarios.contains(&scenario.node)
        {
            targets.insert(test.to_string(), test);
        }
    }

    let mut tests_run = 0;
    for (rendered, test) in targets {
        let filter = test
            .name
            .clone()
            .unwrap_or_else(|| test.path.as_str().to_string());
        match run_shell_with_filter(cmd, &filter, &ws.repo_root) {
            Ok(run) if run.result.status == 0 => tests_run += 1,
            Ok(run) => return Err(test_failed(&rendered, &run.command)),
            Err(_) => {
                let command = substitute_filter(cmd, &filter);
                return Err(test_failed(&rendered, &command));
            }
        }
    }
    Ok(tests_run)
}

/// A full reconcile invokes `[test] cmd` once with `{filter}` substituted by
/// nothing -- the whole suite, once.
///
/// A full reseal with active obligations proves the spec as it stands rather
/// than what a delta reached, so there is no per-target loop and nothing to
/// deduplicate: the project's own runner decides what "everything" means.
/// The caller skips this function entirely for a draft-only model. `cmd`
/// empty skips the gate and reports zero runs, exactly as in the per-change
/// path.
fn run_full_tests(ws: &Workspace) -> Result<u32, TelosError> {
    let cmd = &ws.config.test.cmd;
    if cmd.trim().is_empty() {
        return Ok(0);
    }

    let command = substitute_filter(cmd, "");
    match run_shell_with_filter(cmd, "", &ws.repo_root) {
        Ok(run) if run.result.status == 0 => Ok(1),
        Ok(_) | Err(_) => Err(test_failed("the whole suite", &command)),
    }
}

/// Names what was run and the command that ran for it -- the latter after
/// substitution, so a caller can rerun character-for-character what this
/// did. `target` is a `proves` target for a per-change reconcile and “the
/// whole suite” for a full one. The command's output is left out for the
/// same reason as in [`constraint_failed`]: it is not reproducible, so it
/// cannot be contract.
fn test_failed(target: &str, command: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosIntegrityViolation,
        format!("the test run for `{target}` failed: `{command}`"),
    )
    .hint("run the configured executable with the displayed arguments, then reconcile again")
}

// --- the writes ---------------------------------------------------------

/// Writes one op's target: the entity's canonical bytes for `add`/`edit`,
/// deletion for `remove`, nothing for `accept` (the bytes are already what
/// they must be -- gate 4 just proved it; the seal is what changes).
///
/// Ops are applied in staged order, so a change that adds an entity and then
/// removes it leaves no file, and one that removes and re-adds leaves the
/// re-added one: the same net effect the overlay computed, obtained the same
/// way -- order is data.
///
/// A `remove` whose file is already absent is not an error: an earlier op of
/// this very change may have been the only reason it would have existed.
fn apply_op(ws: &Workspace, op: &StagedOp) -> Result<(), TelosError> {
    let repo_fs = RepoFs::open(&ws.repo_root)?;
    let path = op.target_path();

    let content = match op {
        StagedOp::AddNotion(n) | StagedOp::EditNotion(n) => emit_file(&TelFile::Notion(n.clone())),
        StagedOp::AddIntent(i) | StagedOp::EditIntent(i) => emit_file(&TelFile::Intent(i.clone())),
        StagedOp::AddConstraint(c) | StagedOp::EditConstraint(c) => {
            emit_file(&TelFile::Constraint(c.clone()))
        }
        StagedOp::EditConfig(config) => crate::emit::emit_config(config)?,
        StagedOp::Accept { .. } => return Ok(()),
        StagedOp::RemoveNotion(_) | StagedOp::RemoveIntent(_) | StagedOp::RemoveConstraint(_) => {
            return repo_fs.remove_file(&path);
        }
    };

    repo_fs.write(&path, content.as_bytes())
}

/// Writes `telos/bindings.tel` from the folded model, through the
/// emitter like every other spec file this transaction writes.
///
/// The whole table is re-emitted rather than appended to: `model.bindings`
/// already holds what the sealed file said *plus* what the journal folded in
/// ([`fold_journal_bindings`]), and [`crate::emit::emit_bindings`] is the one
/// place that decides the file's order. So a reconcile that folds nothing
/// rewrites the file byte-identically, which is why this is unconditional
/// rather than guarded on "did the journal add anything".
///
/// The one case that is guarded: a model with no binding at all, in a
/// project that has no `bindings.tel` on disk either. Writing an empty file
/// there would add a spec file -- and a lock entry -- that nothing asked
/// for. (`telos init` creates the file empty, so this only ever spares a
/// tree that was assembled by hand.)
fn write_bindings(ws: &Workspace, model: &TelosModel) -> Result<(), TelosError> {
    let path = RepoPath::new(BINDINGS_PATH);
    let repo_fs = RepoFs::open(&ws.repo_root)?;
    if model.bindings.is_empty() && repo_fs.read_optional(&path)?.is_none() {
        return Ok(());
    }

    let content = emit_file(&TelFile::Bindings(model.bindings.clone()));
    repo_fs.write(&path, content.as_bytes())
}

#[cfg(test)]
mod tests {
    //! The decisions of this module that are pure functions of their
    //! arguments -- constraint scoping, removal detection, and the
    //! carry-over's rewriting of a seal. Everything else here is a
    //! transaction over a real workspace, a real git repository and a real
    //! shell, and is covered end to end in `crates/telos/tests/reconcile.rs`
    //! and `crates/telos/tests/adopt_revert.rs` -- where a partial failure,
    //! or a laundered edit, would actually be observable.

    use super::*;
    use crate::ids::{ConstraintId, NotionName};
    use crate::model::{ConstraintKind, Rule};
    use crate::span::{Sp, Span};

    fn constraint(scope: Scope) -> Constraint {
        Constraint {
            id: ConstraintId(3),
            kind: ConstraintKind::Architecture,
            title: "Hexagonal boundaries".to_string(),
            rule: Rule::Text("Keep them.".to_string()),
            scope,
            check: Some("true".to_string()),
        }
    }

    fn scoped(ids: &[u32]) -> Scope {
        Scope::Intents(
            ids.iter()
                .map(|n| Sp {
                    node: IntentId(*n),
                    span: Span::default(),
                })
                .collect(),
        )
    }

    fn intents(ids: &[u32]) -> BTreeSet<IntentId> {
        ids.iter().copied().map(IntentId).collect()
    }

    #[test]
    fn a_global_constraint_is_in_scope_of_every_delta() {
        // Including one that impacts no intent at all -- adding a lone
        // notion, say.
        assert!(in_scope(&constraint(Scope::Global), &intents(&[])));
        assert!(in_scope(&constraint(Scope::Global), &intents(&[42])));
    }

    #[test]
    fn a_scoped_constraint_needs_one_of_its_intents_impacted() {
        let c = constraint(scoped(&[17, 42]));
        assert!(in_scope(&c, &intents(&[42])), "one match is enough");
        assert!(!in_scope(&c, &intents(&[1])));
        assert!(!in_scope(&c, &intents(&[])));
    }

    #[test]
    fn is_remove_is_true_of_exactly_the_three_removals() {
        assert!(is_remove(&StagedOp::RemoveIntent(IntentId(42))));
        assert!(is_remove(&StagedOp::RemoveConstraint(ConstraintId(3))));
        assert!(is_remove(&StagedOp::RemoveNotion(
            NotionName::new("Invoice").unwrap()
        )));
        assert!(!is_remove(&StagedOp::Accept {
            path: RepoPath::new("telos/telos.toml"),
            oid: Oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string()),
        }));
        assert!(!is_remove(&StagedOp::AddConstraint(constraint(
            Scope::Global
        ))));
    }

    // --- the carry-over --------------------------------------------------

    fn oid(seed: &str) -> Oid {
        Oid(seed.repeat(40 / seed.len()))
    }

    fn lock_of(spec: &[(&str, Oid)], code: &[(&str, Oid)]) -> Lock {
        let spec: BTreeMap<RepoPath, Oid> = spec
            .iter()
            .map(|(path, oid)| (RepoPath::new(*path), oid.clone()))
            .collect();
        Lock {
            version: 1,
            tool: "telos test".to_string(),
            sealed_by: None,
            spec_digest: Lock::compute_digest(&spec),
            spec,
            code: code
                .iter()
                .map(|(path, oid)| (RepoPath::new(*path), oid.clone()))
                .collect(),
        }
    }

    fn carried(paths: &[&str]) -> BTreeSet<RepoPath> {
        paths.iter().map(|p| RepoPath::new(*p)).collect()
    }

    const INVOICE: &str = "telos/notions/Invoice.tel";
    const ROGUE: &str = "telos/notions/Rogue.tel";
    const CODE: &str = "src/billing/invoice.rs";

    #[test]
    fn carrying_nothing_over_leaves_the_seal_exactly_as_sealed() {
        let previous = lock_of(&[(INVOICE, oid("a"))], &[]);
        let sealed = lock_of(&[(INVOICE, oid("b"))], &[]);
        let mut fresh = sealed.clone();

        carry_over(&mut fresh, &previous, &carried(&[]));

        assert_eq!(fresh, sealed);
    }

    #[test]
    fn a_carried_over_path_keeps_its_previously_sealed_oid() {
        // The whole point: the drifted bytes `seal` just hashed never reach
        // the lock, so the drift is still drift after the reconcile.
        let previous = lock_of(&[(INVOICE, oid("a"))], &[(CODE, oid("c"))]);
        let mut fresh = lock_of(&[(INVOICE, oid("b"))], &[(CODE, oid("d"))]);

        carry_over(&mut fresh, &previous, &carried(&[INVOICE, CODE]));

        assert_eq!(fresh.spec[&RepoPath::new(INVOICE)], oid("a"));
        assert_eq!(fresh.code[&RepoPath::new(CODE)], oid("c"));
        assert_eq!(
            fresh.spec_digest, previous.spec_digest,
            "the digest must follow the map it digests"
        );
    }

    #[test]
    fn a_carried_over_path_the_previous_lock_never_held_stays_out() {
        // Carry-over of absence: an untracked file some other change
        // adopted is not sealed by *this* transaction, so it stays
        // untracked -- still drift, still claimed, still reviewable.
        let previous = lock_of(&[(INVOICE, oid("a"))], &[]);
        let mut fresh = lock_of(&[(INVOICE, oid("a")), (ROGUE, oid("b"))], &[]);

        carry_over(&mut fresh, &previous, &carried(&[ROGUE]));

        assert!(!fresh.spec.contains_key(&RepoPath::new(ROGUE)));
        assert_eq!(fresh, previous_with_tool(&previous, &fresh));
    }

    #[test]
    fn a_carried_over_path_is_restored_in_both_tables() {
        // A path the previous lock held as `code` cannot come back as
        // `spec`, nor the reverse: whichever table `seal` put it in, what
        // the lock ends up saying about it is what the previous lock said.
        let previous = lock_of(&[], &[(INVOICE, oid("a"))]);
        let mut fresh = lock_of(&[(INVOICE, oid("b"))], &[]);

        carry_over(&mut fresh, &previous, &carried(&[INVOICE]));

        assert!(fresh.spec.is_empty(), "the spec table must not hold it");
        assert_eq!(fresh.code[&RepoPath::new(INVOICE)], oid("a"));
    }

    // --- gate 7: the sealed code coverage ---------------------------

    fn model_binding(paths: &[&str]) -> TelosModel {
        TelosModel {
            bindings: paths
                .iter()
                .map(|path| Binding::Implements {
                    path: RepoPath::new(*path),
                    intent: Sp {
                        node: IntentId(42),
                        span: Span::default(),
                    },
                })
                .collect(),
            ..TelosModel::default()
        }
    }

    fn accept(path: &str) -> StagedOp {
        StagedOp::Accept {
            path: RepoPath::new(path),
            oid: oid("e"),
        }
    }

    #[test]
    fn a_sealed_code_path_no_binding_still_covers_refuses_the_reconcile() {
        // The ledger attack: `bindings.tel` shrank outside this delta, and
        // this delta does not stage it -- so nobody reviewed the shrink.
        let previous = lock_of(&[], &[(CODE, oid("c"))]);

        let error = require_no_coverage_shrink(&previous, &TelosModel::default(), &[])
            .expect_err("dropping a sealed code path must refuse");

        assert_eq!(error.code, ErrorCode::TelosIntegrityViolation);
        assert_eq!(
            error.message,
            format!(
                "sealing would drop `{CODE}` from the code table: no binding covers it \
                 and this change does not stage telos/bindings.tel"
            )
        );
        assert_eq!(
            error.hint.as_deref(),
            Some(
                "the bindings shrank outside this change; reconcile or abandon the change \
                 that claims telos/bindings.tel, or restore them with `telos revert`"
            )
        );
    }

    #[test]
    fn a_delta_that_stages_the_bindings_file_may_shrink_the_coverage() {
        // The `accept` op `adopt` stages *is* the review: a human approved a
        // delta whose whole content is the new bindings table.
        let previous = lock_of(&[], &[(CODE, oid("c"))]);
        let ops = [accept(BINDINGS_PATH), accept("telos/telos.toml")];

        assert!(require_no_coverage_shrink(&previous, &TelosModel::default(), &ops).is_ok());
    }

    #[test]
    fn a_sealed_code_path_the_model_still_binds_is_no_shrink() {
        // What the gate reads is coverage, not bytes: the file may have been
        // rewritten, moved between tables, or left alone.
        let previous = lock_of(&[], &[(CODE, oid("c"))]);
        let ops = [accept("telos/telos.toml")];

        assert!(require_no_coverage_shrink(&previous, &model_binding(&[CODE]), &ops).is_ok());
    }

    #[test]
    fn a_coverage_that_grew_is_no_shrink_either() {
        // Only `previous.code` is the obligation; a binding the delta adds
        // is nothing to answer for.
        let previous = lock_of(&[], &[]);

        assert!(require_no_coverage_shrink(&previous, &model_binding(&[CODE]), &[]).is_ok());
    }

    // --- gate 6's in-flight exemption -------------------------------------

    fn info(id: u32, claims: &[&str]) -> OpenChangeInfo {
        OpenChangeInfo {
            id: crate::ids::ChangeId(id),
            status: ChangeStatus::Implementing,
            claims: claims.iter().map(|p| RepoPath::new(*p)).collect(),
            obligations: Vec::new(),
        }
    }

    const STRAY: &str = "src/stray.rs";

    #[test]
    fn in_flight_paths_are_this_journals_and_every_other_changes_claims() {
        // `accept STRAY` is the witness that a change's *own op targets* are
        // not in flight: this transaction is applying them, so the unbound-code gate judges
        // the state they produce -- adopting a code file without binding it
        // is still an orphan.
        let change = Change {
            id: crate::ids::ChangeId(1),
            motivation: "x".to_string(),
            status: ChangeStatus::Implementing,
            approved_digest: None,
            ops: vec![StagedOp::RemoveIntent(IntentId(42)), accept(STRAY)],
            journal: vec![JournalEntry::Bind {
                path: RepoPath::new(CODE),
                intent: IntentId(42),
            }],
        };

        let paths = in_flight_paths(
            &change,
            &[info(1, &["telos/intents/INT-0099.tel"]), info(2, &[ROGUE])],
        );

        // Own journal, plus the *foreign* claims -- the caller's own entry
        // in `others` contributes nothing, exactly as in gate 1.
        assert_eq!(paths, carried(&[CODE, ROGUE]));
        assert!(
            !paths.contains(&RepoPath::new(STRAY)),
            "a path this change's own op targets must not be exempted from the unbound-code gate"
        );
    }

    /// `previous` with the fields a carry-over does not touch taken from
    /// `fresh` -- `version`, `tool` and `sealed_by` belong to the new seal,
    /// so comparing whole locks would otherwise fail on them.
    fn previous_with_tool(previous: &Lock, fresh: &Lock) -> Lock {
        Lock {
            version: fresh.version,
            tool: fresh.tool.clone(),
            sealed_by: fresh.sealed_by,
            ..previous.clone()
        }
    }
}
