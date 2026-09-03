//! `telos init`: turn a git repository into a telos project.
//!
//! Everything is created at the *git* root, not at the current directory: a
//! lock seals repo-relative paths (`telos/contexts/billing/notions/Invoice.tel`), and those
//! paths are only meaningful -- and only hashable by `git hash-object` --
//! relative to the worktree root. A `telos/` nested somewhere below it would
//! seal paths git cannot resolve.
//!
//! The order below matters: `.gitattributes` is written *before* sealing, so
//! that the very first seal already runs the `eol=lf` clean filter and the
//! OIDs it records are the cross-OS ones.

use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use telos_core::config::{
    AgentHost as ConfigAgentHost, AgentsCfg, Config, Globs, Policy, TddPolicy, TestCfg,
};
use telos_core::emit::emit_config;
use telos_core::error::{Diagnostic, ErrorCode, TelosError};
use telos_core::git::GitRepo;
use telos_core::lock::{Lock, seal};
use telos_core::reconcile::require_sealable_structure;
use telos_core::workspace::Workspace;

use crate::ci::{self, CiProvider};
use crate::commands::Ctx;
use crate::commands::agents::{self, AgentHost};
use crate::envelope::{CmdResult, Outcome};
use crate::safe_fs::SafeRoot;

/// Pins the byte identity of the spec across operating systems: whatever a
/// checkout does to line endings, the blob git hashes for a `telos/` file is
/// the LF one, so a lock sealed on Linux verifies on Windows.
const GITATTRIBUTES_LINE: &str = "telos/** text eol=lf";

/// The spec subdirectories a project always has, created empty.
const SUBDIRS: [&str; 3] = ["telos/contexts", "telos/constraints", "telos/changes"];

