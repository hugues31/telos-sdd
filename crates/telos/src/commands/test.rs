//! `telos test <SCN-id | --all>`: run one scenario's test and seal the
//! verdict as a witness in the change that owns it (D5, D6).
//!
//! The command is *mutating* -- it appends a `run` line to a change file --
//! so it runs the gates in the order D6 freezes, and writes nothing until
//! every one of them has passed:
//!
//! 1. the preamble ([`project`]: workspace, lock, repository, one store
//!    scan, state);
//! 2. the argument, parsed and resolved against every scenario id the spec
//!    or any open change declares;
//! 3. the **owner** (D5): the open change whose delta stages the scenario --
//!    a witness belongs to the transaction that introduced what it
//!    witnesses, never to the project at large;
//! 4. that owner's status: `approved` or `implementing`, never a delta
//!    nobody has reviewed;
//! 5. a configured runner (`[test] cmd`);
//! 6. **discovery** (D4): the `scn_NNNN` convention, or `--file`;
//! 7. the **drift gate with its carve-out** (D6) -- see
//!    [`require_no_foreign_drift`];
//! 8. the run itself, then the journal line and the `approved` →
//!    `implementing` transition, written together.
//!
//! Two rulings are worth spelling out, because neither is visible in the
//! code that implements them:
//!
//! - **`TELOS_FILE_CLAIMED` does not apply to journal writes.** Two changes
//!   may record runs against the same shared test file. A journal claim
//!   exists to make the drift of that file admissible (D3/D6), not to lock
//!   it: `add`/`edit`'s one-file-one-change gate is about *staging spec*,
//!   and reconcile's carry-over is what resolves an overlap. So nothing here
//!   calls `require_unclaimed`.
//! - **Nothing detects a run that executed zero tests.** The runner is an
//!   arbitrary shell command whose output telos deliberately does not parse,
//!   so a `{filter}` that matches nothing exits 0 and reads as green. The
//!   identifier-boundary discovery of D4 is what keeps the filter honest in
//!   practice; the residual hazard is a documented M3 limitation.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use telos_core::changes::write_change;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::exec::{run_shell, substitute_filter};
use telos_core::ids::{ChangeId, RepoPath, ScenarioId};
use telos_core::model::{
    Change, ChangeStatus, JournalEntry, StagedOp, TelFile, TelosModel, TestRef, TestRun, Witness,
};
use telos_core::overlay::parse_base;
use telos_core::witness::{find_test_for, required_witnesses};

use crate::commands::{
    Ctx, Project, diagnostics_to_error, is_approved, nearest_id, project, require_approved,
    require_no_foreign_drift, unknown,
};
use crate::envelope::{CmdResult, Outcome};

/// `telos test`, both shapes. clap guarantees exactly one of `scenario` and
/// `all` is present (`required_unless_present` / `conflicts_with`), so the
/// dispatch below never has to answer for the other two combinations.
pub fn run(ctx: &Ctx, scenario: Option<&str>, all: bool, file: Option<&str>) -> CmdResult {
    let project = project(ctx)?;
    let file = file.map(RepoPath::new);

    if all {
        return every(&project, file.as_ref());
    }
    let scenario = scenario.expect("clap requires a scenario unless --all is given");
    one(&project, scenario, file.as_ref())
}

// --- the single-scenario flow -----------------------------------------------

/// `telos test SCN-0108`: the eight steps of the module doc, once.
fn one(project: &Project, arg: &str, file: Option<&RepoPath>) -> CmdResult {
    let scenario = parse_scenario_id(arg)?;
    require_known(project, scenario)?;

    let owner = owner_of(project, scenario).ok_or_else(|| no_owner(scenario))?;
    require_approved(owner)?;
    let cmd = require_runner(project)?;
    let test = find_test_for(&project.ws, scenario, file)?;
    require_no_foreign_drift(project, std::slice::from_ref(&test.path))?;

    let mut change = owner.clone();
    let run = journal_run(project, &mut change, scenario, &test, &cmd)?;

    Ok(Outcome {
        result: run_result(&run),
        human: human_line(&run),
        next_actions: next_actions(&run),
    })
}

