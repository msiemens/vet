//! The composition root: serves protocol v1 from the real components. Session
//! logic lives in `vet-app`; this crate wires adapters to it and maps its views
//! to protocol messages.

mod highlight;
mod run;
mod semantic;

pub use run::OUTPUT_CAP;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::AbortHandle;
use vet_app::gc::Retention;
use vet_app::ignore::IgnoringWatcher;
use vet_app::limits::{Limiting, RestoreLimits};
use vet_app::recent::RecentProjects;
use vet_app::semantics::Semantics;
use vet_app::servers::Servers;
use vet_app::session::{Lifecycle, LiveSession, ThreadView};
use vet_app::settle::Settling;
use vet_app::trust::Trust;
use vet_core::review::ThreadId;
use vet_infra::clock::{TokioClock, unix_now};
use vet_infra::git::{GixRepo, find_root};
use vet_infra::lsp::{LspLauncher, config as lsp_config};
use vet_infra::materialize::Materialize;
use vet_infra::restore::ToolRestorer;
use vet_infra::servers::ToolResolver;
use vet_infra::store::SqliteStore;
use vet_infra::watch::FsWatcher;
use vet_mapping as mapping;
use vet_protocol::{Event, Request, Response, SessionId};
use vet_semantic::ports::{Backend, BackendId};
use vet_transport::{Events, Handler};

pub struct Config {
    /// How long the tree must be quiet before a snapshot settles (§6.2).
    pub debounce: Duration,
    /// Poll interval when the topology has no usable file events (§6.4).
    pub poll: Duration,
    /// How long a session without subscribers stays open, so a reloading
    /// shell finds it again.
    pub orphan_grace: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(1500),
            poll: Duration::from_secs(2),
            orphan_grace: Duration::from_secs(15),
        }
    }
}

type Live = LiveSession<GixRepo, SqliteStore>;
type LanguageServers = Servers<ToolResolver, SqliteStore>;
type Semantic = Semantics<
    GixRepo,
    SqliteStore,
    LspLauncher<Arc<LanguageServers>, Limiting<ToolRestorer>>,
    Materialize<GixRepo, SqliteStore>,
>;

/// The connections that opened a session. Sessions are one per repository:
/// opening an open repository subscribes to the existing one.
type Subscribers = Arc<Mutex<Vec<Events>>>;

struct Open {
    id: SessionId,
    repo: PathBuf,
    session: Arc<Live>,
    semantics: Semantic,
    subscribers: Subscribers,
    settling: AbortHandle,
    reaper: AbortHandle,
    runs: run::Runs,
}

fn live(subscribers: &Subscribers) -> std::sync::MutexGuard<'_, Vec<Events>> {
    let mut subscribers = subscribers.lock().unwrap_or_else(PoisonError::into_inner);
    subscribers.retain(|events| !events.is_closed());
    subscribers
}

pub struct Orchestrator {
    store: SqliteStore,
    recent: RecentProjects<SqliteStore>,
    config: Config,
    sessions: Sessions,
    servers: Arc<LanguageServers>,
    backends: Arc<[Backend]>,
    highlighter: highlight::Highlighter,
    /// The protocol id of the next session opened; unrelated to the store's ids.
    next_id: AtomicU32,
}

type Sessions = Arc<tokio::sync::Mutex<Vec<Arc<Open>>>>;

impl Open {
    /// Stops watching and kills the session's runs and language servers. The
    /// session stays open in the store, with its refs, for the next open of the
    /// repository to resume: the last connection leaving (a reload, an app
    /// quit) is not the end of a review.
    async fn shut(&self) {
        self.settling.abort();
        self.runs.stop_all();
        self.semantics.shutdown().await;
    }
}

