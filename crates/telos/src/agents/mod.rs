//! Agent-host installation and the shared preventive guard.

pub mod assets;
mod claude;
mod codex;
pub mod guard;

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use telos_core::error::{ErrorCode, TelosError};

use crate::safe_fs::{SafeRoot, StagedWrite};

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

/// A deterministic publication whose complete bytes and initial target state
/// were captured before init writes.
pub(crate) enum PlannedWrite {
    AlreadyExact {
        relative: PathBuf,
        expected: Vec<u8>,
    },
    CreateOnly {
        relative: PathBuf,
        bytes: Vec<u8>,
    },
    MergeExisting {
        relative: PathBuf,
        expected: Vec<u8>,
        bytes: Vec<u8>,
    },
}

pub(crate) enum InitialState {
    Absent,
    Existing(Vec<u8>),
}

pub(crate) struct ReadValue<T> {
    pub(crate) value: T,
    initial: InitialState,
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

#[cfg(test)]
pub(crate) fn render_with_before_publish<H>(
    plan: &InstallPlan,
    before_publish: H,
) -> Result<(), TelosError>
where
    H: FnMut(&Path) -> io::Result<()>,
{
    render_with_hooks(
        plan,
        |_relative, file, bytes| file.write_all(bytes),
        || Ok(()),
        before_publish,
    )
}

fn render_with_before_write<H>(plan: &InstallPlan, hook: H) -> Result<(), TelosError>
where
    H: FnOnce() -> io::Result<()>,
{
    render_with_hooks(
        plan,
        |_relative, file, bytes| file.write_all(bytes),
        hook,
        |_relative| Ok(()),
    )
}

fn render_with_hooks<S, V, P>(
    plan: &InstallPlan,
    mut stage_write: S,
    before_validation: V,
    mut before_publish: P,
) -> Result<(), TelosError>
where
    S: FnMut(&Path, &mut cap_std::fs::File, &[u8]) -> io::Result<()>,
    V: FnOnce() -> io::Result<()>,
    P: FnMut(&Path) -> io::Result<()>,
{
    let mut staged = Vec::with_capacity(plan.writes.len());
    for (index, write) in plan.writes.iter().enumerate() {
        let Some(bytes) = write.bytes() else {
            continue;
        };
        let relative = write.relative();
        let staged_write = plan
            .root
            .stage_with(relative, bytes, |file, bytes| {
                stage_write(relative, file, bytes)
            })
            .map_err(|error| safe_error("stage", relative, error))?;
        staged.push((index, staged_write));
    }

    before_validation().map_err(|error| {
        io_error(
            "prepare publication for",
            Path::new("agent host artifacts"),
            error,
        )
    })?;

    // This global compare happens before the first final name is published.
    // It closes deterministic preflight-to-render changes without exposing a
    // partially staged file at any owned target.
    validate_exact_noops(&plan.root, &plan.writes)?;
    for (index, staging) in &staged {
        let write = &plan.writes[*index];
        validate_expected(&plan.root, write, staging)?;
    }

    for (index, staging) in staged {
        let write = &plan.writes[index];
        let relative = write.relative();
        before_publish(relative)
            .map_err(|error| io_error("prepare publication for", relative, error))?;
        // Recheck merges next to their replacement too. There is no portable
        // filesystem-wide multi-file CAS; the seal's non-adversarial model
        // therefore permits only the syscall-sized check/rename gap here.
        validate_expected(&plan.root, write, &staging)?;
        match write {
            PlannedWrite::AlreadyExact { .. } => unreachable!("no-op writes are not staged"),
            PlannedWrite::CreateOnly { .. } => staging.publish_create_only(),
            PlannedWrite::MergeExisting { .. } => staging.publish_replace(),
        }
        .map_err(|error| safe_error("publish", relative, error))?;
    }
    validate_exact_noops(&plan.root, &plan.writes)?;
    Ok(())
}

fn validate_exact_noops(root: &SafeRoot, writes: &[PlannedWrite]) -> Result<(), TelosError> {
    for write in writes {
        let PlannedWrite::AlreadyExact { relative, expected } = write else {
            continue;
        };
        let actual = root
            .read_optional(relative)
            .map_err(|error| safe_error("inspect", relative, error))?;
        if actual.as_deref() != Some(expected) {
            return Err(target_collision(relative));
        }
    }
    Ok(())
}

fn validate_expected(
    root: &SafeRoot,
    write: &PlannedWrite,
    staging: &StagedWrite,
) -> Result<(), TelosError> {
    let relative = write.relative();
    staging
        .validate_parent_path(root)
        .map_err(|error| safe_error("inspect", relative, error))?;
    let actual = staging
        .read_target()
        .map_err(|error| safe_error("inspect", relative, error))?;
    let matches = match write {
        PlannedWrite::AlreadyExact { .. } => true,
        PlannedWrite::CreateOnly { .. } => actual.is_none(),
        PlannedWrite::MergeExisting { expected, .. } => actual.as_deref() == Some(expected),
    };
    if matches {
        Ok(())
    } else {
        Err(target_collision(relative))
    }
}

impl PlannedWrite {
    fn relative(&self) -> &Path {
        match self {
            Self::AlreadyExact { relative, .. }
            | Self::CreateOnly { relative, .. }
            | Self::MergeExisting { relative, .. } => relative,
        }
    }

    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::AlreadyExact { .. } => None,
            Self::CreateOnly { bytes, .. } | Self::MergeExisting { bytes, .. } => Some(bytes),
        }
    }
}