// --- the --all flow ---------------------------------------------------------

/// `telos test --all`: every scenario the open, approved changes owe a
/// witness for (D7's scope, [`required_witnesses`]), ascending by id.
///
/// Discovery for *every* target runs before the first shell command, so the
/// carve-out is judged once, against the whole set of test files this
/// invocation is about to claim -- judging it per run would let the second
/// run trip over drift the first one legitimately introduced.
///
/// A red run is not a failure of the command: a witness is the outcome
/// either way, so the loop runs every target and only an infrastructure
/// error (a shell that will not spawn, a change file that will not be
/// written) aborts it. Each run is journalled as it is taken rather than in
/// one final write: evidence already gathered survives whatever the next
/// target does. A target whose test cannot be *discovered* is a different
/// matter and does abort, before anything has run: it says the loop is not
/// at the stage `--all` is for, and running the other scenarios first would
/// bury that under a batch of verdicts.
///
/// `--file` applies to every target when given, which only makes sense for
/// a batch that really does live in one file; it exists here because the
/// flag is the command's, not the single-scenario shape's.
fn every(project: &Project, file: Option<&RepoPath>) -> CmdResult {
    let targets = required_runs(project)?;
    if targets.is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosChangeStateInvalid,
            "no open change is implementing any scenario",
        )
        .hint("run `telos change list`"));
    }

    let cmd = require_runner(project)?;

    let mut resolved = Vec::with_capacity(targets.len());
    for (scenario, owner) in targets {
        let test = find_test_for(&project.ws, scenario, file)?;
        resolved.push((scenario, owner, test));
    }
    let claimed: Vec<RepoPath> = resolved.iter().map(|(_, _, t)| t.path.clone()).collect();
    require_no_foreign_drift(project, &claimed)?;

    // One in-memory copy per owning change, so two runs on the same change
    // append to the same journal instead of each overwriting the other's.
    let mut owners: BTreeMap<ChangeId, Change> = BTreeMap::new();
    for (_, owner, _) in &resolved {
        owners.entry(*owner).or_insert_with(|| {
            project
                .parsed
                .iter()
                .find(|change| change.id == *owner)
                .expect("every target's owner came from this same scan")
                .clone()
        });
    }

    let mut runs = Vec::with_capacity(resolved.len());
    for (scenario, owner, test) in resolved {
        let change = owners
            .get_mut(&owner)
            .expect("every target's owner came from the same scan");
        runs.push(journal_run(project, change, scenario, &test, &cmd)?);
    }

    let human: Vec<String> = runs.iter().map(human_line).collect();
    let result: Vec<Value> = runs.iter().map(run_result).collect();

    Ok(Outcome {
        result: json!({ "runs": result }),
        human: human.join("\n"),
        next_actions: Vec::new(),
    })
}

/// Every `(scenario, owning change)` pair `--all` has to run, ascending by
/// scenario id.
///
/// The scope is D7's, per change: a scenario absent from the sealed base, or
/// present but whose canonical fragment the delta moved. That is exactly
/// what a change owes a red/green pair for at reconcile, so `--all` and the
/// reconcile gate can never disagree about which scenarios are in play.
fn required_runs(project: &Project) -> Result<Vec<(ScenarioId, ChangeId)>, TelosError> {
    let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;

    let mut targets: Vec<(ScenarioId, ChangeId)> = Vec::new();
    for change in &project.parsed {
        if !is_approved(change) {
            continue;
        }
        let post = staged_intents(change);
        for scenario in required_witnesses(&base, &post, &change.ops) {
            targets.push((scenario, change.id));
        }
    }
    targets.sort_by_key(|(scenario, _)| scenario.0);
    Ok(targets)
}

