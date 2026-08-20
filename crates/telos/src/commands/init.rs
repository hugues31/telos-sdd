//! `telos init`: turn a git repository into a telos project.
//!
//! Everything is created at the *git* root, not at the current directory: a
//! lock seals repo-relative paths (`telos/notions/Invoice.tel`), and those
//! paths are only meaningful -- and only hashable by `git hash-object` --
//! relative to the worktree root. A `telos/` nested somewhere below it would
//! seal paths git cannot resolve.
//!
//! The order below matters: `.gitattributes` is written *before* sealing, so
//! that the very first seal already runs the `eol=lf` clean filter and the
//! OIDs it records are the cross-OS ones.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::json;

use telos_core::config::{
    AgentHost as ConfigAgentHost, AgentsCfg, Config, Globs, Policy, TddPolicy, TestCfg,
};
use telos_core::counters::{Counters, write_counters};
use telos_core::emit::emit_config;
use telos_core::error::{Diagnostic, ErrorCode, TelosError};
use telos_core::git::GitRepo;
use telos_core::lock::seal;
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
const SUBDIRS: [&str; 4] = ["notions", "intents", "constraints", "changes"];

const INIT_MARKER_PATH: &str = ".telos-init.json";

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
    let git = GitRepo::discover(&ctx.cwd)?;
    let root = git.root().to_path_buf();
    let telos_dir = root.join("telos");
    let config_path = telos_dir.join("telos.toml");
    let marker_bytes = init_marker_bytes(hosts, ci);
    let safe_root = SafeRoot::open(&root)
        .map_err(|error| io_error("open", Path::new("repository root"), error))?;
    let marker = safe_root
        .read_optional(Path::new(INIT_MARKER_PATH))
        .map_err(|error| marker_error("inspect", error))?;
    let resuming = marker.as_deref() == Some(marker_bytes.as_slice());

    if config_path.exists() && !resuming {
        return Err(already_initialized(&config_path));
    }
    if marker.is_some() && !resuming {
        if config_path.exists() {
            return Err(already_initialized(&config_path));
        }
        return Err(marker_collision());
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

    for subdir in SUBDIRS {
        let path = telos_dir.join(subdir);
        fs::create_dir_all(&path).map_err(|e| io_error("create", &path, e))?;
    }
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
    write(&config_path, &emit_config(&config)?)?;
    // Empty to the byte: `bindings.tel` must seal as git's empty blob,
    // `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391`, on every OS.
    write(&telos_dir.join("bindings.tel"), "")?;
    ensure_gitattributes(&root)?;

    let ws = Workspace::discover(&root)?;
    // Seeded before sealing (D4): `telos/changes/` is excluded from
    // `Workspace::spec_files`, so this write never enters the seal.
    write_counters(&ws, &Counters::default())?;
    let model = ws.load_model().map_err(first_error)?;
    let lock = seal(&ws, &model, &git, None)?;
    lock.write(&ws.lock_path())?;
    render_agents(&agent_plan)?;
    ci::render(&ci_plan)?;
    safe_root
        .remove_file_if_matches(Path::new(INIT_MARKER_PATH), &marker_bytes)
        .map_err(|error| marker_error("remove", error))?;

    Ok(Outcome {
        result: json!({ "root": "telos", "sealed": true }),
        human: "initialized telos/ and sealed telos/telos.lock".to_string(),
        next_actions: vec!["telos status".to_string()],
    })
}

fn init_marker_bytes(hosts: &[AgentHost], ci: Option<CiProvider>) -> Vec<u8> {
    let hosts: Vec<&str> = agents::normalize(hosts)
        .into_iter()
        .map(|host| match host {
            AgentHost::Claude => "claude",
            AgentHost::Codex => "codex",
        })
        .collect();
    let ci = ci.map(|provider| match provider {
        CiProvider::Github => "github",
    });
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "format": "telos-init-v1",
        "agents": hosts,
        "ci": ci,
    }))
    .expect("the init marker contains only serializable literals");
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

/// Makes sure `.gitattributes` at the repository root carries
/// [`GITATTRIBUTES_LINE`], creating the file if it is absent and appending to
/// it otherwise -- never rewriting what is already there. An existing file
/// that does not end in a newline gets one first, so the append cannot glue
/// itself onto somebody else's rule. Already present (on a line of its own,
/// whitespace aside) means there is nothing to do.
fn ensure_gitattributes(root: &Path) -> Result<(), TelosError> {
    let path = root.join(".gitattributes");

    let mut content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_error("read", &path, e)),
    };

    // `lines()` strips the `\r` of a CRLF file too, so a checkout with
    // Windows endings is recognized as already carrying the rule.
    if content
        .lines()
        .any(|line| line.trim() == GITATTRIBUTES_LINE)
    {
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(GITATTRIBUTES_LINE);
    content.push('\n');

    write(&path, &content)
}

fn write(path: &Path, content: &str) -> Result<(), TelosError> {
    fs::write(path, content).map_err(|e| io_error("write", path, e))
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
    use std::fs;
    use std::io;
    use std::process::Command;

    use telos_core::error::ErrorCode;

    use super::{INIT_MARKER_PATH, run, run_with_agent_renderer};
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

    #[test]
    fn post_seal_agent_failure_resumes_only_with_exact_options() {
        let tmp = repo();
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };
        fs::write(tmp.path().join("AGENTS.md"), "# Owner instructions\n").unwrap();
        let mut publications = 0;

        let first = run_with_agent_renderer(
            &ctx,
            &[AgentHost::Codex],
            Some(CiProvider::Github),
            |plan| {
                agents::render_with_before_publish(plan, |_relative| {
                    publications += 1;
                    if publications == 3 {
                        Err(io::Error::other("forced third agent publication failure"))
                    } else {
                        Ok(())
                    }
                })
            },
        );

        assert_eq!(error_code(first), ErrorCode::TelosInternal);
        assert_eq!(publications, 3);
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

        let resumed = run(&ctx, &[AgentHost::Codex], Some(CiProvider::Github)).unwrap();
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
}
