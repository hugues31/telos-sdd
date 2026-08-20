//! Agent-host installation and the shared preventive guard.

pub mod assets;
mod claude;
mod codex;
pub mod guard;

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use telos_core::error::{ErrorCode, TelosError};

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
    path: PathBuf,
    bytes: Vec<u8>,
}

/// All requested host artifacts, including parsed and merged user content.
pub struct InstallPlan {
    root: PathBuf,
    writes: Vec<PlannedWrite>,
}

/// Parses, merges, serializes, and validates every requested host artifact
/// before `init` writes anything.
pub fn preflight(root: &Path, hosts: &[AgentHost]) -> Result<InstallPlan, TelosError> {
    let mut writes = Vec::new();
    for host in normalize(hosts) {
        match host {
            AgentHost::Claude => writes.extend(claude::plan(root)?),
            AgentHost::Codex => writes.extend(codex::plan(root)?),
        }
    }
    Ok(InstallPlan {
        root: root.to_path_buf(),
        writes,
    })
}

/// Renders only the bytes cached by [`preflight`], after Telos is sealed.
pub fn render(plan: &InstallPlan) -> Result<(), TelosError> {
    for write in &plan.writes {
        ensure_parent(
            &plan.root,
            write.path.parent().expect("planned target has a parent"),
        )?;
        validate_target(&plan.root, &write.path)?;
        fs::write(&write.path, &write.bytes)
            .map_err(|error| io_error("write", &write.path, error))?;
    }
    Ok(())
}

pub(crate) fn planned_text(
    root: &Path,
    relative: &str,
    content: String,
) -> Result<PlannedWrite, TelosError> {
    planned_bytes(root, relative, content.into_bytes())
}

pub(crate) fn planned_json(
    root: &Path,
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

fn planned_bytes(root: &Path, relative: &str, bytes: Vec<u8>) -> Result<PlannedWrite, TelosError> {
    let path = root.join(relative);
    validate_target(root, &path)?;
    Ok(PlannedWrite { path, bytes })
}

fn validate_target(root: &Path, target: &Path) -> Result<(), TelosError> {
    ensure_under_root(root, target)?;
    let parent = target.parent().expect("planned target has a parent");
    validate_existing_parents(root, parent)?;
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(target_collision(root, target)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect", target, error)),
    }
}

fn ensure_parent(root: &Path, parent: &Path) -> Result<(), TelosError> {
    ensure_under_root(root, parent)?;
    let relative = parent
        .strip_prefix(root)
        .expect("target was checked under root");
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(target_collision(root, &current)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        target_collision(root, &current)
                    } else {
                        io_error("create", &current, error)
                    }
                })?;
            }
            Err(error) => return Err(io_error("inspect", &current, error)),
        }
        let canonical_root =
            fs::canonicalize(root).map_err(|error| io_error("resolve", root, error))?;
        let canonical =
            fs::canonicalize(&current).map_err(|error| io_error("resolve", &current, error))?;
        if !canonical.starts_with(canonical_root) {
            return Err(target_collision(root, &current));
        }
    }
    Ok(())
}

fn validate_existing_parents(root: &Path, parent: &Path) -> Result<(), TelosError> {
    ensure_under_root(root, parent)?;
    let relative = parent
        .strip_prefix(root)
        .expect("target was checked under root");
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(target_collision(root, &current)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error("inspect", &current, error)),
        }
    }
    Ok(())
}

fn ensure_under_root(root: &Path, path: &Path) -> Result<(), TelosError> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(TelosError::new(
            ErrorCode::TelosInternal,
            format!(
                "generated host artifact escapes repository root: {}",
                path.display()
            ),
        ))
    }
}

fn target_collision(root: &Path, path: &Path) -> TelosError {
    let display = path.strip_prefix(root).unwrap_or(path).display();
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("`{display}` must be a real file or directory as required by Telos"),
    )
    .hint("repair the existing host configuration and rerun `telos init`")
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
