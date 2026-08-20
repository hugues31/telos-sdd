//! Agent-host installation and the shared preventive guard.

pub mod assets;
mod claude;
mod codex;
pub mod guard;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use clap::ValueEnum;
use serde_json::{Map, Value};
use telos_core::error::{ErrorCode, TelosError};

/// Agent hosts supported by `telos init --agents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum AgentHost {
    Claude,
    Codex,
}

impl AgentHost {
    fn matcher(self) -> &'static str {
        match self {
            Self::Claude => "Edit|Write|Bash",
            Self::Codex => "Bash|apply_patch",
        }
    }

    fn guard_command(self) -> &'static str {
        match self {
            Self::Claude => "telos agent-guard --host claude",
            Self::Codex => "telos agent-guard --host codex",
        }
    }
}

/// Sorts and removes duplicate clap values before any host work.
pub fn normalize(hosts: &[AgentHost]) -> Vec<AgentHost> {
    hosts
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Parses every requested host's existing JSON before `init` writes anything.
pub fn preflight(root: &Path, hosts: &[AgentHost]) -> Result<(), TelosError> {
    for host in normalize(hosts) {
        match host {
            AgentHost::Claude => {
                let mut object = read_optional_object(&root.join(".claude/settings.json"))?;
                merge_command_hook(&mut object, host.matcher(), host.guard_command())?;
            }
            AgentHost::Codex => {
                let mut object = read_optional_object(&root.join(".codex/hooks.json"))?;
                merge_command_hook(&mut object, host.matcher(), host.guard_command())?;
            }
        }
    }
    Ok(())
}

pub fn render(root: &Path, hosts: &[AgentHost]) -> Result<(), TelosError> {
    for host in normalize(hosts) {
        match host {
            AgentHost::Claude => claude::render(root)?,
            AgentHost::Codex => codex::render(root)?,
        }
    }
    Ok(())
}

pub(crate) fn read_optional_object(path: &Path) -> Result<Map<String, Value>, TelosError> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(io_error("read", path, e)),
    };
    let value: Value = serde_json::from_str(&content).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosParseError,
            format!("{}: invalid JSON: {e}", path.display()),
        )
        .hint("repair the existing host configuration and rerun `telos init`")
    })?;
    value.as_object().cloned().ok_or_else(|| {
        TelosError::new(
            ErrorCode::TelosParseError,
            format!("{}: expected a JSON object", path.display()),
        )
        .hint("repair the existing host configuration and rerun `telos init`")
    })
}

pub(crate) fn write_json(path: &Path, object: &Map<String, Value>) -> Result<(), TelosError> {
    let parent = path.parent().expect("host config always has a parent");
    fs::create_dir_all(parent).map_err(|e| io_error("create", parent, e))?;
    let mut bytes = serde_json::to_vec_pretty(object).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to encode {}: {e}", path.display()),
        )
    })?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|e| io_error("write", path, e))
}

pub(crate) fn write_text(path: &Path, content: &str) -> Result<(), TelosError> {
    let parent = path.parent().expect("generated file always has a parent");
    fs::create_dir_all(parent).map_err(|e| io_error("create", parent, e))?;
    fs::write(path, content).map_err(|e| io_error("write", path, e))
}

/// Installs exactly one owned command hook while retaining every unrelated
/// matcher group and handler, including unknown future fields.
pub(crate) fn merge_command_hook(
    root: &mut Map<String, Value>,
    matcher: &str,
    command: &str,
) -> Result<(), TelosError> {
    let hooks = object_field(root, "hooks")?;
    let groups = array_field(hooks, "PreToolUse")?;

    groups.retain_mut(|group| {
        let Some(group_obj) = group.as_object_mut() else {
            return true;
        };
        let Some(handlers) = group_obj.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        handlers.retain(|handler| handler.get("command").and_then(Value::as_str) != Some(command));
        !handlers.is_empty()
    });

    groups.push(serde_json::json!({
        "matcher": matcher,
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": 10,
            "statusMessage": "Enforcing the Telos workflow"
        }]
    }));
    Ok(())
}

fn object_field<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, TelosError> {
    let value = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    value
        .as_object_mut()
        .ok_or_else(|| config_shape_error(key, "object"))
}

fn array_field<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Vec<Value>, TelosError> {
    let value = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    value
        .as_array_mut()
        .ok_or_else(|| config_shape_error(key, "array"))
}

fn config_shape_error(field: &str, expected: &str) -> TelosError {
    TelosError::new(
        ErrorCode::TelosParseError,
        format!("host configuration field `{field}` must be a JSON {expected}"),
    )
    .hint("repair the existing host configuration and rerun `telos init`")
}

pub(crate) fn merge_owned_block(existing: &str, start: &str, end: &str, block: &str) -> String {
    if let Some(start_at) = existing.find(start)
        && let Some(relative_end) = existing[start_at + start.len()..].find(end)
    {
        let end_at = start_at + start.len() + relative_end + end.len();
        let mut merged = String::with_capacity(existing.len() - (end_at - start_at) + block.len());
        merged.push_str(&existing[..start_at]);
        merged.push_str(block);
        merged.push_str(&existing[end_at..]);
        return merged;
    }

    let mut merged = existing.to_string();
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    if !merged.is_empty() {
        merged.push('\n');
    }
    merged.push_str(block);
    if !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged
}

pub(crate) fn read_optional_text(path: &Path) -> Result<String, TelosError> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_error("read", path, e)),
    }
}

fn io_error(verb: &str, path: &Path, e: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {}: {e}", display(path)),
    )
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