/// The post-state of every intent a change stages, as the minimal
/// [`TelosModel`] [`required_witnesses`] reads.
///
/// Only `intents` is populated, because that is the only field
/// `required_witnesses` looks at -- it asks each staged intent for its post
/// status and its scenarios, both of which an `add`/`edit` op carries whole
/// (Annex C: an op is the complete post-state, not a patch). Building the
/// *whole* folded model instead would mean applying the delta and running
/// the semantic pass, which can fail for reasons that have nothing to do
/// with which scenarios owe a witness.
fn staged_intents(change: &Change) -> TelosModel {
    let mut model = TelosModel::default();
    for op in &change.ops {
        match op {
            StagedOp::AddIntent(intent) | StagedOp::EditIntent(intent) => {
                model.intents.insert(intent.id, intent.clone());
            }
            StagedOp::RemoveIntent(id) => {
                model.intents.remove(id);
            }
            _ => {}
        }
    }
    model
}

// --- the gates ---------------------------------------------------------------

/// A `SCN-NNNN` argument, or `TELOS_REFERENCE_UNKNOWN` naming what was
/// expected -- the same policy as every other typed id argument (`show`,
/// `change abandon`, the staging verbs).
fn parse_scenario_id(arg: &str) -> Result<ScenarioId, TelosError> {
    arg.parse::<ScenarioId>().map_err(|_| {
        TelosError::new(
            ErrorCode::TelosReferenceUnknown,
            format!("cannot parse `{arg}` as a scenario id"),
        )
    })
}

/// Refuses an id nothing declares, with the nearest one that is declared.
///
/// "Declared" spans both worlds a scenario can live in: the spec files on
/// disk, and the deltas of the open changes -- a scenario staged an hour ago
/// exists as far as its author is concerned, and is precisely the one
/// `telos test` is about to be pointed at. Judging it against the sealed
/// spec alone would answer «unknown scenario» about an id the caller just
/// watched `edit intent` allocate.
fn require_known(project: &Project, scenario: ScenarioId) -> Result<(), TelosError> {
    let known = known_scenarios(project)?;
    if known.contains(&scenario) {
        return Ok(());
    }
    Err(unknown(
        "scenario",
        scenario,
        nearest_id(scenario.0, known.iter().map(|id| id.0), |n| {
            ScenarioId(n).to_string()
        }),
    ))
}

/// Every scenario id the sealed spec or any open change declares.
fn known_scenarios(project: &Project) -> Result<BTreeSet<ScenarioId>, TelosError> {
    let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;

    let mut known: BTreeSet<ScenarioId> = BTreeSet::new();
    for (_, file) in &base {
        if let TelFile::Intent(intent) = file {
            known.extend(intent.scenarios.iter().map(|s| s.id));
        }
    }
    for change in &project.parsed {
        for intent in staged_intents(change).intents.values() {
            known.extend(intent.scenarios.iter().map(|s| s.id));
        }
    }
    Ok(known)
}

/// D5's ownership rule: the open change whose delta stages `scenario`.
///
/// Only `add`/`edit intent` can own a scenario -- a scenario exists inside
/// an intent, so an op that carries no intent carries no scenario either
/// (an `accept` that claims an intent's *file* stages no content and owns
/// nothing). `Project::parsed` is ascending by id, so two changes that
/// somehow staged the same scenario resolve to the lower one,
/// deterministically.
fn owner_of(project: &Project, scenario: ScenarioId) -> Option<&Change> {
    project.parsed.iter().find(|change| {
        change.ops.iter().any(|op| match op {
            StagedOp::AddIntent(intent) | StagedOp::EditIntent(intent) => {
                intent.scenarios.iter().any(|s| s.id == scenario)
            }
            _ => false,
        })
    })
}

/// The frozen wording for a scenario no open change stages (Annex F).
fn no_owner(scenario: ScenarioId) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("no open change is implementing {scenario}"),
    )
    .hint("stage it into a change and approve it first")
}

