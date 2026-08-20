//! Read and transactionally stage project configuration.

use globset::Glob;
use serde::Deserialize;
use serde_json::json;

use telos_core::changes::{read_change, write_change};
use telos_core::config::{AgentHost, AgentsCfg, Config, Globs, Policy, TddPolicy, TestCfg};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::model::{ChangeStatus, StagedOp};
use telos_core::workspace::Workspace;

use crate::commands::change::parse_change_id;
use crate::commands::mutate::require_unclaimed;
use crate::commands::{Ctx, project, require_no_unclaimed_drift};
use crate::envelope::{CmdResult, Outcome};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigPayload {
    code: PayloadGlobs,
    tests: PayloadGlobs,
    test: PayloadTest,
    policy: PayloadPolicy,
    agents: PayloadAgents,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayloadGlobs {
    globs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayloadTest {
    cmd: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayloadPolicy {
    tdd: TddPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayloadAgents {
    hosts: Vec<AgentHost>,
}

pub fn run(ctx: &Ctx, change: Option<&str>, payload: Option<&str>) -> CmdResult {
    let ws = Workspace::discover(&ctx.cwd)?;
    let Some(change) = change else {
        return Ok(Outcome {
            result: json!(ws.config),
            human: telos_core::emit::emit_config(&ws.config)?,
            next_actions: Vec::new(),
        });
    };
    stage(ctx, change, payload.unwrap_or_default())
}

fn stage(ctx: &Ctx, change: &str, raw: &str) -> CmdResult {
    let id = parse_change_id(change)?;
    let project = project(ctx)?;
    require_no_unclaimed_drift(&project)?;
    let mut change = read_change(&project.ws, id)?;
    if !matches!(change.status, ChangeStatus::Open | ChangeStatus::Drafted) {
        return Err(TelosError::new(
            ErrorCode::TelosChangeStateInvalid,
            format!(
                "cannot stage configuration into {} change",
                change.status.as_str()
            ),
        ));
    }
    let payload: ConfigPayload = serde_json::from_str(raw).map_err(|_| {
        TelosError::new(
            ErrorCode::TelosParseError,
            "payload: expected a complete configuration JSON object",
        )
    })?;
    let mut config = Config {
        code: Globs {
            globs: payload.code.globs,
        },
        tests: Globs {
            globs: payload.tests.globs,
        },
        test: TestCfg {
            cmd: payload.test.cmd,
        },
        policy: Policy {
            tdd: payload.policy.tdd,
        },
        agents: AgentsCfg {
            hosts: payload.agents.hosts,
        },
    };
    config.normalize();
    if config.agents.hosts != project.ws.config.agents.hosts {
        return Err(TelosError::new(
            ErrorCode::TelosIntegrityViolation,
            "agents.hosts is managed by `telos init --agents` and cannot be changed by `telos config`",
        ));
    }
    for glob in config.code.globs.iter().chain(&config.tests.globs) {
        Glob::new(glob).map_err(|e| {
            TelosError::new(
                ErrorCode::TelosParseError,
                format!("invalid glob `{glob}`: {e}"),
            )
        })?;
    }
    let op = StagedOp::EditConfig(config.clone());
    require_unclaimed(&project, id, &op.target_path())?;
    if let Some(existing) = change
        .ops
        .iter_mut()
        .find(|op| matches!(op, StagedOp::EditConfig(_)))
    {
        *existing = op;
    } else {
        change.ops.push(op);
    }
    if change.status == ChangeStatus::Open {
        change.status = ChangeStatus::Drafted;
    }
    write_change(&project.ws, &change)?;
    Ok(Outcome {
        result: json!({ "change": id, "path": "telos/telos.toml", "config": config }),
        human: format!("{id}: edit config telos/telos.toml"),
        next_actions: vec![format!("telos change diff {id}")],
    })
}
