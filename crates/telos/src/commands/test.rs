//! `telos test <SCN-id | --all>`: run one scenario's test and seal the
//! verdict as a witness in the change that owns it.
//!
//! The command is *mutating* -- it appends a `run` line to a change file --
//! so it runs every gate before writing anything:
//!
//! 1. the preamble ([`project`]: workspace, lock, repository, one store
//!    scan, state);
//! 2. the argument, parsed and resolved against every scenario id the spec
//!    or any open change declares;
//! 3. the **owner**: the open change whose delta stages the scenario --
//!    a witness belongs to the transaction that introduced what it
//!    witnesses, never to the project at large;
//! 4. that owner's status: `approved` or `implementing`, never a delta
//!    nobody has reviewed;
//! 5. a configured runner (`[test] cmd`);
//! 6. **discovery**: the `scn_NNNN` convention, or `--file`;
//! 7. the **drift gate with its carve-out** -- see
//!    [`require_no_foreign_drift`];
//! 8. the run itself, then the journal line and the `approved` →
//!    `implementing` transition, written together.
//!
//! Two rulings are worth spelling out, because neither is visible in the
//! code that implements them:
//!
//! - **`TELOS_FILE_CLAIMED` does not apply to journal writes.** Two changes
//!   may record runs against the same shared test file. A journal claim
//!   exists to make the drift of that file admissible, not to lock
//!   it: `add`/`edit`'s one-file-one-change gate is about *staging spec*,
//!   and reconcile's carry-over is what resolves an overlap. So nothing here
//!   calls `require_unclaimed`.
//! - **A run that executed no test records nothing.** With `[test] report`
//!   configured the verdict is the report's ([`run_proof`]): a testcase
//!   named after the scenario passed or failed, and anything else --
//!   report missing, invalid, no such testcase, only skipped ones -- is
//!   `TELOS_TEST_NOT_EXECUTED` with no journal line. Without a report the
//!   exit status alone decides, which cannot tell a zero-test run from
//!   green; the run line says so (`exit-status`) and so does the seal.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use serde_json::{Value, json};

use telos_core::changes::write_change;
use telos_core::config::TestCfg;
use telos_core::error::{ErrorCode, TelosError};
use telos_core::exec::{ProofRun, ProofVerdict, run_proof};
use telos_core::ids::{ChangeId, RepoPath, ScenarioId};
use telos_core::model::{
    Change, ChangeStatus, Evidence, JournalEntry, StagedOp, TelFile, TelosModel, TestRef, TestRun,
    Witness,
};
use telos_core::overlay::parse_base;
use telos_core::report::NotExecuted;
use telos_core::witness::{find_test_for, required_witnesses, scenario_pattern};

use crate::commands::{
    Ctx, Project, approved_config_workspace, diagnostics_to_error, is_approved, nearest_id,
    project, require_approved, require_no_foreign_drift, unknown,
};
use crate::envelope::{CmdResult, Outcome};

/// `telos test`, both shapes. clap guarantees exactly one of `scenario` and
/// `all` is present (`required_unless_present` / `conflicts_with`), so the
/// dispatch below never has to answer for the other two combinations.
pub fn run(
    ctx: &Ctx,
    scenario: Option<&str>,
    all: bool,
    file: Option<&str>,
    diagnostics: bool,
) -> CmdResult {
    let project = project(ctx)?;
    let file = file.map(RepoPath::parse_outside_telos).transpose()?;

    if all {
        return every(&project, file.as_ref(), diagnostics);
    }
    let scenario = scenario.expect("clap requires a scenario unless --all is given");
    one(&project, scenario, file.as_ref(), diagnostics)
}

// --- the single-scenario flow -----------------------------------------------

/// `telos test SCN-0108`: the eight steps of the module doc, once.
fn one(project: &Project, arg: &str, file: Option<&RepoPath>, diagnostics: bool) -> CmdResult {
    let scenario = parse_scenario_id(arg)?;
    require_known(project, scenario)?;

    let owner = owner_of(project, scenario).ok_or_else(|| no_owner(scenario))?;
    require_approved(owner)?;
    let effective_ws = approved_config_workspace(project)?;
    let runner = require_runner(&effective_ws)?;
    let test = find_test_for(&effective_ws, scenario, file)?;
    require_no_foreign_drift(project, std::slice::from_ref(&test.path))?;

    let mut change = owner.clone();
    let run = journal_run(project, &mut change, scenario, &test, &runner, diagnostics)?;

    Ok(Outcome {
        result: run_result(&run),
        human: human_line(&run),
        next_actions: next_actions(&run),
    })
}

