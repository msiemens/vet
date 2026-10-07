//! The composition root: serves protocol v1 from the real components. Session
//! logic lives in `vet-app`; this crate wires adapters to it and maps its views
//! to protocol messages.

mod events;
mod highlight;
mod open;
mod review;
mod semantic;
mod servers;
mod threads;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use vet_app::gc::Retention;
use vet_app::limits::Limiting;
use vet_app::recent::RecentProjects;
use vet_app::registry::{Entry, Registry};
use vet_app::semantics::Semantics;
use vet_app::servers::Servers;
use vet_app::session::LiveSession;
use vet_infra::clock::unix_now;
use vet_infra::git::GixRepo;
use vet_infra::lsp::{LspLauncher, config as lsp_config};
use vet_infra::materialize::Materialize;
use vet_infra::restore::ToolRestorer;
use vet_infra::servers::ToolResolver;
use vet_infra::store::SqliteStore;
use vet_protocol::{Request, Response, SessionId};
use vet_semantic::ports::Backend;
use vet_transport::{Events, Handler};

use events::Connection;
use open::Wired;

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

/// An open session: sessions are one per repository, and opening an open
/// repository subscribes to the existing one.
type Open = Entry<Connection, Wired>;
type Sessions = Registry<Connection, Wired>;

pub struct Orchestrator {
    store: SqliteStore,
    recent: RecentProjects<SqliteStore>,
    config: Config,
    sessions: Sessions,
    servers: Arc<LanguageServers>,
    backends: Arc<[Backend]>,
    highlighter: highlight::Highlighter,
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
        }
    }

    async fn find(&self, id: SessionId) -> Result<Arc<Open>, String> {
        self.sessions.find(id).await.map_err(message)
    }
}

impl Handler for Orchestrator {
    async fn handle(&self, request: Request, events: Events) -> Result<Response, String> {
        if !vet_infra::timing::enabled() {
            return self.answer(request, Connection(events)).await;
        }
        let name = format!("{request:?}")
            .split([' ', '{', '('])
            .next()
            .unwrap_or_default()
            .to_owned();
        let started = std::time::Instant::now();
        let response = self.answer(request, Connection(events)).await;
        vet_infra::timing::log(|| format!("request {name}"), started);
        response
    }
}

impl Orchestrator {
    async fn answer(&self, request: Request, from: Connection) -> Result<Response, String> {
        let from = &from;
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
                let (session, root) = self.open(&repo, from.clone()).await?;
                Ok(Response::Session { session, root })
            }
            Request::CloseSession { session } => {
                self.close(session, from).await?;
                Ok(Response::Done)
            }
            Request::FinishReview { session } => self.finish_review(session).await,
            Request::Manifest { session } => self.manifest(session).await,
            Request::FileContents { session, path } => self.file_contents(session, &path).await,
            Request::ToggleHunk {
                session,
                revision,
                path,
                hunk,
            } => self.toggle_hunk(session, from, revision, &path, hunk).await,
            Request::ToggleItem {
                session,
                revision,
                path,
                kind,
            } => self.toggle_item(session, from, revision, &path, kind).await,
            Request::ToggleFileSeen {
                session,
                revision,
                path,
            } => self.toggle_file_seen(session, from, revision, &path).await,
            Request::ItemBlob {
                session,
                revision,
                path,
                kind,
                side,
            } => self.item_blob(session, revision, &path, kind, side).await,
            Request::SetBase { session, base } => self.set_base(session, base).await,
            Request::SetProjectFilter { session, filter } => self.set_project_filter(session, filter).await,
            Request::ExcludePath {
                session,
                path,
                directory,
            } => self.exclude_path(session, &path, directory).await,
            Request::ListBaseCandidates { session } => self.list_base_candidates(session).await,
            Request::ListDirectory { session, path } => self.list_directory(session, &path).await,
            Request::FileText { session, path } => self.file_text(session, &path).await,
            Request::TargetLine {
                session,
                revision,
                side,
                path,
                line,
            } => self.target_line(session, revision, side, &path, line).await,
            Request::Threads { session } => self.threads(session).await,
            Request::CommentTarget {
                session,
                revision,
                path,
                at,
            } => self.comment_target(session, revision, &path, at).await,
            Request::CreateThread {
                session,
                revision,
                target,
                body,
            } => self.create_thread(session, from, revision, target, body).await,
            Request::Reply { session, thread, body } => self.reply(session, from, thread, body).await,
            Request::SetThreadStatus {
                session,
                thread,
                status,
            } => self.set_thread_status(session, from, thread, status).await,
            Request::DeleteThread { session, thread } => self.delete_thread(session, from, thread).await,
            Request::ConfirmAnchor {
                session,
                revision,
                thread,
            } => self.confirm_anchor(session, from, revision, thread).await,
            Request::AttachAnchor {
                session,
                revision,
                thread,
                candidate,
            } => self.attach_anchor(session, from, revision, thread, candidate).await,
            Request::Hover {
                session,
                revision,
                side,
                path,
                line,
                col,
            } => self.find(session).await?.hover(revision, side, &path, line, col).await,
            Request::Definition {
                session,
                revision,
                side,
                path,
                line,
                col,
            } => {
                self.find(session)
                    .await?
                    .definition(revision, side, &path, line, col)
                    .await
            }
            Request::References {
                session,
                revision,
                side,
                path,
                line,
                col,
            } => {
                self.find(session)
                    .await?
                    .references(revision, side, &path, line, col)
                    .await
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
            Request::ListServers { session } => self.list_servers(session).await,
            Request::InstallServer { session, language } => self.install_server(session, from, language).await,
            Request::SetServerPath {
                session,
                language,
                path,
            } => self.set_server_path(session, from, language, path).await,
        }
    }
}

/// Language servers take long to start; a new revision's first query should
/// find them running. Starting never delays the reply of the request that
/// changed the revision.
fn start_servers_in_background(open: Arc<Open>) {
    tokio::spawn(async move { open.start_servers().await });
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
