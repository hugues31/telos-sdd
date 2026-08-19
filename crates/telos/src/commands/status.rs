//! `telos status`: reports whether the project matches its seal, and how
//! much of the spec is covered by scenarios and bindings (Annex B's frozen
//! `status --json` schema).
//!
//! Order matters, and is spelled out by the task: [`compute_state`] runs
//! *first*, and never parses a `.tel` file (spec §6) -- it only compares
//! git blob OIDs -- so a corrupted spec still gets a state answer. Loading
//! the model for `coverage` is best-effort *after* that: if the spec fails
//! to parse, `coverage` is reported as all zeros rather than blocking the
//! command. Annex B's "coverage computed over what parses" is ambiguous
//! about a spec that doesn't parse *at all*; all-zeros is the deterministic
//! reading adopted here, and `status` still exits 0 -- it reports, it does
//! not fail.

use serde_json::{Value, json};

use telos_core::git::GitRepo;
use telos_core::state::{Coverage, ProjectStateKind, StateReport, compute_state, coverage};
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, require_lock};
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
    let ws = Workspace::discover(&ctx.cwd)?;
    let lock = require_lock(&ws)?;
    let git = GitRepo::discover(&ctx.cwd)?;

    // `&[]`: `open_change_infos(ws)` is wired in T5, which also owns the
    // `status --json` `changes` field this currently-empty slice keeps
    // frozen at `[]` below.
    let report = compute_state(&ws, &lock, &git, &[])?;
    let cov = ws
        .load_model()
        .map(|model| coverage(&model))
        .unwrap_or(ZERO_COVERAGE);

    let drifted = !report.drift.is_empty();
    let drift_value = if drifted {
        let paths: Vec<&str> = report.drift.iter().map(|d| d.path.as_str()).collect();
        json!({ "paths": paths, "suggestion": "telos adopt" })
    } else {
        Value::Null
    };
    let next_actions = if drifted {
        vec!["telos adopt".to_string(), "telos revert".to_string()]
    } else {
        Vec::new()
    };

    let result = json!({
        "state": report.state,
        "changes": Vec::<Value>::new(),
        "drift": drift_value,
        "coverage": cov,
    });

    Ok(Outcome {
        result,
        human: human_summary(&report, drifted, cov),
        next_actions,
    })
}

/// A compact, terse human-readable summary: the state, the drifted paths
/// (if any), one per line, then the coverage counters. Exact wording is
/// free -- there is no golden test for it, only for `--json`.
fn human_summary(report: &StateReport, drifted: bool, cov: Coverage) -> String {
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
