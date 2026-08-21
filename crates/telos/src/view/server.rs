//! Read-only Axum projection served from the loopback interface.

use std::net::Ipv4Addr;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use telos_core::error::{ErrorCode, TelosError};
use telos_core::ids::IntentId;
use telos_core::workspace::Workspace;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::commands::{Ctx, diagnostics_to_error, project};

use super::html::{LinkMode, Page, render};
use super::model::ViewSnapshot;

const RELOAD_DEBOUNCE: Duration = Duration::from_millis(75);
const MAX_REBUILD_LATENCY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
struct WatchTiming {
    quiet: Duration,
    max_latency: Duration,
}

impl Default for WatchTiming {
    fn default() -> Self {
        Self {
            quiet: RELOAD_DEBOUNCE,
            max_latency: MAX_REBUILD_LATENCY,
        }
    }
}

#[derive(Default)]
struct PendingEvents {
    sequence: u64,
    first_relevant: Option<Instant>,
    last_relevant: Option<Instant>,
    last_relevant_sequence: u64,
    watcher_error: Option<(u64, String)>,
}

impl PendingEvents {
    fn next_sequence(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.sequence
    }

    fn record_relevant(&mut self, now: Instant) {
        let sequence = self.next_sequence();
        self.first_relevant.get_or_insert(now);
        self.last_relevant = Some(now);
        self.last_relevant_sequence = sequence;
    }

    fn record_error(&mut self, message: String) {
        let sequence = self.next_sequence();
        self.watcher_error = Some((sequence, message));
    }

    fn deadline(&self, timing: WatchTiming) -> Option<Instant> {
        Some(std::cmp::min(
            self.first_relevant? + timing.max_latency,
            self.last_relevant? + timing.quiet,
        ))
    }

    fn take_error(&mut self) -> Option<(u64, String)> {
        self.watcher_error.take()
    }

    fn take_rebuild_if_due(&mut self, now: Instant, timing: WatchTiming) -> Option<u64> {
        if self.deadline(timing)? > now {
            return None;
        }
        self.first_relevant = None;
        self.last_relevant = None;
        Some(self.last_relevant_sequence)
    }

    fn is_empty(&self) -> bool {
        self.first_relevant.is_none() && self.watcher_error.is_none()
    }
}

#[derive(Clone)]
struct WatchNotifier {
    root: Arc<PathBuf>,
    pending: Arc<Mutex<PendingEvents>>,
    sender: mpsc::Sender<()>,
}

struct WatchQueue {
    pending: Arc<Mutex<PendingEvents>>,
    receiver: mpsc::Receiver<()>,
}

impl WatchNotifier {
    fn channel(root: PathBuf) -> (Self, WatchQueue) {
        let pending = Arc::new(Mutex::new(PendingEvents::default()));
        let (sender, receiver) = mpsc::channel(1);
        (
            Self {
                root: Arc::new(root),
                pending: Arc::clone(&pending),
                sender,
            },
            WatchQueue { pending, receiver },
        )
    }

    fn record(&self, event: notify::Result<Event>) {
        let update = match event {
            Ok(event) if ignored_event(&self.root, &event) => return,
            Ok(_) => Ok(Instant::now()),
            Err(error) => Err(format!("file watcher: {error}")),
        };
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match update {
            Ok(now) => pending.record_relevant(now),
            Err(message) => pending.record_error(message),
        }
        drop(pending);
        let _ = self.sender.try_send(());
    }
}

enum WatchWork {
    WatcherError { sequence: u64, message: String },
    Rebuild { sequence: u64 },
}

struct LiveState {
    snapshot: ViewSnapshot,
    reload_error: Option<String>,
    watcher_error: Option<WatcherFailure>,
}

struct WatcherFailure {
    sequence: u64,
    message: String,
}

impl LiveState {
    fn new(snapshot: ViewSnapshot) -> Self {
        Self {
            snapshot,
            reload_error: None,
            watcher_error: None,
        }
    }

    fn record_reload_error(&mut self, message: String) {
        self.reload_error = Some(message);
    }

    fn record_watcher_error(&mut self, sequence: u64, message: String) {
        self.watcher_error = Some(WatcherFailure { sequence, message });
    }

