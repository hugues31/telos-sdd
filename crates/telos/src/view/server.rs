//! Read-only Axum projection served from the loopback interface.

use std::net::Ipv4Addr;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::IntentId;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::commands::{Ctx, diagnostics_to_error, project};

use super::html::{LinkMode, Page, render};
use super::model::ViewSnapshot;

const RELOAD_DEBOUNCE: Duration = Duration::from_millis(75);

struct LiveState {
    snapshot: ViewSnapshot,
    reload_error: Option<String>,
}

type SharedState = Arc<RwLock<LiveState>>;

pub(crate) struct LiveServer {
    listener: TcpListener,
    router: Router,
    _watcher: RecommendedWatcher,
    local_port: u16,
}

impl LiveServer {
    pub(crate) async fn bind(ctx: &Ctx, port: u16) -> Result<Self, TelosError> {
        let (root, snapshot) = build_snapshot(ctx)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| io_error("bind the loopback view server", error))?;
        let local_port = listener
            .local_addr()
            .map_err(|error| io_error("read the loopback view address", error))?
            .port();
        let state = Arc::new(RwLock::new(LiveState {
            snapshot,
            reload_error: None,
        }));
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .map_err(|error| watcher_error("start the file watcher", error))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| watcher_error("watch the repository", error))?;
        tokio::spawn(reload_loop(root, Arc::clone(&state), receiver));
        let router = Router::new()
            .route("/", get(dashboard))
            .route("/graph", get(graph))
            .route("/intent/{id}", get(intent))
            .route("/glossary", get(glossary))
            .route("/coverage", get(coverage))
            .with_state(state);

        Ok(Self {
            listener,
            router,
            _watcher: watcher,
            local_port,
        })
    }

    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.local_port)
    }

    pub(crate) async fn run(self) -> Result<(), TelosError> {
        axum::serve(self.listener, self.router)
            .await
            .map_err(|error| io_error("serve the live view", error))
    }
}

async fn dashboard(State(state): State<SharedState>) -> Response {
    page_response(&state, Page::Dashboard)
}

async fn graph(State(state): State<SharedState>) -> Response {
    page_response(&state, Page::Graph)
}

async fn glossary(State(state): State<SharedState>) -> Response {
    page_response(&state, Page::Glossary)
}

async fn coverage(State(state): State<SharedState>) -> Response {
    page_response(&state, Page::Coverage)
}

async fn intent(State(state): State<SharedState>, Path(raw): Path<String>) -> Response {
    let Ok(id) = raw.parse::<IntentId>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    page_response(&state, Page::Intent(id))
}

fn page_response(state: &SharedState, page: Page) -> Response {
    let state = state
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match render(&state.snapshot, page, LinkMode::Server) {
        Some(html) => Html(add_reload_banner(html, state.reload_error.as_deref())).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn add_reload_banner(mut html: String, reload_error: Option<&str>) -> String {
    let Some(error) = reload_error else {
        return html;
    };
    let banner = format!(
        "<body><aside class=\"reload-error\" role=\"alert\" style=\"padding:12px;background:#7f1d1d;color:#fff\"><strong>Reload error:</strong> {}</aside>",
        escape(error)
    );
    html = html.replacen("<body>", &banner, 1);
    html
}

fn escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

async fn reload_loop(
    root: PathBuf,
    state: SharedState,
    mut receiver: mpsc::UnboundedReceiver<notify::Result<Event>>,
) {
    while let Some(event) = receiver.recv().await {
        match event {
            Err(error) => {
                set_reload_error(&state, format!("file watcher: {error}"));
                continue;
            }
            Ok(event) if ignored_event(&root, &event) => continue,
            Ok(_) => {}
        }

        let mut deadline = tokio::time::Instant::now() + RELOAD_DEBOUNCE;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, receiver.recv()).await {
                Err(_) => break,
                Ok(None) => return,
                Ok(Some(Err(error))) => {
                    set_reload_error(&state, format!("file watcher: {error}"));
                }
                Ok(Some(Ok(event))) if ignored_event(&root, &event) => {}
                Ok(Some(Ok(_))) => {
                    deadline = tokio::time::Instant::now() + RELOAD_DEBOUNCE;
                }
            }
        }

        let ctx = Ctx { cwd: root.clone() };
        match build_snapshot(&ctx) {
            Ok((_, snapshot)) => {
                let mut live = state
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *live = LiveState {
                    snapshot,
                    reload_error: None,
                };
            }
            Err(error) => set_reload_error(&state, error.message),
        }
    }
}

fn set_reload_error(state: &SharedState, error: String) {
    state
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .reload_error = Some(error);
}

fn build_snapshot(ctx: &Ctx) -> Result<(PathBuf, ViewSnapshot), TelosError> {
    let project = project(ctx)?;
    let model = project.ws.load_model().map_err(diagnostics_to_error)?;
    let root = project.ws.repo_root.clone();
    Ok((root, ViewSnapshot::build(&project.state, &model)))
}

fn ignored_event(root: &FsPath, event: &Event) -> bool {
    !event.paths.iter().any(|path| {
        let relative = path.strip_prefix(root).unwrap_or(path);
        !relative.components().any(|component| match component {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                name == ".git"
                    || name == "target"
                    || name == ".superpowers"
                    || (name.starts_with('.') && name.contains(".telos-staging-"))
            }
            _ => false,
        })
    })
}

fn io_error(action: &str, error: std::io::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {action}: {error}"),
    )
}

fn watcher_error(action: &str, error: notify::Error) -> TelosError {
    TelosError::new(
        ErrorCode::TelosInternal,
        format!("failed to {action}: {error}"),
    )
}
