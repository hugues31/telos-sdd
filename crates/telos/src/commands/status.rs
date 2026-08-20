//! `telos status`: reports whether the project matches its seal, and how
//! much of the spec is covered by scenarios and bindings (Annex B's frozen
//! `status --json` schema).
//!
//! Order matters, and is spelled out by the task: state ([`project`], which
//! ends in `compute_state`) is computed *first*, and never parses a `.tel`
//! spec file (spec §6) -- it only compares git blob OIDs, and reads the open
//! changes best-effort, so even a corrupted spec *and* a corrupted change
//! file still get a state answer (D15). Loading
//! the model for `coverage` is best-effort *after* that: if the spec fails
//! to parse, `coverage` is reported as all zeros rather than blocking the
//! command. Annex B's "coverage computed over what parses" is ambiguous
//! about a spec that doesn't parse *at all*; all-zeros is the deterministic
//! reading adopted here, and `status` still exits 0 -- it reports, it does
//! not fail.

use serde_json::{Value, json};

use telos_core::state::{Coverage, ProjectStateKind, StateReport, coverage, drift_token};

use crate::commands::{Ctx, project, require_sealed_integrity};
use crate::envelope::{CmdResult, Outcome};

/// A [`Coverage`] with every counter at zero -- what `status` reports when
/// the spec fails to parse. `Coverage` has no `Default` of its own (Annex
/// B's freeze doesn't grant it one), so this spells the zero value out.
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
    let token = drift_token(&project.lock, &project.state.drift);
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
    // One state, one suggestion: drift is the more urgent of the two (D15
    // ranks it above `changing` for exactly that reason), and a `changing`
    // project's single next step is to look at what is open.
    let next_actions = match report.state {
        ProjectStateKind::Drifted => vec![
            format!("telos adopt --expected-state {token}"),
            format!("telos revert --expected-state {token}"),
        ],
        ProjectStateKind::Changing => vec!["telos change list".to_string()],
        ProjectStateKind::Coherent => Vec::new(),
    };

    let result = json!({
        "state": report.state,
        "changes": report.open_changes,
        "drift": drift_value,
        "coverage": cov,
    });

    Ok(Outcome {
        result,
        human: human_summary(&report, drifted, cov, &token),
        next_actions,
    })
}

/// A compact, terse human-readable summary: the state, the drifted paths
/// (if any), one per line, then the coverage counters. Exact wording is
/// free -- there is no golden test for it, only for `--json`.
fn human_summary(report: &StateReport, drifted: bool, cov: Coverage, token: &str) -> String {
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