pub(crate) fn planned_text(
    root: &SafeRoot,
    relative: &str,
    content: String,
) -> Result<PlannedWrite, TelosError> {
    planned_bytes(root, relative, content.into_bytes())
}

pub(crate) fn planned_json(
    relative: &str,
    object: &Map<String, Value>,
    initial: InitialState,
) -> Result<PlannedWrite, TelosError> {
    let mut bytes = serde_json::to_vec_pretty(object).map_err(|error| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("failed to encode {relative}: {error}"),
        )
    })?;
    bytes.push(b'\n');
    Ok(planned_from_initial(relative, bytes, initial))
}

fn planned_bytes(
    root: &SafeRoot,
    relative: &str,
    bytes: Vec<u8>,
) -> Result<PlannedWrite, TelosError> {
    let relative = PathBuf::from(relative);
    let initial = initial_state(root, &relative)?;
    match initial {
        InitialState::Absent => Ok(PlannedWrite::CreateOnly { relative, bytes }),
        InitialState::Existing(expected) if expected == bytes => {
            Ok(PlannedWrite::AlreadyExact { relative, expected })
        }
        InitialState::Existing(_) => Err(target_collision(&relative)),
    }
}

pub(crate) fn planned_merged_text(
    relative: &str,
    content: String,
    initial: InitialState,
) -> PlannedWrite {
    planned_from_initial(relative, content.into_bytes(), initial)
}

fn planned_from_initial(relative: &str, bytes: Vec<u8>, initial: InitialState) -> PlannedWrite {
    planned_from_initial_path(PathBuf::from(relative), bytes, initial)
}

fn planned_from_initial_path(
    relative: PathBuf,
    bytes: Vec<u8>,
    initial: InitialState,
) -> PlannedWrite {
    match initial {
        InitialState::Absent => PlannedWrite::CreateOnly { relative, bytes },
        InitialState::Existing(expected) if expected == bytes => {
            PlannedWrite::AlreadyExact { relative, expected }
        }
        InitialState::Existing(expected) => PlannedWrite::MergeExisting {
            relative,
            expected,
            bytes,
        },
    }
}

fn initial_state(root: &SafeRoot, relative: &Path) -> Result<InitialState, TelosError> {
    match root.read_optional(relative) {
        Ok(Some(bytes)) => Ok(InitialState::Existing(bytes)),
        Ok(None) => Ok(InitialState::Absent),
        Err(error) => Err(safe_error("inspect", relative, error)),
    }
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
) -> Result<ReadValue<Map<String, Value>>, TelosError> {
    let path = Path::new(relative);
    let (content, initial) = match root.read_optional(path) {
        Ok(Some(bytes)) => {
            let content = String::from_utf8(bytes.clone()).map_err(|error| {
                io_error(
                    "read",
                    path,
                    io::Error::new(io::ErrorKind::InvalidData, error),
                )
            })?;
            (content, InitialState::Existing(bytes))
        }
        Ok(None) => {
            return Ok(ReadValue {
                value: Map::new(),
                initial: InitialState::Absent,
            });
        }
        Err(error) => return Err(safe_error("read", path, error)),
    };
    let value: Value = serde_json::from_str(&content).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosParseError,
            format!("{}: invalid JSON: {e}", path.display()),
        )
        .hint("repair the existing host configuration and rerun `telos init`")
    })?;
    let value = value.as_object().cloned().ok_or_else(|| {
        TelosError::new(
            ErrorCode::TelosParseError,
            format!("{}: expected a JSON object", path.display()),
        )
        .hint("repair the existing host configuration and rerun `telos init`")
    })?;
    Ok(ReadValue { value, initial })
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