// --- the --all flow ---------------------------------------------------------

/// `telos test --all`: every scenario the open, approved changes owe a
/// witness for (as computed by [`required_witnesses`]), ascending by id.
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
/// bury that under a batch of verdicts. A `TELOS_TEST_NOT_EXECUTED` verdict
/// aborts the loop like a discovery error would, after the runs already
/// taken were journalled.
///
/// `--file` applies to every target when given, which only makes sense for
/// a batch that really does live in one file; it exists here because the
/// flag is the command's, not the single-scenario shape's.
fn every(project: &Project, file: Option<&RepoPath>, diagnostics: bool) -> CmdResult {
    let targets = required_runs(project)?;
    if targets.is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosChangeStateInvalid,
            "no open change is implementing any scenario",
        )
        .hint("run `telos change list`"));
    }

    let effective_ws = approved_config_workspace(project)?;
    let runner = require_runner(&effective_ws)?;

    let mut resolved = Vec::with_capacity(targets.len());
    for (scenario, owner) in targets {
        let test = find_test_for(&effective_ws, scenario, file)?;
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
        runs.push(journal_run(
            project,
            change,
            scenario,
            &test,
            &runner,
            diagnostics,
        )?);
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
/// Per change, a scenario needs a witness when absent from the sealed base or
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
/// status and scenarios. An `add`/`edit` op carries both as a complete
/// post-state, not a patch. Building the *whole* folded model would mean
/// applying the delta and running
/// the semantic pass, which can fail for reasons that have nothing to do
/// with which scenarios owe a witness.
fn staged_intents(change: &Change) -> TelosModel {
    let mut model = TelosModel::default();
    for op in &change.ops {
        match op {
            StagedOp::AddOwnedIntent { intent, .. }
            | StagedOp::EditOwnedIntent { intent, .. }
            | StagedOp::AddIntent(intent)
            | StagedOp::EditIntent(intent) => {
                model.intents.insert(intent.id, intent.clone());
            }
            StagedOp::RemoveOwnedIntent { id, .. } | StagedOp::RemoveIntent(id) => {
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
/// spec alone would answer “unknown scenario” about an id the caller just
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
        if let TelFile::OwnedIntent { intent, .. } | TelFile::Intent(intent) = file {
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

/// The open change whose delta stages `scenario` owns the file.
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
            StagedOp::AddOwnedIntent { intent, .. }
            | StagedOp::EditOwnedIntent { intent, .. }
            | StagedOp::AddIntent(intent)
            | StagedOp::EditIntent(intent) => intent.scenarios.iter().any(|s| s.id == scenario),
            _ => false,
        })
    })
}

/// The frozen wording for a scenario no open change stages.
fn no_owner(scenario: ScenarioId) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("no open change is implementing {scenario}"),
    )
    .hint("stage it into a change and approve it first")
}

/// `[test]`, validated, with the runner trimmed -- or the frozen
/// `TELOS_TEST_NOT_FOUND` for a project that never wired a runner up.
///
/// Trimmed, so that a `cmd = "   "` is the same "no runner" as `cmd = ""`.
/// `validate_self` runs first: a `{report}` without a report, or a report
/// under `telos/`, is refused before anything could execute.
fn require_runner(ws: &telos_core::workspace::Workspace) -> Result<TestCfg, TelosError> {
    ws.config.validate_self()?;
    let cmd = ws.config.test.cmd.trim();
    if cmd.is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosTestNotFound,
            "no `[test] cmd` is configured in telos/telos.toml",
        )
        .hint("set [test] cmd, e.g. `cargo test {filter}`"));
    }
    Ok(TestCfg {
        cmd: cmd.to_string(),
        report: ws.config.test.report.clone(),
    })
}

// --- running and journalling --------------------------------------------------

/// What one recorded run reports, in both output modes.
struct RunReport {
    scenario: ScenarioId,
    witness: Witness,
    test: TestRef,
    change: ChangeId,
    /// The runner template's display form, with `{filter}` literally substituted. Execution
    /// uses the validated direct-process argv; this display is not a shell
    /// replay contract.
    command: String,
    evidence: Evidence,
    /// Testcases that ran, under report evidence.
    executed: Option<u32>,
}

