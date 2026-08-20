//! Agent-host installation and the shared preventive guard.

pub mod assets;
mod claude;
mod codex;
pub mod guard;

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use telos_core::error::{ErrorCode, TelosError};

use crate::safe_fs::SafeRoot;

pub use telos_core::config::AgentHost;

fn matcher(host: AgentHost) -> &'static str {
    match host {
        AgentHost::Claude => "Edit|Write|Bash",
        AgentHost::Codex => "Bash|apply_patch",
    }
}

fn guard_command(host: AgentHost) -> &'static str {
    match host {
        AgentHost::Claude => "telos agent-guard --host claude",
        AgentHost::Codex => "telos agent-guard --host codex",
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

/// A deterministic write whose complete bytes were built before init writes.
pub(crate) struct PlannedWrite {
    relative: PathBuf,
    bytes: Vec<u8>,
}

/// All requested host artifacts, including parsed and merged user content.
pub struct InstallPlan {
    root: SafeRoot,
    writes: Vec<PlannedWrite>,
}

/// Parses, merges, serializes, and validates every requested host artifact
/// before `init` writes anything.
pub fn preflight(root: &Path, hosts: &[AgentHost]) -> Result<InstallPlan, TelosError> {
    let root = SafeRoot::open(root)
        .map_err(|error| io_error("open", Path::new("repository root"), error))?;
    let mut writes = Vec::new();
    for host in normalize(hosts) {
        match host {
            AgentHost::Claude => writes.extend(claude::plan(&root)?),
            AgentHost::Codex => writes.extend(codex::plan(&root)?),
        }
    }
    Ok(InstallPlan { root, writes })
}

/// Renders only the bytes cached by [`preflight`], after Telos is sealed.
pub fn render(plan: &InstallPlan) -> Result<(), TelosError> {
    render_with_before_write(plan, || Ok(()))
}

fn render_with_before_write<H>(plan: &InstallPlan, hook: H) -> Result<(), TelosError>
where
    H: FnOnce() -> io::Result<()>,
{
    let mut hook = Some(hook);
    for write in &plan.writes {
        if let Some(hook) = hook.take() {
            plan.root
                .write_cached_with(&write.relative, &write.bytes, hook)
                .map_err(|error| safe_error("write", &write.relative, error))?;
        } else {
            plan.root
                .write_cached(&write.relative, &write.bytes)
                .map_err(|error| safe_error("write", &write.relative, error))?;
        }
    }
    Ok(())
}

pub(crate) fn planned_text(
    root: &SafeRoot,
    relative: &str,
    content: String,
) -> Result<PlannedWrite, TelosError> {
    planned_bytes(root, relative, content.into_bytes())
}

pub(crate) fn planned_json(
    root: &SafeRoot,
    relative: &str,
    object: &Map<String, Value>,
) -> Result<PlannedWrite, TelosError> {
    let mut bytes = serde_json::to_vec_pretty(object).map_err(|error| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to encode {relative}: {error}"),
        )
    })?;
    bytes.push(b'\n');
    planned_bytes(root, relative, bytes)
}

fn planned_bytes(
    root: &SafeRoot,
    relative: &str,
    bytes: Vec<u8>,
) -> Result<PlannedWrite, TelosError> {
    let relative = PathBuf::from(relative);
    root.validate_target(&relative)
        .map_err(|error| safe_error("inspect", &relative, error))?;
    Ok(PlannedWrite { relative, bytes })
}

fn target_collision(path: &Path) -> TelosError {
    let display = path.display();
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("`{display}` must be a real file or directory as required by Telos"),
    )
    .hint("repair the existing host configuration and rerun `telos init`")
}

pub(crate) fn read_optional_object(
    root: &SafeRoot,
    relative: &str,
) -> Result<Map<String, Value>, TelosError> {
    let path = Path::new(relative);
    let content = match root.read_optional(path) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|error| {
            io_error(
                "read",
                path,
                io::Error::new(io::ErrorKind::InvalidData, error),
            )
        })?,
        Ok(None) => return Ok(Map::new()),
        Err(error) => return Err(safe_error("read", path, error)),
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

pub(crate) fn read_optional_text(root: &SafeRoot, relative: &str) -> Result<String, TelosError> {
    let path = Path::new(relative);
    match root.read_optional(path) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|error| {
            io_error(
                "read",
                path,
                io::Error::new(io::ErrorKind::InvalidData, error),
            )
        }),
        Ok(None) => Ok(String::new()),
        Err(error) => Err(safe_error("read", path, error)),
    }
}

fn io_error(verb: &str, path: &Path, e: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {}: {e}", display(path)),
    )
}

fn safe_error(verb: &str, path: &Path, error: io::Error) -> TelosError {
    match error.kind() {
        io::ErrorKind::AlreadyExists
        | io::ErrorKind::InvalidInput
        | io::ErrorKind::NotADirectory
        | io::ErrorKind::PermissionDenied => target_collision(path),
        _ => io_error(verb, path, error),
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Barrier};

    use super::{AgentHost, ErrorCode, preflight, render_with_before_write};

    #[cfg(unix)]
    #[test]
    fn cached_agent_write_does_not_follow_a_late_parent_swap() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let plan = preflight(tmp.path(), &[AgentHost::Codex]).unwrap();
        let agents = tmp.path().join(".agents");
        let barrier = Arc::new(Barrier::new(2));
        let actor_barrier = Arc::clone(&barrier);
        let actor_agents = agents.clone();
        let actor_outside = outside.path().to_path_buf();
        let actor = std::thread::spawn(move || {
            actor_barrier.wait();
            fs::rename(&actor_agents, actor_agents.with_extension("owned"))?;
            symlink(actor_outside, actor_agents)
        });

        let error = render_with_before_write(&plan, || {
            barrier.wait();
            actor
                .join()
                .map_err(|_| std::io::Error::other("agent actor panicked"))?
        })
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::TelosChangeStateInvalid);
        assert!(!outside.path().join("skills/telos/SKILL.md").exists());
    }
}
