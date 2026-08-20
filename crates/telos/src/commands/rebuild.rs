//! Read-only reconstruction planning and executable scenario progress.

use std::collections::BTreeMap;

use clap::Subcommand;
use serde_json::{Value, json};

use telos_core::error::{ErrorCode, TelosError};
use telos_core::exec::{run_shell, substitute_filter};
use telos_core::ids::{ChangeId, IntentId, RepoPath};
use telos_core::model::{Binding, Change, StagedOp, TelosModel, TestRef};
use telos_core::overlay::{apply_ops_idempotent, fold_journal_bindings, parse_base};
use telos_core::rebuild;
use telos_core::semantic::build_model;
use telos_core::workspace::Workspace;

use crate::commands::{Ctx, diagnostics_to_error, project, require_no_unclaimed_drift};
use crate::envelope::{CmdResult, Outcome};

#[derive(Debug, Subcommand)]
pub enum RebuildCommand {
    /// Emit every intent in deterministic prerequisite-first order.
    Plan,
    /// Run every scenario's proof targets and report current progress.
    Status,
}

pub fn run(ctx: &Ctx, command: &RebuildCommand) -> CmdResult {
    let input = load(ctx)?;
    match command {
        RebuildCommand::Plan => plan(&input),
        RebuildCommand::Status => status(&input),
    }
}

struct RebuildInput {
    ws: Workspace,
    model: TelosModel,
    owners: BTreeMap<IntentId, ChangeId>,
}

/// Loads the effective reconstruction model without writing it.
///
/// A missing lock is deliberate for these two commands only. When a lock is
/// present, the ordinary project preamble supplies the exact drift verdict;
/// a changing project then folds all parseable changes together before one
/// semantic pass judges their combined result.
fn load(ctx: &Ctx) -> Result<RebuildInput, TelosError> {
    let discovered = Workspace::discover(&ctx.cwd)?;
    if !discovered.lock_path().exists() {
        let model = discovered.load_model().map_err(diagnostics_to_error)?;
        return Ok(RebuildInput {
            ws: discovered,
            model,
            owners: BTreeMap::new(),
        });
    }

    let project = project(ctx)?;
    require_no_unclaimed_drift(&project)?;

    if project.parsed.is_empty() {
        let model = project.ws.load_model().map_err(diagnostics_to_error)?;
        return Ok(RebuildInput {
            ws: project.ws,
            model,
            owners: BTreeMap::new(),
        });
    }

    let base = parse_base(&project.ws).map_err(diagnostics_to_error)?;
    let owners = intent_owners(&project.parsed)?;
    let mut folded = base;
    for change in &project.parsed {
        folded = apply_ops_idempotent(folded, &change.ops);
    }
    for change in &project.parsed {
        folded = fold_journal_bindings(folded, change);
    }
    let model = build_model(folded).map_err(diagnostics_to_error)?;

    Ok(RebuildInput {
        ws: project.ws,
        model,
        owners,
    })
}

/// Records which change owns each added/edited intent and rejects the first
/// cross-change target collision instead of allowing later ops to win.
fn intent_owners(changes: &[Change]) -> Result<BTreeMap<IntentId, ChangeId>, TelosError> {
    let mut claims: BTreeMap<RepoPath, ChangeId> = BTreeMap::new();
    let mut owners = BTreeMap::new();

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

            match op {
                StagedOp::AddIntent(intent) | StagedOp::EditIntent(intent) => {
                    owners.insert(intent.id, change.id);
                }
                StagedOp::RemoveIntent(intent) => {
                    owners.remove(intent);
                }
                _ => {}
            }
        }
    }

    Ok(owners)
}

fn plan(input: &RebuildInput) -> CmdResult {
    let steps: Vec<Value> = rebuild::plan(&input.model)
        .into_iter()
        .enumerate()
        .map(|(index, step)| {
            let intent = input
                .model
                .intents
                .get(&step.intent)
                .expect("the planner only emits intents held by its model");
            let pack = crate::commands::context::build_pack(
                &input.model,
                intent,
                input.owners.get(&step.intent).copied(),
            );
            json!({
                "n": index + 1,
                "intent": step.intent,
                "requires": step.requires,
                "context": crate::commands::context::to_json(&pack),
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

        for test in proofs.values() {
            let filter = test.name.as_deref().unwrap_or_else(|| test.path.as_str());
            let command = substitute_filter(&runner, filter);
            let target_green = if input.ws.abs_path(&test.path).is_file() {
                run_shell(&command, &input.ws.repo_root)?.status == 0
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

/// Distinct proving targets in canonical locator order.
fn proofs_for(model: &TelosModel, id: telos_core::ids::ScenarioId) -> BTreeMap<String, TestRef> {
    model
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Proves { test, scenario } if scenario.node == id => {
                Some((test.to_string(), test.clone()))
            }
            _ => None,
        })
        .collect()
}

fn require_runner(ws: &Workspace) -> Result<String, TelosError> {
    let command = ws.config.test.cmd.trim();
    if command.is_empty() {
        return Err(TelosError::new(
            ErrorCode::TelosTestNotFound,
            "no `[test] cmd` is configured in telos/telos.toml",
        )
        .hint("set [test] cmd, e.g. `cargo test {filter}`"));
    }
    Ok(command.to_string())
}
