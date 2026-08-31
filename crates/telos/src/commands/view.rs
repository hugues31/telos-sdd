//! Static export and the foreground live-view lifecycle.

use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use serde_json::json;
use telos_core::changes::scan_changes;
use telos_core::error::{ErrorCode, TelosError};

use telos_core::state::{ProjectStateKind, compute_state};

use crate::commands::{
    Ctx, diagnostics_to_error, project, require_no_open_changes, require_no_unclaimed_drift,
};
use crate::envelope::{CmdResult, Outcome};
use crate::render::render;
use crate::view::export::export as export_snapshot;
use crate::view::model::ViewSnapshot;
use crate::view::server::LiveServer;

pub fn export(ctx: &Ctx, destination: &str, open: bool) -> CmdResult {
    let outcome = export_with_after_model(ctx, destination, || Ok(()))?;
    if open {
        open_in_browser(&ctx.cwd.join(destination).join("index.html"))?;
    }
    Ok(outcome)
}

fn open_in_browser(target: &Path) -> Result<(), TelosError> {
    let target = target.to_str().ok_or_else(|| {
        browser_open_error(
            &target.to_string_lossy(),
            io::Error::new(io::ErrorKind::InvalidInput, "view path is not valid UTF-8"),
        )
    })?;
    open_target_in_browser(target)
}

fn open_target_in_browser(target: &str) -> Result<(), TelosError> {
    webbrowser::open(target).map_err(|error| browser_open_error(target, error))
}

fn browser_open_error(target: &str, error: io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to open the Telos view in the default web browser: {error}"),
    )
    .hint(format!("open `{target}` manually"))
}

fn export_with_after_model<F>(ctx: &Ctx, destination: &str, after_model: F) -> CmdResult
where
    F: FnOnce() -> Result<(), TelosError>,
{
    let mut project = project(ctx)?;
    // Match `check --sealed` gate order exactly: damage wins over work in
    // progress, and neither state is permitted to publish as a sealed view.
    require_no_unclaimed_drift(&project)?;
    require_no_open_changes(&project)?;

    debug_assert_eq!(project.state.state, ProjectStateKind::Coherent);
    let model = project.ws.load_model().map_err(diagnostics_to_error)?;
    after_model()?;
    // Authenticate the exact model read above. A normal save between the
    // first state pass and model loading is now visible here, and a newly
    // opened change is included by the fresh single scan.
    let scan = scan_changes(&project.ws)?;
    project.state = compute_state(&project.ws, &project.lock, &project.git, &scan.infos)?;
    project.changes = scan.infos;
    project.parsed = scan.parsed;
    require_no_unclaimed_drift(&project)?;
    require_no_open_changes(&project)?;
    let snapshot = ViewSnapshot::build(&project.state, &model);
    let files = export_snapshot(&snapshot, destination.as_ref())?;
    let files: Vec<String> = files
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();

    Ok(Outcome {
        result: json!({
            "mode": "export",
            "destination": destination,
            "files": files,
        }),
        human: format!("exported {} files to {}", files.len(), destination),
        next_actions: Vec::new(),
    })
}

/// Starts the exceptional foreground lifecycle used only by `telos view`.
///
/// The listener and watcher are ready before the success answer is rendered
/// and flushed, so a caller may connect as soon as it reads that first line.
pub fn serve(ctx: &Ctx, port: u16, open: bool, json: bool) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return render_startup_error(
                TelosError::new(
                    ErrorCode::TelosInternal,
                    format!("failed to start the live view runtime: {error}"),
                ),
                json,
            );
        }
    };

    let server = match runtime.block_on(LiveServer::bind(ctx, port)) {
        Ok(server) => server,
        Err(error) => return render_startup_error(error, json),
    };
    let url = server.url();
    if open && let Err(error) = open_target_in_browser(&url) {
        return render_startup_error(error, json);
    }
    let outcome = Outcome {
        result: json!({"mode": "server", "url": url}),
        human: url,
        next_actions: Vec::new(),
    };
    let (text, code) = render("view", Ok(outcome), json);
    println!("{text}");
    if std::io::stdout().flush().is_err() {
        return ExitCode::FAILURE;
    }
    if code != ExitCode::SUCCESS {
        return code;
    }

    match runtime.block_on(server.run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("live view server stopped: {}", error.message);
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn render_startup_error(error: TelosError, json: bool) -> ExitCode {
    let (text, code) = render("view", Err(error), json);
    if json {
        println!("{text}");
    } else {
        eprintln!("{text}");
    }
    code
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use telos_core::git::GitRepo;
    use telos_core::lock::seal;
    use telos_core::workspace::Workspace;

    use super::export_with_after_model;
    use crate::commands::Ctx;

    fn copy_dir(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn sealed_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(status.success());
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing");
        copy_dir(&source, tmp.path());

        let bindings_path = tmp.path().join("telos/contexts/billing/bindings.tel");
        let bindings = fs::read_to_string(&bindings_path).unwrap();
        let (first, rest) = bindings.split_once('\n').unwrap();
        fs::write(
            bindings_path,
            format!("{first}\nproves     \"tests/billing.rs\" -> SCN-0091\n{rest}"),
        )
        .unwrap();
        let config_path = tmp.path().join("telos/telos.toml");
        let config = fs::read_to_string(&config_path).unwrap();
        fs::write(
            config_path,
            config.replace("cmd = \"\"", "cmd = \"git --version\""),
        )
        .unwrap();

        let ws = Workspace::discover(tmp.path()).unwrap();
        let git = GitRepo::discover(tmp.path()).unwrap();
        let model = ws.load_model().unwrap();
        seal(&ws, &model, &git, None)
            .unwrap()
            .write(&ws.lock_path())
            .unwrap();
        tmp
    }

    #[test]
    fn export_refuses_a_normal_save_between_model_read_and_authentication() {
        let tmp = sealed_fixture();
        let destination = tmp.path().join("site");
        let intent_path = tmp
            .path()
            .join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel");
        let ctx = Ctx {
            cwd: tmp.path().to_path_buf(),
        };

        let result = export_with_after_model(&ctx, destination.to_str().unwrap(), || {
            let mut source = fs::read_to_string(&intent_path).unwrap();
            source.push('\n');
            fs::write(&intent_path, source).unwrap();
            Ok(())
        });

        let error = result.unwrap_err();
        assert_eq!(error.code, telos_core::error::ErrorCode::TelosDriftDetected);
        assert!(!destination.exists());
    }
}