/// `[test] cmd`, or the frozen `TELOS_TEST_NOT_FOUND` for a project that
/// never wired a runner up (Annex F).
///
/// Trimmed, so that a `cmd = "   "` is the same "no runner" as `cmd = ""`:
/// both would run the shell on nothing and report a meaningless verdict.
fn require_runner(project: &Project) -> Result<String, TelosError> {
    let cmd = project.ws.config.test.cmd.trim();
    if cmd.is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosTestNotFound,
            "no `[test] cmd` is configured in telos/telos.toml",
        )
        .hint("set [test] cmd, e.g. `cargo test {filter}`"));
    }
    Ok(cmd.to_string())
}

// --- running and journalling --------------------------------------------------

/// What one recorded run reports, in both output modes.
struct RunReport {
    scenario: ScenarioId,
    witness: Witness,
    test: TestRef,
    change: ChangeId,
    /// The command as it actually ran -- `{filter}` already substituted --
    /// so a caller can rerun character for character what this did.
    command: String,
}

/// Runs the scenario's test and writes the verdict into `change` (D1, D5).
///
/// The filter is the discovered test's `name` when there is one and its path
/// otherwise, the same rule `reconcile` uses to filter a `proves` target:
/// the name is what a test runner selects on, and a file is the best
/// available answer when discovery found no identifier (an explicit
/// `--file`, where the file itself *is* the filter).
///
/// The oid is re-hashed here rather than carried from anywhere earlier: it
/// must be the bytes the run just saw, and the same git filters the seal
/// applies (D1). The file existing is discovery's guarantee, so its absence
/// here is a filesystem race, reported as internal rather than as a
/// modelling question.
///
/// The `approved` → `implementing` transition rides along with the write
/// (D5): the grammar requires a journalled change to be `implementing`, so
/// the line and the status can never be written apart. Idempotent -- a
/// change already `implementing` stays so, and its frozen digest is left
/// untouched (the journal is digest-inert, D1).
fn journal_run(
    project: &Project,
    change: &mut Change,
    scenario: ScenarioId,
    test: &TestRef,
    cmd: &str,
) -> Result<RunReport, TelosError> {
    let filter = test
        .name
        .clone()
        .unwrap_or_else(|| test.path.as_str().to_string());
    let command = substitute_filter(cmd, &filter);

    let outcome = run_shell(&command, &project.ws.repo_root)?;
    let witness = if outcome.status == 0 {
        Witness::Green
    } else {
        Witness::Red
    };

    let mut oids = project.git.blob_oids(std::slice::from_ref(&test.path))?;
    let oid = oids.remove(&test.path).ok_or_else(|| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "the test file {} disappeared between its discovery and the run",
                test.path
            ),
        )
    })?;

    change.journal.push(JournalEntry::Run(TestRun {
        scenario,
        witness,
        test: test.clone(),
        oid,
    }));
    if change.status == ChangeStatus::Approved {
        change.status = ChangeStatus::Implementing;
    }
    write_change(&project.ws, change)?;

    Ok(RunReport {
        scenario,
        witness,
        test: test.clone(),
        change: change.id,
        command,
    })
}

/// One run's result object (Annex C), shared by both shapes: `test SCN-…`
/// answers with it directly, `test --all` with an array of them.
fn run_result(run: &RunReport) -> Value {
    json!({
        "scenario": run.scenario,
        "witness": run.witness.as_str(),
        "test": run.test.to_string(),
        "change": run.change,
        "command": run.command,
    })
}

fn human_line(run: &RunReport) -> String {
    format!(
        "{} {}: {} (recorded in {})",
        run.scenario,
        run.witness.as_str(),
        run.test,
        run.change
    )
}

/// What to do next about one verdict: a red witness asks to be turned green,
/// a green one closes the loop and points at the reconcile (Annex C).
///
/// `--all` suggests nothing at all rather than a list of these: several runs
/// with several verdicts have no single next step, and inventing one would
/// make an agent act on the last run in the batch.
fn next_actions(run: &RunReport) -> Vec<String> {
    match run.witness {
        Witness::Red => vec![format!("telos test {}", run.scenario)],
        Witness::Green => vec![format!("telos change reconcile {}", run.change)],
    }
}