    fn record_reload_success(&mut self, sequence: u64, snapshot: ViewSnapshot) {
        self.snapshot = snapshot;
        self.reload_error = None;
        if self
            .watcher_error
            .as_ref()
            .is_some_and(|error| sequence > error.sequence)
        {
            self.watcher_error = None;
        }
    }
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
        let root = Workspace::discover(&ctx.cwd)?.repo_root;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| io_error("bind the loopback view server", error))?;
        let local_port = listener
            .local_addr()
            .map_err(|error| io_error("read the loopback view address", error))?
            .port();
        let (notifier, queue) = WatchNotifier::channel(root.clone());
        let (watcher, snapshot) = subscribe_before_build(
            || {
                let notifier = notifier.clone();
                let mut watcher = notify::recommended_watcher(move |event| {
                    notifier.record(event);
                })
                .map_err(|error| watcher_error("start the file watcher", error))?;
                watcher
                    .watch(&root, RecursiveMode::Recursive)
                    .map_err(|error| watcher_error("watch the repository", error))?;
                Ok(watcher)
            },
            || build_snapshot(ctx).map(|(_, snapshot)| snapshot),
        )?;
        let state = Arc::new(RwLock::new(LiveState::new(snapshot)));
        let reload_state = Arc::clone(&state);
        tokio::spawn(watch_loop(queue, WatchTiming::default(), move |work| {
            process_watch_work(&root, &reload_state, work)
        }));
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