pub(crate) fn read_optional_text(
    root: &SafeRoot,
    relative: &str,
) -> Result<ReadValue<String>, TelosError> {
    let path = Path::new(relative);
    match root.read_optional(path) {
        Ok(Some(bytes)) => {
            let value = String::from_utf8(bytes.clone()).map_err(|error| {
                io_error(
                    "read",
                    path,
                    io::Error::new(io::ErrorKind::InvalidData, error),
                )
            })?;
            Ok(ReadValue {
                value,
                initial: InitialState::Existing(bytes),
            })
        }
        Ok(None) => Ok(ReadValue {
            value: String::new(),
            initial: InitialState::Absent,
        }),
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
    use std::io::{self, Write};
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    use super::{AgentHost, ErrorCode, preflight, render_with_before_write, render_with_hooks};

    fn assert_no_staging_files(root: &Path) {
        fn walk(directory: &Path) {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).unwrap();
                assert!(
                    !entry
                        .file_name()
                        .to_string_lossy()
                        .contains(".telos-staging-"),
                    "leaked staging entry {}",
                    path.display()
                );
                if metadata.is_dir() {
                    walk(&path);
                }
            }
        }

        walk(root);
    }

    #[test]
    fn late_regular_owners_of_absent_agent_targets_are_never_overwritten() {
        for (host, target) in [
            (AgentHost::Claude, ".claude/skills/telos/SKILL.md"),
            (AgentHost::Claude, ".claude/settings.json"),
            (AgentHost::Codex, ".agents/skills/telos/SKILL.md"),
            (AgentHost::Codex, "AGENTS.md"),
            (AgentHost::Codex, ".codex/hooks.json"),
            (AgentHost::Codex, ".codex/rules/telos.rules"),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let plan = preflight(tmp.path(), &[host]).unwrap();
            let target_path = tmp.path().join(target);

            let error = render_with_hooks(
                &plan,
                |_relative, file, bytes| file.write_all(bytes),
                || Ok(()),
                |relative| {
                    if relative == Path::new(target) {
                        fs::create_dir_all(target_path.parent().unwrap())?;
                        fs::write(&target_path, b"late owner\n")?;
                    }
                    Ok(())
                },
            )
            .unwrap_err();

            assert_eq!(error.code, ErrorCode::TelosChangeStateInvalid, "{target}");
            assert_eq!(fs::read(&target_path).unwrap(), b"late owner\n", "{target}");
            assert_no_staging_files(tmp.path());
        }
    }

    #[test]
    fn existing_merge_targets_changed_after_preflight_abort_before_publication() {
        for (host, target, initial, changed) in [
            (
                AgentHost::Claude,
                ".claude/settings.json",
                b"{\"env\":{\"OWNER\":\"initial\"}}\n".as_slice(),
                b"{\"env\":{\"OWNER\":\"changed\"}}\n".as_slice(),
            ),
            (
                AgentHost::Codex,
                "AGENTS.md",
                b"# initial owner\n".as_slice(),
                b"# changed owner\n".as_slice(),
            ),
            (
                AgentHost::Codex,
                ".codex/hooks.json",
                b"{\"description\":\"initial owner\"}\n".as_slice(),
                b"{\"description\":\"changed owner\"}\n".as_slice(),
            ),
            (
                AgentHost::Codex,
                ".codex/rules/telos.rules",
                b"# initial owner\n".as_slice(),
                b"# changed owner\n".as_slice(),
            ),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let target_path = tmp.path().join(target);
            fs::create_dir_all(target_path.parent().unwrap()).unwrap();
            fs::write(&target_path, initial).unwrap();
            let plan = preflight(tmp.path(), &[host]).unwrap();

            let error = render_with_hooks(
                &plan,
                |_relative, file, bytes| file.write_all(bytes),
                || fs::write(&target_path, changed),
                |_relative| Ok(()),
            )
            .unwrap_err();

            assert_eq!(error.code, ErrorCode::TelosChangeStateInvalid, "{target}");
            assert_eq!(fs::read(&target_path).unwrap(), changed, "{target}");
            for generated in [
                ".claude/skills/telos/SKILL.md",
                ".agents/skills/telos/SKILL.md",
                ".codex/hooks.json",
            ] {
                if generated != target {
                    assert!(
                        !tmp.path().join(generated).is_file(),
                        "{target}: {generated}"
                    );
                }
            }
            assert_no_staging_files(tmp.path());
        }
    }

    #[test]
    fn partial_staging_write_publishes_no_agent_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), b"original owner\n").unwrap();
        let plan = preflight(tmp.path(), &[AgentHost::Codex]).unwrap();

        let error = render_with_hooks(
            &plan,
            |relative, file, bytes| {
                if relative == Path::new("AGENTS.md") {
                    file.write_all(&bytes[..bytes.len() / 2])?;
                    return Err(io::Error::other("forced partial staging write"));
                }
                file.write_all(bytes)
            },
            || Ok(()),
            |_relative| Ok(()),
        )
        .unwrap_err();

        assert!(error.message.contains("forced partial staging write"));
        for target in [
            ".agents/skills/telos/SKILL.md",
            ".agents/skills/telos-challenger/SKILL.md",
            ".agents/skills/telos-implementer/SKILL.md",
            ".codex/hooks.json",
            ".codex/rules/telos.rules",
        ] {
            assert!(!tmp.path().join(target).is_file(), "published {target}");
        }
        assert_eq!(
            fs::read(tmp.path().join("AGENTS.md")).unwrap(),
            b"original owner\n"
        );
        assert_no_staging_files(tmp.path());
    }

    #[test]
    fn exact_agent_artifacts_are_noops_when_replanned() {
        let tmp = tempfile::tempdir().unwrap();
        let first = preflight(tmp.path(), &[AgentHost::Codex]).unwrap();
        super::render(&first).unwrap();
        let retry = preflight(tmp.path(), &[AgentHost::Codex]).unwrap();
        let mut staged = 0;
        let mut published = 0;

        render_with_hooks(
            &retry,
            |_relative, file, bytes| {
                staged += 1;
                file.write_all(bytes)
            },
            || Ok(()),
            |_relative| {
                published += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(staged, 0);
        assert_eq!(published, 0);
    }

    #[test]
    fn an_exact_noop_changed_after_replanning_aborts_the_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let first = preflight(tmp.path(), &[AgentHost::Codex]).unwrap();
        super::render(&first).unwrap();
        let retry = preflight(tmp.path(), &[AgentHost::Codex]).unwrap();
        let agents = tmp.path().join("AGENTS.md");

        let error = render_with_hooks(
            &retry,
            |_relative, file, bytes| file.write_all(bytes),
            || fs::write(&agents, b"late owner change\n"),
            |_relative| Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::TelosChangeStateInvalid);
        assert_eq!(fs::read(agents).unwrap(), b"late owner change\n");
    }

    #[test]
    fn a_non_telos_owner_at_a_generated_skill_path_is_not_replanned_as_a_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let skill = tmp.path().join(".agents/skills/telos/SKILL.md");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, b"owner bytes\n").unwrap();

        let error = match preflight(tmp.path(), &[AgentHost::Codex]) {
            Ok(_) => panic!("non-Telos skill owner was accepted"),
            Err(error) => error,
        };

        assert_eq!(error.code, ErrorCode::TelosChangeStateInvalid);
        assert_eq!(fs::read(skill).unwrap(), b"owner bytes\n");
    }

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

    #[test]
    fn staged_agent_write_rejects_a_real_directory_parent_replacement() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let plan = preflight(tmp.path(), &[AgentHost::Codex]).unwrap();
        let agents = tmp.path().join(".agents");
        let moved_agents = outside.path().join("held-agents");

        let error = render_with_before_write(&plan, || {
            fs::rename(&agents, &moved_agents)?;
            for parent in [
                ".agents/skills/telos",
                ".agents/skills/telos-challenger",
                ".agents/skills/telos-implementer",
                ".codex/rules",
            ] {
                fs::create_dir_all(tmp.path().join(parent))?;
            }
            Ok(())
        })
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::TelosChangeStateInvalid);
        assert!(!agents.join("skills/telos/SKILL.md").exists());
        assert!(!moved_agents.join("skills/telos/SKILL.md").exists());
        assert_no_staging_files(tmp.path());
        assert_no_staging_files(outside.path());
    }
}