/// Closes the session once no connection has been subscribed for `grace`.
async fn reap(sessions: Sessions, id: SessionId, grace: Duration) {
    let tick = (grace / 4).max(Duration::from_millis(20));
    let mut orphaned_since = None;
    loop {
        tokio::time::sleep(tick).await;
        let mut sessions = sessions.lock().await;
        let Some(open) = sessions.iter().find(|o| o.id == id).cloned() else {
            return;
        };
        if !live(&open.subscribers).is_empty() {
            orphaned_since = None;
            continue;
        }
        let since = *orphaned_since.get_or_insert_with(tokio::time::Instant::now);
        if since.elapsed() >= grace {
            // Holding the list lock, so a connection cannot attach meanwhile.
            sessions.retain(|o| o.id != id);
            open.shut().await;
            return;
        }
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl Orchestrator {
    pub fn new(store: SqliteStore, config: Config) -> Self {
        let backends: Arc<[Backend]> = lsp_config::defaults().into();
        let languages = backends.iter().map(|b| b.config().backend.clone()).collect();
        let resolver = ToolResolver::new(store.dir().to_owned());
        Self {
            servers: Arc::new(Servers::new(resolver, store.clone(), languages)),
            backends,
            highlighter: Default::default(),
            recent: RecentProjects::new(store.clone()),
            store,
            config,
            sessions: Sessions::default(),
            next_id: AtomicU32::new(1),
        }
    }

    /// Restarts the language's running servers in every session: their
    /// executable changed.
    async fn restart_language(&self, language: &str) {
        let language = BackendId(language.to_owned());
        let sessions: Vec<_> = self.sessions.lock().await.clone();
        for open in sessions {
            open.semantics.restart(&language).await;
        }
    }

    /// Tells the other connections of every session that server settings changed.
    async fn servers_changed(&self, from: &Events) {
        for open in self.sessions.lock().await.iter() {
            Self::notify_others(open, from, Event::ServersChanged { session: open.id });
        }
    }

    async fn list_servers(&self, open: &Open) -> Result<Response, String> {
        let all = self.servers.status(&open.repo).await.map_err(message)?;
        Ok(mapping::semantic::servers(&all))
    }

    async fn find(&self, id: SessionId) -> Result<Arc<Open>, String> {
        let sessions = self.sessions.lock().await;
        sessions
            .iter()
            .find(|open| open.id == id)
            .cloned()
            .ok_or_else(|| "no such session".to_string())
    }

    /// The session id and the root of the repository containing `repo`.
    async fn open(&self, repo: &str, events: Events) -> Result<(SessionId, String), String> {
        let repo = find_root(Path::new(repo))?;
        let root = repo.to_string_lossy().into_owned();
        let mut sessions = self.sessions.lock().await;
        if let Some(open) = sessions.iter().find(|open| open.repo == repo) {
            let mut subscribers = live(&open.subscribers);
            if !subscribers.iter().any(|e| e.same_connection(&events)) {
                subscribers.push(events);
            }
            return Ok((open.id, root));
        }

        // Watching starts before the initial scan, so no edit falls between them.
        let watcher = FsWatcher::open(&repo, self.config.poll).map_err(message)?;
        let git = GixRepo::new(&repo);
        let session = Arc::new(
            Live::open(git.clone(), self.store.clone(), root.clone(), unix_now)
                .await
                .map_err(message)?,
        );
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let subscribers: Subscribers = Arc::new(Mutex::new(vec![events]));

        let settling = Settling::new(
            IgnoringWatcher::new(watcher, git),
            TokioClock::new(),
            self.config.debounce.as_millis() as u64,
            Some(session.settled_snapshot().await),
        );
        let task = {
            let (session, subscribers) = (session.clone(), subscribers.clone());
            tokio::spawn(async move {
                session
                    .follow(settling, |revision| {
                        run::broadcast(&subscribers, Event::SnapshotSettled { session: id, revision });
                    })
                    .await;
            })
        };
        let reaper = tokio::spawn(reap(self.sessions.clone(), id, self.config.orphan_grace));
        let limits = RestoreLimits::default();
        let restorer = Limiting::new(
            ToolRestorer::new(),
            limits.clone(),
            Box::new({
                let subscribers = subscribers.clone();
                move |failures| {
                    let failures = mapping::semantic::restore_failures(&failures);
                    run::broadcast(&subscribers, Event::RestoreFailed { session: id, failures });
                }
            }),
        );
        let semantics = Semantics::new(
            Materialize::new(session.acquisition()),
            LspLauncher::new(self.servers.clone(), restorer),
            repo.clone(),
            self.backends.to_vec(),
            limits,
            Trust::Local,
        );
        sessions.push(Arc::new(Open {
            id,
            repo,
            session,
            semantics,
            subscribers,
            settling: task.abort_handle(),
            reaper: reaper.abort_handle(),
            runs: Default::default(),
        }));
        Ok((id, root))
    }

    /// Unsubscribes the connection; the last one to leave closes the session.
    async fn close(&self, id: SessionId, events: &Events) -> Result<(), String> {
        let open = self.find(id).await?;
        let last = {
            let mut subscribers = live(&open.subscribers);
            subscribers.retain(|e| !e.same_connection(events));
            subscribers.is_empty()
        };
        if last {
            self.sessions.lock().await.retain(|o| o.id != id);
            open.reaper.abort();
            open.shut().await;
        }
        Ok(())
    }

    /// Tells the other connections of the session about a change `from` made.
    fn notify_others(open: &Open, from: &Events, event: Event) {
        for events in live(&open.subscribers).iter().filter(|e| !e.same_connection(from)) {
            events.send(event.clone());
        }
    }
}

impl Handler for Orchestrator {
    async fn handle(&self, request: Request, events: Events) -> Result<Response, String> {
        if !vet_infra::timing::enabled() {
            return self.answer(request, events).await;
        }
        let name = format!("{request:?}")
            .split([' ', '{', '('])
            .next()
            .unwrap_or_default()
            .to_owned();
        let started = std::time::Instant::now();
        let response = self.answer(request, events).await;
        vet_infra::timing::log(|| format!("request {name}"), started.elapsed());
        response
    }
}

impl Orchestrator {
    async fn answer(&self, request: Request, events: Events) -> Result<Response, String> {
        match request {
            Request::Ping { payload } => Ok(Response::Pong { payload }),
            Request::RecentProjects => Ok(Response::RecentProjects {
                paths: self.recent.list().await.map_err(|e| e.0)?,
            }),
            Request::RecordRecentProject { path } => Ok(Response::RecentProjects {
                paths: self.recent.record(path).await.map_err(|e| e.0)?,
            }),
            Request::ForgetRecentProject { path } => Ok(Response::RecentProjects {
                paths: self.recent.forget(&path).await.map_err(|e| e.0)?,
            }),
            Request::OpenSession { repo } => {
                let (session, root) = self.open(&repo, events).await?;
                Ok(Response::Session { session, root })
            }
            Request::CloseSession { session } => {
                self.close(session, &events).await?;
                Ok(Response::Done)
            }
            Request::FinishReview { session } => {
                let open = self.find(session).await?;
                let complete = open.session.finish_review().await.map_err(message)?;
                let revision = open.session.revision().await;
                run::broadcast(&open.subscribers, Event::SnapshotSettled { session, revision });
                Ok(Response::ReviewFinished { complete })
            }
            Request::Manifest { session } => {
                let open = self.find(session).await?;
                let manifest = open.session.manifest().await.map_err(message)?;
                let failures = open.semantics.restore_failures();
                Ok(Response::Manifest(Box::new(mapping::manifest(
                    session, &manifest, &failures,
                ))))
            }
            Request::FileContents { session, path } => {
                let text = self
                    .find(session)
                    .await?
                    .session
                    .file_text(&path)
                    .await
                    .map_err(message)?;
                Ok(self.highlighter.file_contents(&path, text).await)
            }
            Request::SetSeen {
                session,
                revision,
                path,
                hunks,
                seen,
            } => {
                let open = self.find(session).await?;
                let update = open
                    .session
                    .set_seen(revision, &path, &hunks, seen)
                    .await
                    .map_err(message)?;
                announce_start(&open, update.started);
                Ok(Response::Review(review_changed(&open, &events, &update)))
            }
            Request::Acknowledge {
                session,
                revision,
                path,
                kind,
                acknowledged,
            } => {
                let open = self.find(session).await?;
                let update = open
                    .session
                    .acknowledge(revision, &path, mapping::core_non_text_kind(kind), acknowledged)
                    .await
                    .map_err(message)?;
                announce_start(&open, update.started);
                Ok(Response::Review(review_changed(&open, &events, &update)))
            }
            Request::ItemBlob {
                session,
                revision,
                path,
                kind,
                side,
            } => {
                let blob = self
                    .find(session)
                    .await?
                    .session
                    .item_blob(
                        revision,
                        &path,
                        mapping::core_non_text_kind(kind),
                        mapping::core_side(side),
                    )
                    .await
                    .map_err(message)?;
                Ok(mapping::item_blob(blob))
            }
            Request::SetBase { session, base } => {
                let open = self.find(session).await?;
                let (base, revision) = open
                    .session
                    .set_base(mapping::core_base_choice(base))
                    .await
                    .map_err(message)?;
                run::broadcast(&open.subscribers, Event::SnapshotSettled { session, revision });
                Ok(Response::Base(mapping::base(&base)))
            }
            Request::SetProjectFilter { session, filter } => {
                let open = self.find(session).await?;
                let revision = open
                    .session
                    .set_filter(mapping::core_filter(filter))
                    .await
                    .map_err(message)?;
                run::broadcast(&open.subscribers, Event::SnapshotSettled { session, revision });
                Ok(Response::Done)
            }
            Request::ListBaseCandidates { session } => {
                let candidates = self
                    .find(session)
                    .await?
                    .session
                    .base_candidates()
                    .await
                    .map_err(message)?;
                Ok(mapping::base_candidates(&candidates))
            }
            Request::ListDirectory { session, path } => {
                let listing = self
                    .find(session)
                    .await?
                    .session
                    .list_directory(&path)
                    .await
                    .map_err(message)?;
                Ok(mapping::directory(&listing))
            }
            Request::FileText { session, path } => {
                let text = self
                    .find(session)
                    .await?
                    .session
                    .target_text(&path)
                    .await
                    .map_err(message)?;
                Ok(self.highlighter.file_text(&path, &text).await)
            }
            Request::Threads { session } => {
                let threads = self.find(session).await?.session.threads().await.map_err(message)?;
                Ok(mapping::threads(&threads))
            }
            Request::TargetLine {
                session,
                revision,
                side,
                path,
                line,
            } => {
                let line = self
                    .find(session)
                    .await?
                    .session
                    .line_in_target(revision, mapping::core_side(side), &path, line)
                    .await
                    .map_err(message)?;
                Ok(Response::TargetLine { revision, line })
            }
            Request::CreateThread {
                session,
                revision,
                target,
                body,
            } => {
                let open = self.find(session).await?;
                let thread = open
                    .session
                    .create_thread(revision, mapping::new_thread(target), body, unix_now())
                    .await
                    .map_err(message)?;
                announce_start(&open, thread.started);
                Ok(thread_changed(&open, &events, &thread).await)
            }
            Request::Reply { session, thread, body } => {
                let open = self.find(session).await?;
                let thread = open
                    .session
                    .reply(ThreadId(thread.into()), body, unix_now())
                    .await
                    .map_err(message)?;
                Ok(thread_changed(&open, &events, &thread).await)
            }
            Request::SetThreadStatus {
                session,
                thread,
                status,
            } => {
                let open = self.find(session).await?;
                let thread = open
                    .session
                    .set_thread_status(ThreadId(thread.into()), mapping::core_thread_status(status))
                    .await
                    .map_err(message)?;
                Ok(thread_changed(&open, &events, &thread).await)
            }
            Request::DeleteThread { session, thread } => {
                let open = self.find(session).await?;
                open.session
                    .delete_thread(ThreadId(thread.into()))
                    .await
                    .map_err(message)?;
                threads_changed(&open, &events).await;
                Ok(Response::Done)
            }
            Request::ConfirmAnchor {
                session,
                revision,
                thread,
            } => {
                let open = self.find(session).await?;
                let thread = open
                    .session
                    .confirm_anchor(revision, ThreadId(thread.into()))
                    .await
                    .map_err(message)?;
                Ok(thread_changed(&open, &events, &thread).await)
            }
            Request::AttachAnchor {
                session,
                revision,
                thread,
                candidate,
            } => {
                let open = self.find(session).await?;
                let thread = open
                    .session
                    .attach_anchor(revision, ThreadId(thread.into()), candidate as usize)
                    .await
                    .map_err(message)?;
                Ok(thread_changed(&open, &events, &thread).await)
            }
            Request::ListRunTargets { session } => self.find(session).await?.run_targets().await,
            Request::SaveRunTarget { session, target } => {
                let open = self.find(session).await?;
                open.save_run_target(mapping::core_run_target(target)?).await?;
                Ok(Response::Done)
            }
            Request::ResetRunTarget { session, id } => {
                self.find(session).await?.reset_run_target(id).await?;
                Ok(Response::Done)
            }
            Request::Run {
                session,
                target_id,
                approved_hash,
            } => {
                let open = self.find(session).await?;
                open.run(&target_id, approved_hash.as_deref()).await
            }
            Request::RunApprovalDiff { session, hash } => self.find(session).await?.run_approval_diff(&hash).await,
            Request::StopRun { session, run_id } => {
                self.find(session).await?.stop_run(run_id);
                Ok(Response::Done)
            }
            Request::Hover {
                session,
                revision,
                side,
                path,
                line,
                col,
            } => {
                let open = self.find(session).await?;
                open.hover(revision, side, &path, line, col).await
            }
            Request::Definition {
                session,
                revision,
                side,
                path,
                line,
                col,
            } => {
                let open = self.find(session).await?;
                open.definition(revision, side, &path, line, col).await
            }
            Request::References {
                session,
                revision,
                side,
                path,
                line,
                col,
            } => {
                let open = self.find(session).await?;
                open.references(revision, side, &path, line, col).await
            }
            Request::Diagnostics {
                session,
                revision,
                path,
            } => self.find(session).await?.diagnostics(revision, &path).await,
            Request::ChangedSymbols { session, revision } => self.find(session).await?.changed_symbols(revision).await,
            Request::Signature {
                session,
                revision,
                symbol,
            } => self.find(session).await?.signature(revision, &symbol).await,
            Request::Impact {
                session,
                revision,
                symbol,
            } => self.find(session).await?.impact(revision, &symbol).await,
            Request::RemovalImpact {
                session,
                revision,
                symbol,
            } => self.find(session).await?.removal_impact(revision, &symbol).await,
            Request::Reachability {
                session,
                revision,
                symbol,
            } => self.find(session).await?.reachability(revision, &symbol).await,
            Request::ListServers { session } => {
                let open = self.find(session).await?;
                self.list_servers(&open).await
            }
            Request::InstallServer { session, language } => {
                let open = self.find(session).await?;
                let progress = |line: &str| {
                    run::broadcast(
                        &open.subscribers,
                        Event::ServerInstallOutput {
                            session,
                            language: language.clone(),
                            line: line.to_owned(),
                        },
                    );
                };
                self.servers.install(&language, &progress).await.map_err(message)?;
                self.restart_language(&language).await;
                self.servers_changed(&events).await;
                self.list_servers(&open).await
            }
            Request::SetServerPath {
                session,
                language,
                path,
            } => {
                let open = self.find(session).await?;
                self.servers.set_custom_path(&language, path).await.map_err(message)?;
                self.restart_language(&language).await;
                self.servers_changed(&events).await;
                self.list_servers(&open).await
            }
        }
    }
}

/// Tells every subscriber that a request started the review, as `started` says.
fn announce_start(open: &Open, started: Option<Lifecycle>) {
    if let Some(lifecycle) = started {
        run::broadcast(
            &open.subscribers,
            Event::ReviewStarted {
                session: open.id,
                review: mapping::review_info(lifecycle),
            },
        );
    }
}

/// Tells the other subscribers that threads changed.
async fn threads_changed(open: &Open, from: &Events) {
    let revision = open.session.revision().await;
    Orchestrator::notify_others(
        open,
        from,
        Event::ThreadsChanged {
            session: open.id,
            revision,
        },
    );
}

async fn thread_changed(open: &Open, from: &Events, thread: &ThreadView) -> Response {
    threads_changed(open, from).await;
    Response::Thread(mapping::thread(thread))
}

/// The protocol update for the requester; the other subscribers are told too.
fn review_changed(
    open: &Open,
    from: &Events,
    update: &vet_app::session::ReviewUpdateView,
) -> vet_protocol::ReviewUpdate {
    let mapped = mapping::review_update(update);
    Orchestrator::notify_others(
        open,
        from,
        Event::ReviewChanged {
            session: open.id,
            revision: update.revision,
            update: mapped.clone(),
        },
    );
    mapped
}

/// The store in the per-platform data directory (§8).
pub async fn open_default_store() -> Result<SqliteStore, String> {
    let dir = SqliteStore::default_dir().ok_or("no data directory")?;
    open_store(&dir).await
}

/// Opens the store and collects what its retention no longer keeps. A failed
/// collection only leaves the garbage for the next start.
pub async fn open_store(dir: &Path) -> Result<SqliteStore, String> {
    let store = SqliteStore::open(dir).await.map_err(|e| e.0)?;
    if let Err(e) = vet_app::gc::collect(&store, unix_now(), &Retention::default()).await {
        eprintln!("vet: garbage collection failed: {}", e.0);
    }
    Ok(store)
}