fn subscribe_before_build<S, T, E>(
    subscribe: impl FnOnce() -> Result<S, E>,
    build: impl FnOnce() -> Result<T, E>,
) -> Result<(S, T), E> {
    let subscription = subscribe()?;
    let snapshot = build()?;
    Ok((subscription, snapshot))
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
        Some(html) => Html(add_reload_banner(
            html,
            state.reload_error.as_deref(),
            state
                .watcher_error
                .as_ref()
                .map(|error| error.message.as_str()),
        ))
        .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn add_reload_banner(
    mut html: String,
    reload_error: Option<&str>,
    watcher_error: Option<&str>,
) -> String {
    if reload_error.is_none() && watcher_error.is_none() {
        return html;
    }
    let mut banner = String::from(
        "<body><aside class=\"reload-error\" role=\"alert\" style=\"padding:12px;background:#7f1d1d;color:#fff\">",
    );
    if let Some(error) = reload_error {
        banner.push_str(&format!(
            "<p><strong>Reload error:</strong> {}</p>",
            escape(error)
        ));
    }
    if let Some(error) = watcher_error {
        banner.push_str(&format!(
            "<p><strong>Watcher error:</strong> {}</p>",
            escape(error)
        ));
    }
    banner.push_str("</aside>");
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

async fn watch_loop(
    mut queue: WatchQueue,
    timing: WatchTiming,
    mut process: impl FnMut(WatchWork),
) {
    let mut channel_closed = false;
    loop {
        if !channel_closed {
            channel_closed = queue.receiver.recv().await.is_none();
        }

        loop {
            let (error, deadline) = {
                let mut pending = queue
                    .pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                (pending.take_error(), pending.deadline(timing))
            };
            if let Some((sequence, message)) = error {
                process(WatchWork::WatcherError { sequence, message });
                continue;
            }
            let Some(deadline) = deadline else {
                break;
            };

            let now = Instant::now();
            if now < deadline {
                if channel_closed {
                    tokio::time::sleep(deadline.duration_since(now)).await;
                } else {
                    match tokio::time::timeout(deadline.duration_since(now), queue.receiver.recv())
                        .await
                    {
                        Ok(Some(())) => continue,
                        Ok(None) => {
                            channel_closed = true;
                            continue;
                        }
                        Err(_) => {}
                    }
                }
            }

            let sequence = queue
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take_rebuild_if_due(Instant::now(), timing);
            if let Some(sequence) = sequence {
                process(WatchWork::Rebuild { sequence });
            }
        }

        if channel_closed {
            let empty = queue
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty();
            if empty {
                return;
            }
        }
    }
}

fn process_watch_work(root: &FsPath, state: &SharedState, work: WatchWork) {
    match work {
        WatchWork::WatcherError { sequence, message } => state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_watcher_error(sequence, message),
        WatchWork::Rebuild { sequence } => {
            let ctx = Ctx {
                cwd: root.to_path_buf(),
            };
            match build_snapshot(&ctx) {
                Ok((_, snapshot)) => state
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .record_reload_success(sequence, snapshot),
                Err(error) => state
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .record_reload_error(error.message),
            }
        }
    }
}

fn build_snapshot(ctx: &Ctx) -> Result<(PathBuf, ViewSnapshot), TelosError> {
    let project = project(ctx)?;
    let model = project.ws.load_model().map_err(diagnostics_to_error)?;
    let root = project.ws.repo_root.clone();
    Ok((root, ViewSnapshot::build(&project.state, &model)))
}

fn ignored_event(root: &FsPath, event: &Event) -> bool {
    if event.paths.is_empty() || event.need_rescan() {
        return false;
    }
    !event.paths.iter().any(|path| {
        let relative = path.strip_prefix(root).unwrap_or(path);
        !relative
            .components()
            .enumerate()
            .any(|(index, component)| match component {
                Component::Normal(name) => {
                    let name = name.to_string_lossy();
                    (index == 0 && name == ".git")
                        || (index == 0 && name == "target")
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use notify::{Event, EventKind};
    use telos_core::state::{ProjectStateKind, StateReport};
    use telos_core::workspace::Workspace;
    use tokio::sync::mpsc::error::TryRecvError;

    use super::{
        LiveState, MAX_REBUILD_LATENCY, PendingEvents, WatchNotifier, WatchTiming, WatchWork,
        add_reload_banner, ignored_event, subscribe_before_build, watch_loop,
    };
    use crate::view::model::ViewSnapshot;

    fn event(path: &str) -> Event {
        Event::new(EventKind::Any).add_path(PathBuf::from(path))
    }

    fn fixture_snapshot() -> ViewSnapshot {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../telos-core/tests/corpus/billing");
        let workspace = Workspace::discover(&fixture).unwrap();
        let model = workspace.load_model().unwrap();
        ViewSnapshot::build(
            &StateReport {
                state: ProjectStateKind::Coherent,
                drift: vec![],
                open_changes: vec![],
            },
            &model,
        )
    }

    #[test]
    fn startup_subscribes_before_the_authoritative_build() {
        let source = Arc::new(Mutex::new(String::from("old snapshot")));
        let subscribed = Arc::new(Barrier::new(2));
        let edited = Arc::new(Barrier::new(2));
        let editor = {
            let source = Arc::clone(&source);
            let subscribed = Arc::clone(&subscribed);
            let edited = Arc::clone(&edited);
            thread::spawn(move || {
                subscribed.wait();
                *source.lock().unwrap() = String::from("edited after subscription");
                edited.wait();
            })
        };

        let (subscription, snapshot) = subscribe_before_build(
            || -> Result<_, ()> {
                subscribed.wait();
                edited.wait();
                Ok("watching")
            },
            || -> Result<_, ()> { Ok(source.lock().unwrap().clone()) },
        )
        .unwrap();
        editor.join().unwrap();

        assert_eq!(subscription, "watching");
        assert_eq!(snapshot, "edited after subscription");
    }

    #[test]
    fn callback_filters_ignored_paths_and_coalesces_a_burst_to_one_wake() {
        let (notifier, mut queue) = WatchNotifier::channel(PathBuf::from("/repo"));

        for path in [
            "/repo/.git/index",
            "/repo/target/debug/telos",
            "/repo/.site.telos-staging-42/index.html",
        ] {
            notifier.record(Ok(event(path)));
        }
        assert!(matches!(
            queue.receiver.try_recv(),
            Err(TryRecvError::Empty)
        ));

        for _ in 0..10_000 {
            notifier.record(Ok(event("/repo/telos/intents/INT-0042.tel")));
        }
        assert_eq!(queue.receiver.try_recv(), Ok(()));
        assert!(matches!(
            queue.receiver.try_recv(),
            Err(TryRecvError::Empty)
        ));
        assert_eq!(
            queue.pending.lock().unwrap().last_relevant_sequence,
            10_000,
            "a full wake channel must retain the newest dirty event"
        );
    }

    #[test]
    fn watcher_ignores_only_repository_metadata_and_build_outputs() {
        let root = PathBuf::from("/repo");

        assert!(ignored_event(&root, &event("/repo/target/debug/telos")));
        assert!(ignored_event(&root, &event("/repo/.git/index")));
        assert!(!ignored_event(
            &root,
            &event("/repo/.project-meta/progress.md")
        ));
        assert!(!ignored_event(
            &root,
            &event("/repo/examples/target/source.rs")
        ));
        assert!(!ignored_event(
            &root,
            &event("/repo/examples/.git/source.rs")
        ));
        assert!(!ignored_event(
            &root,
            &event("/repo/examples/.project-meta/source.rs")
        ));
    }

    #[test]
    fn sustained_writes_are_capped_from_the_first_relevant_event() {
        let start = Instant::now();
        let mut pending = PendingEvents::default();
        pending.record_relevant(start);
        pending.record_relevant(start + Duration::from_millis(490));

        assert_eq!(
            pending.deadline(WatchTiming::default()).unwrap(),
            start + MAX_REBUILD_LATENCY
        );
    }

    #[test]
    fn an_event_arriving_during_rebuild_schedules_another_rebuild() {
        let (notifier, queue) = WatchNotifier::channel(PathBuf::from("/repo"));
        notifier.record(Ok(event("/repo/telos/intents/INT-0042.tel")));
        let during_rebuild = notifier.clone();
        drop(notifier);

        let entered_rebuild = Arc::new(Barrier::new(2));
        let event_recorded = Arc::new(Barrier::new(2));
        let sender_thread = {
            let entered_rebuild = Arc::clone(&entered_rebuild);
            let event_recorded = Arc::clone(&event_recorded);
            thread::spawn(move || {
                entered_rebuild.wait();
                during_rebuild.record(Ok(event("/repo/telos/bindings.tel")));
                drop(during_rebuild);
                event_recorded.wait();
            })
        };

        let mut rebuilds = Vec::new();
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(watch_loop(
                queue,
                WatchTiming {
                    quiet: Duration::ZERO,
                    max_latency: Duration::ZERO,
                },
                |work| {
                    if let WatchWork::Rebuild { sequence } = work {
                        rebuilds.push(sequence);
                        if rebuilds.len() == 1 {
                            entered_rebuild.wait();
                            event_recorded.wait();
                        }
                    }
                },
            ));
        sender_thread.join().unwrap();

        assert_eq!(rebuilds.len(), 2);
        assert!(rebuilds[1] > rebuilds[0]);
    }

    #[test]
    fn watcher_health_clears_only_after_a_later_successful_relevant_batch() {
        let snapshot = fixture_snapshot();
        let mut live = LiveState::new(snapshot.clone());
        live.record_reload_error("invalid model".to_string());
        live.record_watcher_error(2, "watch backend failed".to_string());

        live.record_reload_success(1, snapshot.clone());
        assert!(live.reload_error.is_none());
        assert_eq!(
            live.watcher_error
                .as_ref()
                .map(|error| error.message.as_str()),
            Some("watch backend failed")
        );

        live.record_reload_success(3, snapshot);
        assert!(live.reload_error.is_none());
        assert!(live.watcher_error.is_none());
    }

    #[test]
    fn banner_combines_and_escapes_model_and_watcher_failures() {
        let mut live = LiveState::new(fixture_snapshot());
        live.record_reload_error("model <invalid> & stale".to_string());
        live.record_watcher_error(4, "watch <offline> & stale".to_string());

        let html = add_reload_banner(
            "<body><main>last good</main></body>".to_string(),
            live.reload_error.as_deref(),
            live.watcher_error
                .as_ref()
                .map(|error| error.message.as_str()),
        );

        assert!(html.contains("Reload error:</strong> model &lt;invalid&gt; &amp; stale"));
        assert!(html.contains("Watcher error:</strong> watch &lt;offline&gt; &amp; stale"));
        assert!(!html.contains("<invalid>"));
        assert!(!html.contains("<offline>"));
    }
}
