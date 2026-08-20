//! Static export and the foreground live-view lifecycle.

use std::io::Write;
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

pub fn export(ctx: &Ctx, destination: &str) -> CmdResult {
    let mut project = project(ctx)?;
    // Match `check --sealed` gate order exactly: damage wins over work in
    // progress, and neither state is permitted to publish as a sealed view.
    require_no_unclaimed_drift(&project)?;
    require_no_open_changes(&project)?;

    debug_assert_eq!(project.state.state, ProjectStateKind::Coherent);
    let model = project.ws.load_model().map_err(diagnostics_to_error)?;
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
pub fn serve(ctx: &Ctx, port: u16, json: bool) -> ExitCode {
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