/// Runs the scenario's test and writes the verdict into its owning change.
///
/// The filter is the discovered test's `name` when there is one and its path
/// otherwise, the same rule `reconcile` uses to filter a `proves` target:
/// the name is what a test runner selects on, and a file is the best
/// available answer when discovery found no identifier (an explicit
/// `--file`, where the file itself *is* the filter).
///
/// The oid is re-hashed here rather than carried from anywhere earlier: it
/// must be the bytes the run just saw, and the same git filters the seal
/// applies. The file existing is discovery's guarantee, so its absence
/// here is a filesystem race, reported as internal rather than as a
/// modelling question.
///
/// The `approved` → `implementing` transition rides along with the write:
/// the grammar requires a journalled change to be `implementing`, so
/// the line and the status can never be written apart. Idempotent -- a
/// change already `implementing` stays so, and its frozen digest is left
/// untouched because journal entries are digest-inert.
///
/// The verdict is judged only after the post-run hash check: a runner that
/// rewrote its proof is refused before the report is even read.
fn journal_run(
    project: &Project,
    change: &mut Change,
    scenario: ScenarioId,
    test: &TestRef,
    runner: &TestCfg,
    diagnostics: bool,
) -> Result<RunReport, TelosError> {
    let filter = test
        .name
        .clone()
        .unwrap_or_else(|| test.path.as_str().to_string());
    let mut before = project.git.blob_oids(std::slice::from_ref(&test.path))?;
    let oid = before.remove(&test.path).ok_or_else(|| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("the test file {} disappeared before the run", test.path),
        )
    })?;
    let execution = run_proof(runner, &filter, &project.ws.repo_root)?;
    if diagnostics {
        print_diagnostics(scenario, &execution);
    }
    let command = execution.command.clone();

    let after = project.git.blob_oids(std::slice::from_ref(&test.path))?;
    if after.get(&test.path) != Some(&oid) {
        return Err(TelosError::new(
            ErrorCode::TelosIntegrityViolation,
            format!(
                "the test file {} changed while its test was running",
                test.path
            ),
        )
        .hint("restore the intended test bytes and run `telos test` again"));
    }

    let (witness, executed) = match execution.verdict(scenario) {
        ProofVerdict::Green { executed } => (Witness::Green, executed),
        ProofVerdict::Red { executed } => (Witness::Red, executed),
        ProofVerdict::NotExecuted(reason) => {
            let report = execution
                .report_path()
                .expect("a not-executed verdict comes from a configured report");
            return Err(not_executed(scenario, report, &reason));
        }
    };
    let evidence = execution.kind();

    change.journal.push(JournalEntry::Run(TestRun {
        scenario,
        witness,
        test: test.clone(),
        oid,
        evidence,
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
        evidence,
        executed,
    })
}

/// A diagnostic sink never changes proof classification or journal admission.
/// Print before post-run validation so even a refused run remains diagnosable.
fn print_diagnostics(scenario: ScenarioId, execution: &ProofRun) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(
        stderr,
        "runner diagnostics for {scenario}\ncommand: {}\nexit status: {}\nstdout:\n{}\nstderr:\n{}",
        execution.command, execution.status, execution.stdout, execution.stderr
    );
}

/// The frozen `TELOS_TEST_NOT_EXECUTED` for a run that proved nothing: the
/// reason's own sentence, and a hint naming the scenario's pattern.
fn not_executed(scenario: ScenarioId, report: &RepoPath, reason: &NotExecuted) -> TelosError {
    TelosError::new(
        ErrorCode::TelosTestNotExecuted,
        reason.message(report, scenario),
    )
    .hint(format!(
        "make the runner execute the test named after `{}` and write the report, then run `telos test {scenario}` again",
        scenario_pattern(scenario)
    ))
}

/// One run's result object, shared by both shapes: `test SCN-…`
/// answers with it directly, `test --all` with an array of them.
fn run_result(run: &RunReport) -> Value {
    json!({
        "scenario": run.scenario,
        "witness": run.witness.as_str(),
        "test": run.test.to_string(),
        "change": run.change,
        "command": run.command,
        "evidence": run.evidence.as_str(),
        "executed": run.executed,
    })
}

fn human_line(run: &RunReport) -> String {
    let evidence = match run.executed {
        Some(1) => "1 test executed".to_string(),
        Some(n) => format!("{n} tests executed"),
        None => "exit status only".to_string(),
    };
    format!(
        "{} {}: {} (recorded in {}, {evidence})",
        run.scenario,
        run.witness.as_str(),
        run.test,
        run.change
    )
}

/// What to do next about one verdict: a red witness asks to be turned green,
/// a green one closes the loop and points at the reconcile.
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
