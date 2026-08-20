//! Read-only reconstruction planning and executable scenario progress.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use clap::Subcommand;
use serde_json::{Value, json};

use telos_core::error::{ErrorCode, TelosError};
use telos_core::exec::{run_shell_with_filter, substitute_filter};
use telos_core::ids::{IntentId, RepoPath, ScenarioId};
use telos_core::model::{Binding, Change, TelosModel, TestRef};
use telos_core::overlay::{apply_ops_idempotent, fold_journal_bindings, parse_base};
use telos_core::rebuild;
use telos_core::semantic::build_model;
use telos_core::witness::find_test_for;
use telos_core::workspace::Workspace;

use crate::commands::{
    Ctx, approved_config_workspace, diagnostics_to_error, project, require_no_unclaimed_drift,
};
use crate::envelope::{CmdResult, Outcome};

#[derive(Debug, Subcommand)]
pub enum RebuildCommand {
    /// Emit every intent in deterministic prerequisite-first order.
    Plan,
    /// Run every scenario's proof targets and report current progress.
    Status,
}

pub fn run(ctx: &Ctx, command: &RebuildCommand) -> CmdResult {
    let input = load(ctx, matches!(command, RebuildCommand::Plan))?;
    match command {
        RebuildCommand::Plan => plan(&input),
        RebuildCommand::Status => status(&input),
    }
}

struct RebuildInput {
    ws: Workspace,
    model: TelosModel,
    contexts: BTreeMap<IntentId, Value>,
}