const INIT_MARKER_PATH: &str = ".telos-init.json";
const CONFIG_PATH: &str = "telos/telos.toml";
const CONTEXT_MAP_PATH: &str = "telos/context-map.tel";
const COUNTERS_PATH: &str = "telos/changes/counters.toml";
const LOCK_PATH: &str = "telos/telos.lock";
const GITATTRIBUTES_PATH: &str = ".gitattributes";
const COUNTERS_BYTES: &[u8] = b"intent = 0\nscenario = 0\nconstraint = 0\nchange = 0\n";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct InitMarker {
    format: String,
    agents: Vec<String>,
    ci: Option<String>,
    core: CorePlan,
    phase: InitPhase,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum InitPhase {
    // Boundary phases are persisted before their corresponding publications.
    // A crash after either CAS therefore resumes the advanced phase instead
    // of trusting bytes cached from the preceding preflight.
    Preparing,
    CoreWriting,
    Sealed { lock: Vec<u8> },
    Integrating { lock: Vec<u8> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CorePlan {
    config: CoreFile,
    context_map: CoreFile,
    counters: CoreFile,
    gitattributes: CoreFile,
    initial_lock: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CoreFile {
    initial: Option<Vec<u8>>,
    desired: Vec<u8>,
}

pub fn run(ctx: &Ctx, hosts: &[AgentHost], ci: Option<CiProvider>) -> CmdResult {
    run_with_agent_renderer(ctx, hosts, ci, agents::render)
}

fn run_with_agent_renderer<F>(
    ctx: &Ctx,
    hosts: &[AgentHost],
    ci: Option<CiProvider>,
    render_agents: F,
) -> CmdResult
where
    F: FnOnce(&agents::InstallPlan) -> Result<(), TelosError>,
{
    run_with_hooks(ctx, hosts, ci, |_| Ok(()), render_agents)
}

fn run_with_hooks<F, H>(
    ctx: &Ctx,
    hosts: &[AgentHost],
    ci: Option<CiProvider>,
    after_core_publish: H,
    render_agents: F,
) -> CmdResult
where
    F: FnOnce(&agents::InstallPlan) -> Result<(), TelosError>,
    H: FnMut(&Path) -> std::io::Result<()>,
{
    run_with_boundary_hooks(
        ctx,
        hosts,
        ci,
        || Ok(()),
        after_core_publish,
        || Ok(()),
        render_agents,
    )
}

fn run_with_boundary_hooks<F, B, H, I>(
    ctx: &Ctx,
    hosts: &[AgentHost],
    ci: Option<CiProvider>,
    before_core_boundary: B,
    mut after_core_publish: H,
    before_integration_boundary: I,
    render_agents: F,
) -> CmdResult
where
    F: FnOnce(&agents::InstallPlan) -> Result<(), TelosError>,
    B: FnOnce() -> std::io::Result<()>,
    H: FnMut(&Path) -> std::io::Result<()>,
    I: FnOnce() -> std::io::Result<()>,
{
    let git = GitRepo::discover(&ctx.cwd)?;
    let root = git.root().to_path_buf();
    let config_path = root.join(CONFIG_PATH);
    let config_bytes = initial_config_bytes(hosts)?;
    let (requested_agents, requested_ci) = requested_options(hosts, ci);
    let safe_root = SafeRoot::open(&root)
        .map_err(|error| io_error("open", Path::new("repository root"), error))?;
    let marker_on_disk = safe_root
        .read_optional(Path::new(INIT_MARKER_PATH))
        .map_err(|error| marker_error("inspect", error))?;
    let config_exists = safe_root
        .exists_no_follow(Path::new(CONFIG_PATH))
        .map_err(|error| io_error("inspect", &config_path, error))?;

    let (marker, marker_bytes, resuming) = match marker_on_disk {
        Some(bytes) => {
            let parsed = serde_json::from_slice::<InitMarker>(&bytes).ok();
            let Some(marker) = parsed.filter(|marker| {
                marker.format == "telos-init-v3"
                    && marker.agents == requested_agents
                    && marker.ci == requested_ci
                    && marker.core.definition_matches(&config_bytes)
            }) else {
                if config_exists {
                    return Err(already_initialized(&config_path));
                }
                return Err(marker_collision());
            };
            (marker, bytes, true)
        }
        None => {
            if config_exists {
                return Err(already_initialized(&config_path));
            }
            validate_telos_tree(&safe_root, false)?;
            let core = CorePlan::fresh(&safe_root, config_bytes)?;
            let marker = InitMarker {
                format: "telos-init-v3".to_string(),
                agents: requested_agents,
                ci: requested_ci,
                core,
                phase: InitPhase::Preparing,
            };
            let bytes = marker_bytes(&marker);
            (marker, bytes, false)
        }
    };

    if resuming {
        validate_resume_core(&safe_root, &root, &git, &marker)?;
    }

    // Host JSON is the only user-owned input init has to merge. Parse every
    // requested file before the first project write so malformed config can
    // never leave a partial Telos tree behind.
    let agent_plan = agents::preflight(&root, hosts)?;
    let ci_plan = if resuming {
        ci::preflight_resume(&root, ci)?
    } else {
        ci::preflight(&root, ci)?
    };

    if !resuming {
        safe_root
            .create_new_write_with(
                Path::new(INIT_MARKER_PATH),
                &marker_bytes,
                |file, bytes| file.write_all(bytes),
                || Ok(()),
            )
            .map_err(|error| marker_error("create", error))?;
    }

    let (core_marker, core_marker_bytes) = match marker.phase {
        InitPhase::Preparing | InitPhase::CoreWriting => {
            enter_core_boundary(&safe_root, &marker, &marker_bytes, before_core_boundary)?
        }
        InitPhase::Sealed { .. } | InitPhase::Integrating { .. } => (marker, marker_bytes),
    };
    let sealed_marker_bytes = match &core_marker.phase {
        InitPhase::CoreWriting => finish_core(
            &safe_root,
            &root,
            &git,
            &core_marker,
            &core_marker_bytes,
            &mut after_core_publish,
        )?,
        InitPhase::Sealed { .. } | InitPhase::Integrating { .. } => core_marker_bytes,
        InitPhase::Preparing => unreachable!("the core boundary advances the marker phase"),
    };

    let sealed_marker: InitMarker = serde_json::from_slice(&sealed_marker_bytes)
        .expect("Telos serialized this marker immediately above");
    validate_resume_core(&safe_root, &root, &git, &sealed_marker)?;
    let (integrating_marker, integrating_marker_bytes) = enter_integration_boundary(
        &safe_root,
        &sealed_marker,
        &sealed_marker_bytes,
        before_integration_boundary,
    )?;
    validate_marker_exact(&safe_root, &integrating_marker_bytes)?;
    render_agents(&agent_plan)?;
    validate_marker_exact(&safe_root, &integrating_marker_bytes)?;
    ci::render(&ci_plan)?;
    validate_resume_core(&safe_root, &root, &git, &integrating_marker)?;
    validate_marker_exact(&safe_root, &integrating_marker_bytes)?;
    safe_root
        .remove_file_if_matches(Path::new(INIT_MARKER_PATH), &integrating_marker_bytes)
        .map_err(|error| marker_error("remove", error))?;

    Ok(Outcome {
        result: json!({ "root": "telos", "sealed": true }),
        human: "initialized telos/ and sealed telos/telos.lock".to_string(),
        next_actions: vec!["telos status".to_string()],
    })
}

impl CorePlan {
    fn fresh(safe_root: &SafeRoot, config: Vec<u8>) -> Result<Self, TelosError> {
        let initial_gitattributes = read_core(safe_root, GITATTRIBUTES_PATH)?;
        let desired_gitattributes = gitattributes_bytes(initial_gitattributes.as_deref())?;

        Ok(Self {
            config: CoreFile {
                initial: None,
                desired: config,
            },
            context_map: CoreFile {
                initial: None,
                desired: b"context-map {\n}\n".to_vec(),
            },
            counters: CoreFile {
                initial: None,
                desired: COUNTERS_BYTES.to_vec(),
            },
            gitattributes: CoreFile {
                initial: initial_gitattributes,
                desired: desired_gitattributes,
            },
            initial_lock: None,
        })
    }

    fn definition_matches(&self, config: &[u8]) -> bool {
        self.config.initial.is_none()
            && self.config.desired == config
            && self.context_map.desired == b"context-map {\n}\n"
            && self.counters.desired == COUNTERS_BYTES
            && gitattributes_bytes(self.gitattributes.initial.as_deref())
                .is_ok_and(|expected| expected == self.gitattributes.desired)
    }
}

fn enter_core_boundary<B>(
    safe_root: &SafeRoot,
    marker: &InitMarker,
    marker_on_disk: &[u8],
    before_boundary: B,
) -> Result<(InitMarker, Vec<u8>), TelosError>
where
    B: FnOnce() -> std::io::Result<()>,
{
    before_boundary()
        .map_err(|error| io_error("enter core publication", Path::new(INIT_MARKER_PATH), error))?;
    match marker.phase {
        InitPhase::Preparing => {
            let mut writing = marker.clone();
            writing.phase = InitPhase::CoreWriting;
            let writing_bytes = marker_bytes(&writing);
            replace_exact_file(safe_root, INIT_MARKER_PATH, marker_on_disk, &writing_bytes)?;
            Ok((writing, writing_bytes))
        }
        InitPhase::CoreWriting => {
            validate_marker_exact(safe_root, marker_on_disk)?;
            Ok((marker.clone(), marker_on_disk.to_vec()))
        }
        InitPhase::Sealed { .. } | InitPhase::Integrating { .. } => {
            unreachable!("a sealed marker does not enter core publication")
        }
    }
}

/// Advances the marker before any integration final name can be published.
/// `replace_exact_file` is deliberately strict: finding the desired next
/// phase already present is still a stale-marker error, not an idempotent
/// no-op, because this invocation did not perform that transition.
fn enter_integration_boundary<I>(
    safe_root: &SafeRoot,
    marker: &InitMarker,
    marker_on_disk: &[u8],
    before_boundary: I,
) -> Result<(InitMarker, Vec<u8>), TelosError>
where
    I: FnOnce() -> std::io::Result<()>,
{
    before_boundary().map_err(|error| {
        io_error(
            "enter integration publication",
            Path::new(INIT_MARKER_PATH),
            error,
        )
    })?;
    match &marker.phase {
        InitPhase::Sealed { lock } => {
            let mut integrating = marker.clone();
            integrating.phase = InitPhase::Integrating { lock: lock.clone() };
            let integrating_bytes = marker_bytes(&integrating);
            replace_exact_file(
                safe_root,
                INIT_MARKER_PATH,
                marker_on_disk,
                &integrating_bytes,
            )?;
            Ok((integrating, integrating_bytes))
        }
        InitPhase::Integrating { .. } => {
            validate_marker_exact(safe_root, marker_on_disk)?;
            Ok((marker.clone(), marker_on_disk.to_vec()))
        }
        InitPhase::Preparing | InitPhase::CoreWriting => {
            unreachable!("an unsealed marker does not enter integration publication")
        }
    }
}

fn finish_core<H>(
    safe_root: &SafeRoot,
    root: &Path,
    git: &GitRepo,
    marker: &InitMarker,
    preparing_marker_bytes: &[u8],
    after_core_publish: &mut H,
) -> Result<Vec<u8>, TelosError>
where
    H: FnMut(&Path) -> std::io::Result<()>,
{
    validate_preparing_core(safe_root, root, git, &marker.core)?;
    validate_marker_exact(safe_root, preparing_marker_bytes)?;
    create_required_directories(safe_root)?;

    for (relative, plan) in core_files(&marker.core) {
        if publish_core_file(safe_root, relative, plan)? {
            after_core_publish(Path::new(relative))
                .map_err(|error| io_error("publish", Path::new(relative), error))?;
        }
    }
    validate_directory_shapes(safe_root, true)?;
    validate_deterministic_core_exact(safe_root, &marker.core)?;

    let lock_bytes = compute_lock_bytes(root, git)?;
    let lock_plan = CoreFile {
        initial: marker.core.initial_lock.clone(),
        desired: lock_bytes.clone(),
    };
    if publish_core_file(safe_root, LOCK_PATH, &lock_plan)? {
        after_core_publish(Path::new(LOCK_PATH))
            .map_err(|error| io_error("publish", Path::new(LOCK_PATH), error))?;
    }
    validate_core_file_exact(safe_root, LOCK_PATH, &lock_bytes)?;

    let mut sealed = marker.clone();
    sealed.phase = InitPhase::Sealed { lock: lock_bytes };
    let sealed_bytes = marker_bytes(&sealed);
    replace_exact_file(
        safe_root,
        INIT_MARKER_PATH,
        preparing_marker_bytes,
        &sealed_bytes,
    )?;
    Ok(sealed_bytes)
}

fn validate_resume_core(
    safe_root: &SafeRoot,
    root: &Path,
    git: &GitRepo,
    marker: &InitMarker,
) -> Result<(), TelosError> {
    match &marker.phase {
        InitPhase::Preparing | InitPhase::CoreWriting => {
            validate_preparing_core(safe_root, root, git, &marker.core)
        }
        InitPhase::Sealed { lock } | InitPhase::Integrating { lock } => {
            validate_telos_tree(safe_root, true)?;
            validate_directory_shapes(safe_root, true)?;
            validate_deterministic_core_exact(safe_root, &marker.core)?;
            validate_core_file_exact(safe_root, LOCK_PATH, lock)?;
            if compute_lock_bytes(root, git)? != *lock {
                return Err(core_changed(Path::new(LOCK_PATH)));
            }
            Ok(())
        }
    }
}

fn validate_preparing_core(
    safe_root: &SafeRoot,
    root: &Path,
    git: &GitRepo,
    core: &CorePlan,
) -> Result<(), TelosError> {
    validate_telos_tree(safe_root, true)?;
    let mut deterministic_complete = true;
    for (relative, plan) in core_files(core) {
        let current = read_core(safe_root, relative)?;
        if current.as_deref() == Some(plan.desired.as_slice()) {
            continue;
        }
        deterministic_complete = false;
        if current != plan.initial {
            return Err(core_changed(Path::new(relative)));
        }
    }

    let current_lock = read_core(safe_root, LOCK_PATH)?;
    if deterministic_complete {
        let expected_lock = compute_lock_bytes(root, git)?;
        if current_lock != core.initial_lock && current_lock.as_deref() != Some(&expected_lock) {
            return Err(core_changed(Path::new(LOCK_PATH)));
        }
    } else if current_lock != core.initial_lock {
        return Err(core_changed(Path::new(LOCK_PATH)));
    }
    Ok(())
}

fn validate_deterministic_core_exact(
    safe_root: &SafeRoot,
    core: &CorePlan,
) -> Result<(), TelosError> {
    for (relative, plan) in core_files(core) {
        validate_core_file_exact(safe_root, relative, &plan.desired)?;
    }
    Ok(())
}

fn core_files(core: &CorePlan) -> [(&'static str, &CoreFile); 4] {
    [
        (CONFIG_PATH, &core.config),
        (CONTEXT_MAP_PATH, &core.context_map),
        (GITATTRIBUTES_PATH, &core.gitattributes),
        (COUNTERS_PATH, &core.counters),
    ]
}

fn publish_core_file(
    safe_root: &SafeRoot,
    relative: &str,
    plan: &CoreFile,
) -> Result<bool, TelosError> {
    let current = read_core(safe_root, relative)?;
    if current.as_deref() == Some(plan.desired.as_slice()) {
        return Ok(false);
    }
    if current != plan.initial {
        return Err(core_changed(Path::new(relative)));
    }

    let staged = safe_root
        .stage_with(Path::new(relative), &plan.desired, |file, bytes| {
            file.write_all(bytes)
        })
        .map_err(|error| core_write_error("stage", relative, error))?;
    if staged
        .read_target()
        .map_err(|_| core_changed(Path::new(relative)))?
        != plan.initial
    {
        return Err(core_changed(Path::new(relative)));
    }
    staged
        .validate_parent_path(safe_root)
        .map_err(|_| core_changed(Path::new(relative)))?;
    match plan.initial {
        Some(_) => staged.publish_replace(),
        None => staged.publish_create_only(),
    }
    .map_err(|error| core_write_error("publish", relative, error))?;
    Ok(true)
}

fn replace_exact_file(
    safe_root: &SafeRoot,
    relative: &str,
    expected: &[u8],
    desired: &[u8],
) -> Result<(), TelosError> {
    if read_core(safe_root, relative)?.as_deref() != Some(expected) {
        return Err(core_changed(Path::new(relative)));
    }
    let staged = safe_root
        .stage_with(Path::new(relative), desired, |file, bytes| {
            file.write_all(bytes)
        })
        .map_err(|error| core_write_error("stage", relative, error))?;
    if staged
        .read_target()
        .map_err(|_| core_changed(Path::new(relative)))?
        .as_deref()
        != Some(expected)
    {
        return Err(core_changed(Path::new(relative)));
    }
    staged
        .validate_parent_path(safe_root)
        .map_err(|_| core_changed(Path::new(relative)))?;
    staged
        .publish_replace()
        .map_err(|error| core_write_error("publish", relative, error))?;
    Ok(())
}

fn validate_marker_exact(safe_root: &SafeRoot, expected: &[u8]) -> Result<(), TelosError> {
    match safe_root.read_optional(Path::new(INIT_MARKER_PATH)) {
        Ok(Some(actual)) if actual == expected => Ok(()),
        Ok(_) | Err(_) => Err(marker_collision()),
    }
}

fn validate_core_file_exact(
    safe_root: &SafeRoot,
    relative: &str,
    expected: &[u8],
) -> Result<(), TelosError> {
    if read_core(safe_root, relative)?.as_deref() == Some(expected) {
        Ok(())
    } else {
        Err(core_changed(Path::new(relative)))
    }
}

/// A fresh init may reuse only empty canonical Telos directories. Every
/// byte-bearing entry is somebody else's owner until a matching init marker
/// proves otherwise. During resume, only the exact transaction-owned core
/// names are additionally admitted; their bytes are checked separately.
fn validate_telos_tree(safe_root: &SafeRoot, resuming: bool) -> Result<(), TelosError> {
    let invalid = |path: &Path| {
        if resuming {
            core_changed(path)
        } else {
            foreign_telos_owner(path)
        }
    };
    let Some(top) = safe_root
        .directory_entries(Path::new("telos"))
        .map_err(|_| invalid(Path::new("telos")))?
    else {
        return Ok(());
    };

    for (name, is_directory) in top {
        let path = PathBuf::from("telos").join(&name);
        let Some(name) = name.to_str() else {
            return Err(invalid(&path));
        };
        match name {
            "contexts" | "constraints" | "changes" if is_directory => {}
            "telos.toml" | "context-map.tel" | "telos.lock" if resuming && !is_directory => {}
            _ => return Err(invalid(&path)),
        }
    }

    for relative in ["telos/contexts", "telos/constraints"] {
        if let Some(entries) = safe_root
            .directory_entries(Path::new(relative))
            .map_err(|_| invalid(Path::new(relative)))?
            && !entries.is_empty()
        {
            return Err(invalid(Path::new(relative)));
        }
    }
    if let Some(entries) = safe_root
        .directory_entries(Path::new("telos/changes"))
        .map_err(|_| invalid(Path::new("telos/changes")))?
    {
        for (name, is_directory) in entries {
            if !(resuming && name == "counters.toml" && !is_directory) {
                return Err(invalid(&PathBuf::from("telos/changes").join(name)));
            }
        }
    }
    Ok(())
}

fn validate_directory_shapes(
    safe_root: &SafeRoot,
    require_present: bool,
) -> Result<(), TelosError> {
    for relative in std::iter::once("telos").chain(SUBDIRS.iter().copied()) {
        let exists = safe_root
            .exists_no_follow(Path::new(relative))
            .map_err(|_| core_changed(Path::new(relative)))?;
        let valid = safe_root
            .validate_directory(Path::new(relative))
            .map_err(|_| core_changed(Path::new(relative)))?;
        if (exists && !valid) || (require_present && !valid) {
            return Err(core_changed(Path::new(relative)));
        }
    }
    Ok(())
}

fn create_required_directories(safe_root: &SafeRoot) -> Result<(), TelosError> {
    for relative in SUBDIRS {
        safe_root
            .create_directory(Path::new(relative))
            .map_err(|error| core_write_error("create", relative, error))?;
    }
    Ok(())
}

fn compute_lock_bytes(root: &Path, git: &GitRepo) -> Result<Vec<u8>, TelosError> {
    let ws = Workspace::discover(root)?;
    ws.config.validate_self()?;
    let model = ws.load_model().map_err(first_error)?;
    require_sealable_structure(&ws, &model)?;
    Ok(render_lock(&seal(&ws, &model, git, None)?).into_bytes())
}

fn render_lock(lock: &Lock) -> String {
    let mut out = String::new();
    writeln!(out, "version = {}", lock.version).unwrap();
    writeln!(out, "tool = {}", quote_lock(&lock.tool)).unwrap();
    if let Some(id) = &lock.sealed_by {
        writeln!(out, "sealed_by = {}", quote_lock(&id.to_string())).unwrap();
    }
    writeln!(out, "spec_digest = {}", quote_lock(&lock.spec_digest)).unwrap();
    out.push_str("\n[spec]\n");
    for (path, oid) in &lock.spec {
        writeln!(
            out,
            "{} = {}",
            quote_lock(path.as_str()),
            quote_lock(&oid.0)
        )
        .unwrap();
    }
    out.push_str("\n[code]\n");
    for (path, oid) in &lock.code {
        writeln!(
            out,
            "{} = {}",
            quote_lock(path.as_str()),
            quote_lock(&oid.0)
        )
        .unwrap();
    }
    out
}

fn quote_lock(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn read_core(safe_root: &SafeRoot, relative: &str) -> Result<Option<Vec<u8>>, TelosError> {
    safe_root
        .read_optional(Path::new(relative))
        .map_err(|_| core_changed(Path::new(relative)))
}

fn core_write_error(verb: &str, relative: &str, error: std::io::Error) -> TelosError {
    match error.kind() {
        std::io::ErrorKind::AlreadyExists
        | std::io::ErrorKind::InvalidInput
        | std::io::ErrorKind::NotADirectory
        | std::io::ErrorKind::PermissionDenied => core_changed(Path::new(relative)),
        _ => io_error(verb, Path::new(relative), error),
    }
}

fn core_changed(path: &Path) -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!(
            "`{}` no longer matches the incomplete Telos init transaction",
            display_path(path)
        ),
    )
    .hint(format!(
        "preserve `{INIT_MARKER_PATH}` and restore the transaction-owned path before retrying"
    ))
}

fn foreign_telos_owner(path: &Path) -> TelosError {
    TelosError::new(
        ErrorCode::TelosAlreadyInitialized,
        format!("`{}` already has a foreign owner", display_path(path)),
    )
    .hint("move the pre-existing Telos entry aside before retrying `telos init`")
}

fn initial_config_bytes(hosts: &[AgentHost]) -> Result<Vec<u8>, TelosError> {
    let mut config = Config {
        code: Globs::default(),
        tests: Globs::default(),
        test: TestCfg::default(),
        policy: Policy {
            tdd: TddPolicy::Strict,
        },
        agents: AgentsCfg {
            hosts: hosts
                .iter()
                .map(|host| match host {
                    AgentHost::Claude => ConfigAgentHost::Claude,
                    AgentHost::Codex => ConfigAgentHost::Codex,
                })
                .collect(),
        },
    };
    config.normalize();
    Ok(emit_config(&config)?.into_bytes())
}

fn requested_options(hosts: &[AgentHost], ci: Option<CiProvider>) -> (Vec<String>, Option<String>) {
    let hosts = agents::normalize(hosts)
        .into_iter()
        .map(|host| match host {
            AgentHost::Claude => "claude".to_string(),
            AgentHost::Codex => "codex".to_string(),
        })
        .collect();
    let ci = ci.map(|provider| match provider {
        CiProvider::Github => "github".to_string(),
    });
    (hosts, ci)
}

fn marker_bytes(marker: &InitMarker) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(marker)
        .expect("the init marker contains only serializable values");
    bytes.push(b'\n');
    bytes
}

fn already_initialized(config_path: &Path) -> TelosError {
    TelosError::new(
        ErrorCode::TelosAlreadyInitialized,
        format!("`{}` already exists", display_path(config_path)),
    )
    .hint("project already initialized; see `telos status`")
}

fn marker_collision() -> TelosError {
    TelosError::new(
        ErrorCode::TelosChangeStateInvalid,
        format!("`{INIT_MARKER_PATH}` is not a matching Telos init transaction"),
    )
    .hint("preserve or move the existing marker before retrying `telos init`")
}

fn marker_error(verb: &str, error: std::io::Error) -> TelosError {
    match error.kind() {
        std::io::ErrorKind::AlreadyExists
        | std::io::ErrorKind::InvalidInput
        | std::io::ErrorKind::NotADirectory
        | std::io::ErrorKind::PermissionDenied => marker_collision(),
        _ => io_error(verb, Path::new(INIT_MARKER_PATH), error),
    }
}

fn gitattributes_bytes(initial: Option<&[u8]>) -> Result<Vec<u8>, TelosError> {
    let mut content = String::from_utf8(initial.unwrap_or_default().to_vec()).map_err(|error| {
        io_error(
            "read",
            Path::new(GITATTRIBUTES_PATH),
            std::io::Error::new(std::io::ErrorKind::InvalidData, error),
        )
    })?;

    // `lines()` strips the `\r` of a CRLF file too, so a checkout with
    // Windows endings is recognized as already carrying the rule.
    if content
        .lines()
        .any(|line| line.trim() == GITATTRIBUTES_LINE)
    {
        return Ok(content.into_bytes());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(GITATTRIBUTES_LINE);
    content.push('\n');

    Ok(content.into_bytes())
}

fn io_error(verb: &str, path: &Path, e: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {verb} {}: {e}", display_path(path)),
    )
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

/// A fresh project cannot fail to load -- but `load_model` reports
/// diagnostics, and the envelope surfaces one error, so the first diagnostic
/// is what a caller sees.
fn first_error(diagnostics: Vec<Diagnostic>) -> TelosError {
    diagnostics
        .into_iter()
        .next()
        .map(TelosError::from)
        .unwrap_or_else(|| {
            TelosError::new(
                ErrorCode::TelosInternal,
                "the workspace failed to load without reporting a diagnostic",
            )
        })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use telos_core::error::ErrorCode;

    use super::{
        INIT_MARKER_PATH, InitMarker, InitPhase, LOCK_PATH, marker_bytes, run,
        run_with_agent_renderer, run_with_boundary_hooks, run_with_hooks,
    };
    use crate::ci::CiProvider;
    use crate::commands::Ctx;
    use crate::commands::agents::{self, AgentHost};

    fn repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(status.success());
        tmp
    }

    fn error_code(result: crate::envelope::CmdResult) -> ErrorCode {
        match result {
            Ok(_) => panic!("expected init to fail"),
            Err(error) => error.code,
        }
    }

    fn fail_after_two_agent_publications(ctx: &Ctx) -> crate::envelope::CmdResult {
        let mut publications = 0;
        run_with_agent_renderer(ctx, &[AgentHost::Codex], Some(CiProvider::Github), |plan| {
            agents::render_with_before_publish(plan, |_relative| {
                publications += 1;
                if publications == 3 {
                    Err(io::Error::other("forced third agent publication failure"))
                } else {
                    Ok(())
                }
            })
        })
    }

    #[test]
    fn post_seal_agent_failure_resumes_only_with_exact_options() {
        let tmp = repo();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };
        fs::write(tmp.path().join("AGENTS.md"), "# Owner instructions\n").unwrap();
        let first = fail_after_two_agent_publications(&ctx);

        assert_eq!(error_code(first), ErrorCode::TelosInternal);
        assert!(tmp.path().join("telos/telos.lock").is_file());
        assert!(tmp.path().join(INIT_MARKER_PATH).is_file());
        assert!(tmp.path().join(".agents/skills/telos/SKILL.md").is_file());
        assert!(
            tmp.path()
                .join(".agents/skills/telos-challenger/SKILL.md")
                .is_file()
        );
        assert!(
            !tmp.path()
                .join(".agents/skills/telos-implementer/SKILL.md")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap(),
            "# Owner instructions\n"
        );
        assert!(!tmp.path().join(".github/workflows/telos.yml").exists());

        assert_eq!(
            error_code(run(&ctx, &[AgentHost::Claude], Some(CiProvider::Github))),
            ErrorCode::TelosAlreadyInitialized
        );

        let resumed = run_with_hooks(
            &ctx,
            &[AgentHost::Codex],
            Some(CiProvider::Github),
            |relative| -> io::Result<()> {
                panic!(
                    "sealed resume must not republish core path {}",
                    relative.display()
                )
            },
            agents::render,
        )
        .unwrap();
        assert_eq!(
            resumed.result,
            serde_json::json!({"root": "telos", "sealed": true})
        );
        assert!(!tmp.path().join(INIT_MARKER_PATH).exists());
        assert!(
            tmp.path()
                .join(".agents/skills/telos-implementer/SKILL.md")
                .is_file()
        );
        let agents = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(agents.starts_with("# Owner instructions\n"));
        assert_eq!(agents.matches("<!-- telos-sdd:start -->").count(), 1);
        assert!(tmp.path().join(".github/workflows/telos.yml").is_file());

        assert_eq!(
            error_code(run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github))),
            ErrorCode::TelosAlreadyInitialized
        );
    }

    #[test]
    fn retry_after_partial_agent_publication_refuses_corrupted_owned_markers() {
        let tmp = repo();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };
        let first = run_with_agent_renderer(&ctx, &[AgentHost::Codex], None, |plan| {
            agents::render_with_before_publish(plan, |relative| {
                if relative == Path::new(".codex/hooks.json") {
                    Err(io::Error::other("stop after AGENTS publication"))
                } else {
                    Ok(())
                }
            })
        });
        assert_eq!(error_code(first), ErrorCode::TelosInternal);
        let agents_path = tmp.path().join("AGENTS.md");
        let mut corrupted = fs::read_to_string(&agents_path).unwrap();
        corrupted.push_str("\n<!-- telos-sdd:start -->\nowner bytes after orphan marker\n");
        fs::write(&agents_path, corrupted).unwrap();
        let before = non_git_tree(tmp.path());

        let retried = run(&ctx, &[AgentHost::Codex], None);

        assert_eq!(error_code(retried), ErrorCode::TelosChangeStateInvalid);
        assert_eq!(non_git_tree(tmp.path()), before);
        assert!(tmp.path().join(INIT_MARKER_PATH).exists());
        assert!(!tmp.path().join(".codex/hooks.json").exists());
        assert!(!tmp.path().join(".codex/rules/telos.rules").exists());
    }

    #[test]
    fn pre_seal_partial_core_publication_resumes_without_duplicate_merge() {
        let tmp = repo();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };
        fs::write(tmp.path().join(".gitattributes"), b"# owner rule\n").unwrap();

        let first = run_with_hooks(
            &ctx,
            &[AgentHost::Codex],
            Some(CiProvider::Github),
            |relative| {
                if relative == Path::new(".gitattributes") {
                    Err(io::Error::other("forced pre-seal failure"))
                } else {
                    Ok(())
                }
            },
            agents::render,
        );

        assert_eq!(error_code(first), ErrorCode::TelosInternal);
        assert!(tmp.path().join(INIT_MARKER_PATH).is_file());
        assert!(tmp.path().join("telos/telos.toml").is_file());
        assert!(tmp.path().join("telos/context-map.tel").is_file());
        assert!(!tmp.path().join("telos/changes/counters.toml").exists());
        assert!(!tmp.path().join(LOCK_PATH).exists());
        assert!(!tmp.path().join(".agents").exists());

        run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github)).unwrap();
        let attributes = fs::read_to_string(tmp.path().join(".gitattributes")).unwrap();
        assert!(attributes.starts_with("# owner rule\n"));
        assert_eq!(attributes.matches("telos/** text eol=lf").count(), 1);
        assert!(!tmp.path().join(INIT_MARKER_PATH).exists());
    }

    #[test]
    fn failure_after_lock_publication_before_phase_change_is_resumable() {
        let tmp = repo();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };

        let first = run_with_hooks(
            &ctx,
            &[AgentHost::Codex],
            Some(CiProvider::Github),
            |relative| {
                if relative == Path::new(LOCK_PATH) {
                    Err(io::Error::other("forced pre-transition failure"))
                } else {
                    Ok(())
                }
            },
            agents::render,
        );

        assert_eq!(error_code(first), ErrorCode::TelosInternal);
        assert!(tmp.path().join(LOCK_PATH).is_file());
        assert!(tmp.path().join(INIT_MARKER_PATH).is_file());
        assert!(!tmp.path().join(".agents").exists());

        run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github)).unwrap();
        assert!(!tmp.path().join(INIT_MARKER_PATH).exists());
        assert!(tmp.path().join(".agents/skills/telos/SKILL.md").is_file());
        assert!(tmp.path().join(".github/workflows/telos.yml").is_file());
    }

    #[test]
    fn marker_mutation_at_core_boundary_prevents_every_core_publication() {
        for mutation in MarkerMutation::ALL {
            let tmp = repo();
            let ctx = Ctx {
                cwd: tmp.path().to_path_buf(),
            };
            let after_mutation = RefCell::new(None);

            let result = run_with_boundary_hooks(
                &ctx,
                &[AgentHost::Codex],
                Some(CiProvider::Github),
                || {
                    mutation.apply(tmp.path());
                    after_mutation.replace(Some(non_git_tree(tmp.path())));
                    Ok(())
                },
                |_| Ok(()),
                || Ok(()),
                agents::render,
            );

            assert_eq!(
                error_code(result),
                ErrorCode::TelosChangeStateInvalid,
                "{mutation:?}"
            );
            assert_eq!(
                non_git_tree(tmp.path()),
                after_mutation.into_inner().unwrap(),
                "{mutation:?}"
            );
            assert!(!tmp.path().join("telos").exists(), "{mutation:?}");
            assert!(!tmp.path().join(".agents").exists(), "{mutation:?}");
            assert!(marker_path(tmp.path()).exists(), "{mutation:?}");
        }
    }

    #[test]
    fn marker_mutation_at_integration_boundary_prevents_every_new_integration() {
        for mutation in MarkerMutation::ALL {
            let tmp = repo();
            let ctx = Ctx {
                cwd: tmp.path().to_path_buf(),
            };
            let stopped_at_sealed = run_with_boundary_hooks(
                &ctx,
                &[AgentHost::Codex],
                Some(CiProvider::Github),
                || Ok(()),
                |_| Ok(()),
                || Err(io::Error::other("stop before integration transition")),
                agents::render,
            );
            assert_eq!(error_code(stopped_at_sealed), ErrorCode::TelosInternal);
            let marker_before: InitMarker =
                serde_json::from_slice(&fs::read(marker_path(tmp.path())).unwrap()).unwrap();
            assert!(matches!(marker_before.phase, InitPhase::Sealed { .. }));
            assert!(!tmp.path().join(".agents").exists());
            let after_mutation = RefCell::new(None);

            let result = run_with_boundary_hooks(
                &ctx,
                &[AgentHost::Codex],
                Some(CiProvider::Github),
                || Ok(()),
                |_| Ok(()),
                || {
                    mutation.apply(tmp.path());
                    after_mutation.replace(Some(non_git_tree(tmp.path())));
                    Ok(())
                },
                agents::render,
            );

            assert_eq!(
                error_code(result),
                ErrorCode::TelosChangeStateInvalid,
                "{mutation:?}"
            );
            assert_eq!(
                non_git_tree(tmp.path()),
                after_mutation.into_inner().unwrap(),
                "{mutation:?}"
            );
            assert!(
                !tmp.path()
                    .join(".agents/skills/telos-implementer/SKILL.md")
                    .exists(),
                "{mutation:?}"
            );
            assert!(
                !tmp.path().join(".github/workflows/telos.yml").exists(),
                "{mutation:?}"
            );
            assert!(marker_path(tmp.path()).exists(), "{mutation:?}");
        }
    }

    #[test]
    fn a_foreign_init_marker_never_authorizes_project_writes() {
        let tmp = repo();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };
        fs::write(tmp.path().join(INIT_MARKER_PATH), b"foreign owner\n").unwrap();

        assert_eq!(
            error_code(run(&ctx, &[AgentHost::Codex], None)),
            ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(
            fs::read(tmp.path().join(INIT_MARKER_PATH)).unwrap(),
            b"foreign owner\n"
        );
        assert!(!tmp.path().join("telos").exists());
        assert!(!tmp.path().join(".agents").exists());
    }

    #[test]
    fn fresh_init_refuses_every_foreign_telos_owner_without_a_marker() {
        for (relative, directory) in [
            ("telos/context-map.tel", false),
            ("telos/changes/counters.toml", false),
            ("telos/telos.lock", false),
            ("telos/contexts/foreign/context.tel", false),
            ("telos/foreign", true),
            ("telos/context-map.tel", true),
        ] {
            let tmp = repo();
            let target = tmp.path().join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            if directory {
                fs::create_dir(&target).unwrap();
            } else {
                fs::write(&target, b"foreign owner\n").unwrap();
            }
            let before = non_git_tree(tmp.path());
            let ctx = Ctx {
                cwd: tmp.path().to_path_buf(),
            };

            assert_eq!(
                error_code(run(&ctx, &[AgentHost::Codex], None)),
                ErrorCode::TelosAlreadyInitialized,
                "{relative} directory={directory}"
            );
            assert_eq!(non_git_tree(tmp.path()), before);
            assert!(!tmp.path().join(INIT_MARKER_PATH).exists());
            assert!(!tmp.path().join(".agents").exists());
        }
    }

    #[test]
    fn fresh_init_accepts_only_empty_canonical_telos_directories() {
        let tmp = repo();
        for relative in ["telos/contexts", "telos/constraints", "telos/changes"] {
            fs::create_dir_all(tmp.path().join(relative)).unwrap();
        }
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };

        run(&ctx, &[], None).unwrap();
        assert!(tmp.path().join(LOCK_PATH).is_file());
    }

    #[test]
    fn fresh_init_refuses_an_active_unproved_prepopulation_without_sealing_it() {
        let tmp = repo();
        fs::create_dir_all(tmp.path().join("telos/contexts/billing")).unwrap();
        fs::write(
            tmp.path().join("telos/telos.toml"),
            b"[code]\nglobs = []\n\n[tests]\nglobs = []\n\n[test]\ncmd = \"\"\nreport = \"\"\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("telos/contexts/billing/context.tel"),
            b"context billing core \"Billing\" {\n  def \"Billing\"\n}\n",
        )
        .unwrap();
        let before = non_git_tree(tmp.path());
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };

        assert_eq!(
            error_code(run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github))),
            ErrorCode::TelosAlreadyInitialized
        );
        assert_eq!(non_git_tree(tmp.path()), before);
        assert!(!tmp.path().join(INIT_MARKER_PATH).exists());
        assert!(!tmp.path().join("telos/telos.lock").exists());
        assert!(!tmp.path().join(".agents").exists());
        assert!(!tmp.path().join(".github/workflows/telos.yml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn fresh_init_refuses_live_and_dangling_telos_symlinks_without_touching_owner() {
        use std::os::unix::fs::symlink;

        for dangling in [false, true] {
            let tmp = repo();
            let outside = tempfile::tempdir().unwrap();
            let owner = outside.path().join("owner");
            if !dangling {
                fs::write(&owner, b"outside owner\n").unwrap();
            }
            fs::create_dir_all(tmp.path().join("telos")).unwrap();
            symlink(&owner, tmp.path().join("telos/context-map.tel")).unwrap();
            let before = non_git_tree(tmp.path());
            let outside_before = non_git_tree(outside.path());
            let ctx = Ctx {
                cwd: tmp.path().to_path_buf(),
            };

            assert_eq!(
                error_code(run(&ctx, &[], None)),
                ErrorCode::TelosAlreadyInitialized
            );
            assert_eq!(non_git_tree(tmp.path()), before);
            assert_eq!(non_git_tree(outside.path()), outside_before);
            assert!(!tmp.path().join(INIT_MARKER_PATH).exists());
        }
    }

    #[test]
    fn post_seal_resume_rejects_foreign_core_bytes_without_any_write() {
        for relative in [
            "telos/telos.toml",
            "telos/context-map.tel",
            "telos/changes/counters.toml",
            "telos/telos.lock",
            ".gitattributes",
        ] {
            let tmp = repo();
            let ctx = Ctx {
                cwd: tmp.path().to_path_buf(),
            };
            assert_eq!(
                error_code(fail_after_two_agent_publications(&ctx)),
                ErrorCode::TelosInternal
            );
            fs::write(tmp.path().join(relative), b"foreign owner\n").unwrap();
            let before = non_git_tree(tmp.path());

            assert_eq!(
                error_code(run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github))),
                ErrorCode::TelosChangeStateInvalid,
                "{relative}"
            );
            assert_eq!(non_git_tree(tmp.path()), before, "{relative}");
            assert!(tmp.path().join(INIT_MARKER_PATH).is_file(), "{relative}");
        }
    }

    #[test]
    fn post_seal_resume_rejects_a_core_file_replaced_by_a_directory() {
        let tmp = repo();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };
        assert_eq!(
            error_code(fail_after_two_agent_publications(&ctx)),
            ErrorCode::TelosInternal
        );
        let context_map = tmp.path().join("telos/context-map.tel");
        fs::remove_file(&context_map).unwrap();
        fs::create_dir(&context_map).unwrap();
        let before = non_git_tree(tmp.path());

        assert_eq!(
            error_code(run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github))),
            ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(non_git_tree(tmp.path()), before);
        assert!(tmp.path().join(INIT_MARKER_PATH).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn post_seal_resume_rejects_a_symlinked_config_without_touching_its_owner() {
        use std::os::unix::fs::symlink;

        let tmp = repo();
        let outside = tempfile::tempdir().unwrap();
        let owner = outside.path().join("owner.toml");
        fs::write(&owner, b"foreign owner\n").unwrap();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };
        assert_eq!(
            error_code(fail_after_two_agent_publications(&ctx)),
            ErrorCode::TelosInternal
        );
        let config = tmp.path().join("telos/telos.toml");
        fs::remove_file(&config).unwrap();
        symlink(&owner, &config).unwrap();
        let before = non_git_tree(tmp.path());
        let outside_before = non_git_tree(outside.path());

        assert_eq!(
            error_code(run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github))),
            ErrorCode::TelosChangeStateInvalid
        );
        assert_eq!(non_git_tree(tmp.path()), before);
        assert_eq!(non_git_tree(outside.path()), outside_before);
        assert!(config.is_symlink());
        assert!(tmp.path().join(INIT_MARKER_PATH).is_file());
    }

    #[derive(Debug, Eq, PartialEq)]
    enum TreeEntry {
        Directory,
        File(Vec<u8>),
        Symlink(PathBuf),
    }

    #[derive(Clone, Copy, Debug)]
    enum MarkerMutation {
        Bytes,
        Phase,
        Directory,
    }

    impl MarkerMutation {
        const ALL: [Self; 3] = [Self::Bytes, Self::Phase, Self::Directory];

        fn apply(self, root: &Path) {
            let marker = marker_path(root);
            match self {
                Self::Bytes => fs::write(marker, b"foreign marker owner\n").unwrap(),
                Self::Phase => {
                    let mut parsed: InitMarker =
                        serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
                    parsed.phase = match parsed.phase {
                        InitPhase::Preparing => InitPhase::CoreWriting,
                        InitPhase::Sealed { lock } => InitPhase::Integrating { lock },
                        InitPhase::CoreWriting | InitPhase::Integrating { .. } => {
                            unreachable!("boundary tests begin in preparing or sealed")
                        }
                    };
                    fs::write(marker, marker_bytes(&parsed)).unwrap();
                }
                Self::Directory => {
                    fs::remove_file(&marker).unwrap();
                    fs::create_dir(marker).unwrap();
                }
            }
        }
    }

    fn marker_path(root: &Path) -> PathBuf {
        root.join(INIT_MARKER_PATH)
    }

    fn non_git_tree(root: &Path) -> BTreeMap<PathBuf, TreeEntry> {
        let mut entries = BTreeMap::new();
        snapshot_dir(root, root, &mut entries);
        entries
    }

    fn snapshot_dir(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, TreeEntry>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if relative == Path::new(".git") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.file_type().is_symlink() {
                entries.insert(relative, TreeEntry::Symlink(fs::read_link(path).unwrap()));
            } else if metadata.is_dir() {
                entries.insert(relative.clone(), TreeEntry::Directory);
                snapshot_dir(root, &path, entries);
            } else {
                entries.insert(relative, TreeEntry::File(fs::read(path).unwrap()));
            }
        }
    }
}
