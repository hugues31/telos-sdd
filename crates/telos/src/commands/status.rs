//! `telos status`: reports whether the project matches its seal and how much
//! of the spec is covered by scenarios and bindings.
//!
//! Order matters: state ([`project`], which ends in `compute_state`) is
//! computed *first* and never parses a `.tel` file. It only compares Git blob
//! OIDs and reads open changes best-effort, so even a corrupted spec and a
//! corrupted change file still get a state answer. Coverage is loaded
//! best-effort afterward; if the spec does not parse, coverage is reported as
//! all zeros and `status` still exits 0.

use serde_json::{Value, json};

use telos_core::ids::ChangeId;
use telos_core::state::{Coverage, ProjectStateKind, StateReport, coverage, drift_token};

use crate::commands::{Ctx, Project, project, require_sealed_integrity};
use crate::envelope::{CmdResult, Outcome};

/// A [`Coverage`] with every counter at zero -- what `status` reports when
/// the spec fails to parse. `Coverage` has no `Default` of its own, so this
/// spells the public schema's zero value out.
const ZERO_COVERAGE: Coverage = Coverage {
    notions: 0,
    constraints: 0,
    intents_total: 0,
    intents_active: 0,
    scenarios_total: 0,
    scenarios_proved: 0,
    intents_implemented: 0,
};

pub fn run(ctx: &Ctx) -> CmdResult {
    let project = project(ctx)?;
    if project.state.state == ProjectStateKind::Coherent {
        require_sealed_integrity(&project)?;
    }
    let token = drift_token(
        &project.ws,
        &project.git,
        &project.lock,
        &project.state.drift,
    )?;
    let evidence = project.lock.proof_evidence.as_str();
    let unparseable = unparseable_change_ids(&project);
    let report = project.state;

    let cov = project
        .ws
        .load_model()
        .map(|model| coverage(&model))
        .unwrap_or(ZERO_COVERAGE);

    let drifted = !report.drift.is_empty();
    let drift_value = if drifted {
        let paths: Vec<&str> = report.drift.iter().map(|d| d.path.as_str()).collect();
        json!({ "paths": paths, "suggestion": "telos adopt", "token": token.clone() })
    } else {
        Value::Null
    };
    // One state, one suggestion: drift is more urgent than `changing`, and a
    // changing project's single next step is to inspect what is open. The
    // one addition is per change, not per state: a change whose file does
    // not parse can only be cleared by `abandon` (the workflow forbids
    // repairing the file by hand), so that command is spelled out for it --
    // otherwise `status` sends the caller to `change list`, which reports
    // the same obligation and suggests nothing, a loop with no way out.
    let next_actions = match report.state {
        ProjectStateKind::Drifted => vec![
            format!("telos adopt --expected-state {token}"),
            format!("telos revert --expected-state {token}"),
        ],
        ProjectStateKind::Changing => std::iter::once("telos change list".to_string())
            .chain(
                unparseable
                    .iter()
                    .map(|id| format!("telos change abandon {id}")),
            )
            .collect(),
        ProjectStateKind::Coherent => Vec::new(),
    };

    let result = json!({
        "state": report.state,
        "changes": report.open_changes,
        "drift": drift_value,
        "proof_evidence": evidence,
        "coverage": cov,
    });

    Ok(Outcome {
        result,
        human: human_summary(&report, drifted, cov, &token, evidence),
        next_actions,
    })
}

/// The ids of the open changes whose file did not parse, ascending: every
/// id the best-effort scan reported ([`Project::changes`]) that has no
/// parsed twin in [`Project::parsed`] -- which is `scan_changes`'s own
/// definition of an unparseable file, read back off the two halves it
/// produced in one pass rather than re-scanning the store.
fn unparseable_change_ids(project: &Project) -> Vec<ChangeId> {
    project
        .changes
        .iter()
        .map(|info| info.id)
        .filter(|id| !project.parsed.iter().any(|change| change.id == *id))
        .collect()
}

/// A compact, terse human-readable summary: the state, the drifted paths
/// (if any), one per line, then the coverage counters. Exact wording is
/// free -- there is no golden test for it, only for `--json`.
fn human_summary(
    report: &StateReport,
    drifted: bool,
    cov: Coverage,
    token: &str,
    evidence: &str,
) -> String {
    let state_name = match report.state {
        ProjectStateKind::Coherent => "coherent",
        ProjectStateKind::Changing => "changing",
        ProjectStateKind::Drifted => "drifted",
    };

    let mut lines = vec![format!("state: {state_name}")];
    if drifted {
        lines.push("drift:".to_string());
        for entry in &report.drift {
            lines.push(format!("  {}", entry.path));
        }
        lines.push(format!("drift token: {token}"));
    }
    if !report.open_changes.is_empty() {
        lines.push("changes:".to_string());
        for change in &report.open_changes {
            lines.push(format!("  {} {}", change.id, change.status));
        }
    }
    lines.push(format!("proof evidence: {evidence}"));
    lines.push(format!(
        "coverage: {} notions, {} constraints, {}/{} intents active, {}/{} intents implemented, {}/{} scenarios proved",
        cov.notions,
        cov.constraints,
        cov.intents_active,
        cov.intents_total,
        cov.intents_implemented,
        cov.intents_total,
        cov.scenarios_proved,
        cov.scenarios_total,
    ));
    lines.join("\n")
}