/// Loads the effective reconstruction model without writing it.
///
/// A missing lock is deliberate for these two commands only. When a lock is
/// present, the ordinary project preamble supplies the exact drift verdict;
/// a changing project then folds all parseable changes together before one
/// semantic pass judges their combined result.
fn load(ctx: &Ctx, include_contexts: bool) -> Result<RebuildInput, TelosError> {
    let discovered = Workspace::discover(&ctx.cwd)?;
    let lock_path = discovered.lock_path();
    let has_lock_entry = match fs::symlink_metadata(&lock_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(TelosError::new(
                ErrorCode::TelosInternal,
                format!("failed to access {}: {error}", lock_path.display()),
            ));
        }
    };
    if !has_lock_entry {
        let model = discovered.load_model().map_err(diagnostics_to_error)?;
        let contexts = if include_contexts {
            model
                .intents
                .values()
                .map(|intent| {
                    let pack = crate::commands::context::build_pack(&model, intent, None);
                    (intent.id, crate::commands::context::to_json(&pack))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        return Ok(RebuildInput {
            ws: discovered,
            model,
            contexts,
        });
    }

    let project = project(ctx)?;
    require_no_unclaimed_drift(&project)?;
    require_parseable_changes(&project)?;
    let effective_ws = approved_config_workspace(&project)?;
    let disk = project.ws.load_model().map_err(diagnostics_to_error)?;

    if project.parsed.is_empty() {
        let contexts = if include_contexts {
            disk.intents
                .keys()
                .map(|id| {
                    let pack = crate::commands::context::pack_for_intent(&project, &disk, *id)?;
                    Ok((*id, crate::commands::context::to_json(&pack)))
                })
                .collect::<Result<BTreeMap<_, _>, TelosError>>()?
        } else {
            BTreeMap::new()
        };
        return Ok(RebuildInput {
            ws: effective_ws,
            model: disk,
            contexts,
        });
    }

    let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;
    require_no_conflicting_claims(&project.parsed)?;
    let mut folded = base;
    for change in &project.parsed {
        folded = apply_ops_idempotent(folded, &change.ops);
    }
    for change in &project.parsed {
        folded = fold_journal_bindings(folded, change);
    }
    let model = build_model(folded).map_err(diagnostics_to_error)?;
    let contexts = if include_contexts {
        model
            .intents
            .keys()
            .map(|id| {
                let pack = crate::commands::context::pack_for_intent(&project, &disk, *id)?;
                Ok((*id, crate::commands::context::to_json(&pack)))
            })
            .collect::<Result<BTreeMap<_, _>, TelosError>>()?
    } else {
        BTreeMap::new()
    };

    Ok(RebuildInput {
        ws: effective_ws,
        model,
        contexts,
    })
}

fn require_parseable_changes(project: &crate::commands::Project) -> Result<(), TelosError> {
    let parsed: BTreeSet<_> = project.parsed.iter().map(|change| change.id).collect();
    if let Some(invalid) = project
        .changes
        .iter()
        .find(|change| !parsed.contains(&change.id))
    {
        return telos_core::changes::read_change(&project.ws, invalid.id).map(|_| ());
    }
    Ok(())
}

/// Rejects the first cross-change target collision instead of allowing later
/// idempotent ops to win silently.
fn require_no_conflicting_claims(changes: &[Change]) -> Result<(), TelosError> {
    let mut claims = BTreeMap::new();

    for change in changes {
        for op in &change.ops {
            let path = op.target_path();
            if let Some(first) = claims.get(&path)
                && *first != change.id
            {
                return Err(TelosError::new(
                    ErrorCode::TelosIntegrityViolation,
                    format!("{path} is claimed by both {first} and {}", change.id),
                ));
            }
            claims.insert(path, change.id);
        }
    }

    Ok(())
}

fn plan(input: &RebuildInput) -> CmdResult {
    let steps: Vec<Value> = rebuild::plan(&input.model)
        .into_iter()
        .enumerate()
        .map(|(index, step)| {
            json!({
                "n": index + 1,
                "intent": step.intent,
                "requires": step.requires,
                "context": input.contexts.get(&step.intent)
                    .expect("plan admission built one public pack per planned intent"),
            })
        })
        .collect();
    let human = steps
        .iter()
        .map(|step| {
            format!(
                "{}. {}",
                step["n"],
                step["intent"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(Outcome {
        result: json!({ "steps": steps }),
        human,
        next_actions: Vec::new(),
    })
}

fn status(input: &RebuildInput) -> CmdResult {
    let runner = require_runner(&input.ws)?;
    let mut green_count = 0usize;
    let mut scenarios = Vec::with_capacity(input.model.scenario_owner.len());

    for scenario in input.model.scenario_owner.keys() {
        let proofs = proofs_for(&input.model, *scenario);
        let mut tests = Vec::with_capacity(proofs.len());
        let mut green = !proofs.is_empty();

        for test in &proofs {
            let filter = test.name.as_deref().unwrap_or_else(|| test.path.as_str());
            let command = substitute_filter(&runner, filter);
            let target_green = if proof_resolves(&input.ws, *scenario, test)? {
                run_shell_with_filter(&runner, filter, &input.ws.repo_root)?
                    .result
                    .status
                    == 0
            } else {
                false
            };
            green &= target_green;
            tests.push(json!({
                "test": test.to_string(),
                "green": target_green,
                "command": command,
            }));
        }

        if green {
            green_count += 1;
        }
        scenarios.push(json!({
            "id": scenario,
            "green": green,
            "tests": tests,
        }));
    }

    let total = scenarios.len();
    Ok(Outcome {
        result: json!({
            "scenarios_green": green_count,
            "scenarios_total": total,
            "scenarios": scenarios,
        }),
        human: format!("scenarios: {green_count}/{total} green"),
        next_actions: Vec::new(),
    })
}

/// Distinct proving targets in structural `(path, optional name)` order.
fn proofs_for(model: &TelosModel, id: ScenarioId) -> Vec<TestRef> {
    let mut proofs: Vec<TestRef> = model
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Proves { test, scenario } if scenario.node == id => Some(test.clone()),
            _ => None,
        })
        .collect();
    proofs.sort_by(|a, b| {
        (a.path.as_str(), a.name.as_deref()).cmp(&(b.path.as_str(), b.name.as_deref()))
    });
    proofs.dedup();
    proofs
}

/// A proof target resolves only inside the repository and outside `telos/`.
/// Named targets must additionally match the exact identifier discovered by
/// the established `scn_NNNN` boundary scan.
fn proof_resolves(
    ws: &Workspace,
    scenario: ScenarioId,
    test: &TestRef,
) -> Result<bool, TelosError> {
    if !is_safe_test_path(test.path.as_str()) {
        return Ok(false);
    }

    if ws.read_optional_bytes(&test.path)?.is_none() {
        return Ok(false);
    }

    if test.name.is_some() {
        let discovered = find_test_for(ws, scenario, Some(&test.path))?;
        return Ok(discovered.name == test.name);
    }
    Ok(true)
}

fn is_safe_test_path(raw: &str) -> bool {
    RepoPath::parse_outside_telos(raw).is_ok()
}

fn require_runner(ws: &Workspace) -> Result<String, TelosError> {
    if ws.config.test.cmd.trim().is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosTestNotFound,
            "no `[test] cmd` is configured in telos/telos.toml",
        )
        .hint("set [test] cmd, e.g. `cargo test {filter}`"));
    }
    Ok(ws.config.test.cmd.clone())
}
