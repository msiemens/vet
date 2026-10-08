//! The composition root: serves protocol v1 from the real components. Session
//! logic lives in `vet-app`; this crate wires adapters to it and maps its views
//! to protocol messages.

mod accounts;
mod changesets;
mod events;
mod fetch;
mod highlight;
mod open;
mod references;
mod review;
mod semantic;
mod servers;
mod threads;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use vet_app::gc::Retention;
use vet_app::limits::Limiting;
use vet_app::platform::Accounts;
use vet_app::ports::{RecentProject, StoreError};
use vet_app::recent::RecentProjects;
use vet_app::registry::{Entry, Registry, Subscribers};
use vet_app::semantics::Semantics;
use vet_app::servers::Servers;
use vet_app::session::LiveSession;
use vet_infra::clock::unix_now;
use vet_infra::credentials::HttpValidator;
use vet_infra::exec::TokioRunner;
use vet_infra::git::GixRepo;
use vet_infra::lsp::{LspLauncher, config as lsp_config};
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::{Collected, KeptTrees, Limits};
use vet_infra::platform::azure_devops::AzureDevOpsClient;
use vet_infra::platform::github::GitHubClient;
use vet_infra::restore::ToolRestorer;
use vet_infra::servers::ToolResolver;
use vet_infra::store::SqliteStore;
use vet_protocol::{Event, ProjectId, Request, Response};
use vet_semantic::ports::Backend;
use vet_transport::{Events, Handler};

use events::Connection;
use open::Wired;
use references::StoreReferences;

pub struct Config {
    /// How long the tree must be quiet before a snapshot settles (§6.2).
    pub debounce: Duration,
    /// Poll interval when the topology has no usable file events (§6.4).
    pub poll: Duration,
    /// How long a project without subscribers stays open, so a reloading
    /// shell finds it again.
    pub orphan_grace: Duration,
    /// How often an open session's remotes are fetched, after the fetch at open.
    pub fetch_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(1500),
            poll: Duration::from_secs(2),
            orphan_grace: Duration::from_secs(15),
            fetch_interval: vet_app::fetch::DEFAULT_INTERVAL,
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

/// An open project: projects are one per repository, and opening an open
/// repository subscribes to the existing one. It may not have scanned yet; see
/// `Orchestrator::ready`.
type Open = Entry<Connection, Wired>;
type Projects = Registry<Connection, Wired>;
type PlatformAccounts = Accounts<TokioRunner, HttpValidator>;
type GitHub = GitHubClient<Arc<PlatformAccounts>>;
type AzureDevOps = AzureDevOpsClient<Arc<PlatformAccounts>>;

pub struct Orchestrator {
    store: SqliteStore,
    recent: RecentProjects<SqliteStore>,
    config: Config,
    projects: Projects,
    servers: Arc<LanguageServers>,
    backends: Arc<[Backend]>,
    highlighter: highlight::Highlighter,
    /// The materialized snapshots kept between launches (§8).
    trees: Arc<KeptTrees>,
    /// Credentials for hosting platforms: per host and app-wide, not per project.
    accounts: Arc<PlatformAccounts>,
    /// The client for GitHub, with the lists it has kept, for every project.
    github: Arc<GitHub>,
    /// The client for Azure DevOps, likewise.
    azure_devops: Arc<AzureDevOps>,
    /// The connections that asked for `Accounts`, which is where the events that belong
    /// to the process go.
    process_subscribers: Subscribers<Connection>,
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn recent_projects(list: Result<Vec<RecentProject>, StoreError>) -> Result<Response, String> {
    let projects = list.map_err(|e| e.0)?;
    Ok(Response::RecentProjects {
        projects: projects
            .into_iter()
            .map(|p| vet_protocol::RecentProject {
                path: p.path,
                opened_at: u32::try_from(p.opened_at).unwrap_or(0),
            })
            .collect(),
    })
}

impl Orchestrator {
    pub fn new(store: SqliteStore, config: Config) -> Self {
        let backends: Arc<[Backend]> = lsp_config::defaults().into();
        let languages = backends.iter().map(|b| b.config().backend.clone()).collect();
        let resolver = ToolResolver::new(store.dir().to_owned());
        let referenced = Arc::new(StoreReferences(store.clone()));
        let trees = KeptTrees::new(store.dir().join("materialized"), Limits::default(), referenced);
        let process_subscribers = Subscribers::<Connection>::default();
        let needed = process_subscribers.clone();
        let accounts = Arc::new(Accounts::new(
            TokioRunner,
            HttpValidator::default(),
            std::env::temp_dir(),
            move |host| {
                needed.broadcast(Event::CredentialsNeeded { host: host.to_owned() });
            },
        ));
        Self {
            github: Arc::new(GitHubClient::new(accounts.clone())),
            azure_devops: Arc::new(AzureDevOpsClient::new(accounts.clone())),
            accounts,
            process_subscribers,
            servers: Arc::new(Servers::new(resolver, store.clone(), languages)),
            backends,
            highlighter: Default::default(),
            trees: Arc::new(trees),
            recent: RecentProjects::new(store.clone()),
            store,
            config,
            projects: Projects::default(),
        }
    }

    async fn find(&self, id: ProjectId) -> Result<Arc<Open>, String> {
        self.projects.find(id).await.map_err(message)
    }

    /// At startup: removes what a launch that died left, then the kept trees
    /// that are unused for too long or over the disk budget.
    pub async fn collect_trees(&self) {
        let swept = self.trees.sweep().await;
        let collected = self.trees.collect(SystemTime::now()).await;
        if swept + collected.removed > 0 {
            eprintln!(
                "vet: removed {swept} leftovers and {} kept trees ({} MB)",
                collected.removed,
                collected.freed >> 20
            );
        }
    }

    /// "Clear semantic caches": removes every kept tree that no running Vet uses.
    pub async fn clear_kept_trees(&self) -> Collected {
        self.trees.clear().await
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
            Request::RecentProjects => recent_projects(self.recent.list().await),
            Request::RecordRecentProject { path } => recent_projects(self.recent.record(path, unix_now()).await),
            Request::ForgetRecentProject { path } => recent_projects(self.recent.forget(&path).await),
            Request::Accounts => self.accounts(from).await,
            Request::SetCredentials { host, token, validate } => self.set_credentials(&host, &token, validate).await,
            Request::RemoveCredentials { host } => self.remove_credentials(&host).await,
            Request::OpenProject { repo } => {
                let (project, root) = self.open(&repo, from.clone()).await?;
                Ok(Response::Project { project, root })
            }
            Request::CloseProject { project } => {
                self.close(project, from).await?;
                Ok(Response::Done)
            }
            Request::ListChangesets { project } => self.list_changesets(project).await,
            Request::ListPullRequests { project } => self.list_pull_requests(project).await,
            Request::Checks { project, commits } => self.checks(project, commits).await,
            Request::CheckLog { project, run } => self.check_log(project, run).await,
            Request::OpenChangeset { project, changeset } => self.open_changeset(project, changeset).await,
            Request::Tasks { project } => self.tasks(project).await,
            Request::FinishReview { project } => self.finish_review(project).await,
            Request::Versions { project } => self.versions(project).await,
            Request::Commits { project } => self.commits(project).await,
            Request::SetLens { project, lens } => self.set_lens(project, lens).await,
            Request::Manifest { project, view } => self.manifest(project, view).await,
            Request::FileContents { project, view, path } => self.file_contents(project, view, &path).await,
            Request::ToggleHunk {
                project,
                view,
                revision,
                path,
                hunk,
            } => self.toggle_hunk(project, from, view, revision, &path, hunk).await,
            Request::ToggleItem {
                project,
                view,
                revision,
                path,
                kind,
            } => self.toggle_item(project, from, view, revision, &path, kind).await,
            Request::ToggleFileSeen {
                project,
                view,
                revision,
                path,
            } => self.toggle_file_seen(project, from, view, revision, &path).await,
            Request::ReconfirmFile {
                project,
                revision,
                path,
            } => self.reconfirm_file(project, from, revision, &path).await,
            Request::BaseDiff {
                project,
                revision,
                path,
                hunk,
            } => self.base_diff(project, revision, &path, hunk).await,
            Request::ItemBlob {
                project,
                view,
                revision,
                path,
                kind,
                side,
            } => self.item_blob(project, view, revision, &path, kind, side).await,
            Request::SetBase { project, base } => self.set_base(project, base).await,
            Request::SetProjectFilter { project, filter } => self.set_project_filter(project, filter).await,
            Request::ExcludePath {
                project,
                path,
                directory,
            } => self.exclude_path(project, &path, directory).await,
            Request::ListBaseCandidates { project } => self.list_base_candidates(project).await,
            Request::Fetch { project } => self.fetch(project).await,
            Request::ListDirectory { project, path } => self.list_directory(project, &path).await,
            Request::FileText { project, path } => self.file_text(project, &path).await,
            Request::TargetLine {
                project,
                view,
                revision,
                side,
                path,
                line,
            } => self.target_line(project, view, revision, side, &path, line).await,
            Request::Threads { project, view } => self.threads(project, view).await,
            Request::CommentTarget {
                project,
                view,
                revision,
                path,
                at,
            } => self.comment_target(project, view, revision, &path, at).await,
            Request::CreateThread {
                project,
                view,
                revision,
                target,
                body,
            } => self.create_thread(project, from, view, revision, target, body).await,
            Request::Reply { project, thread, body } => self.reply(project, from, thread, body).await,
            Request::SetThreadStatus {
                project,
                thread,
                status,
            } => self.set_thread_status(project, from, thread, status).await,
            Request::DeleteThread { project, thread } => self.delete_thread(project, from, thread).await,
            Request::ConfirmAnchor {
                project,
                revision,
                thread,
            } => self.confirm_anchor(project, from, revision, thread).await,
            Request::AttachAnchor {
                project,
                revision,
                thread,
                candidate,
            } => self.attach_anchor(project, from, revision, thread, candidate).await,
            Request::Hover {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => self.ready(project).await?.hover(revision, side, &path, line, col).await,
            Request::Definition {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => {
                self.ready(project)
                    .await?
                    .definition(revision, side, &path, line, col)
                    .await
            }
            Request::References {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => {
                self.ready(project)
                    .await?
                    .references(revision, side, &path, line, col)
                    .await
            }
            Request::Diagnostics {
                project,
                revision,
                path,
            } => self.ready(project).await?.diagnostics(revision, &path).await,
            Request::ChangedSymbols { project, revision } => self.ready(project).await?.changed_symbols(revision).await,
            Request::Signature {
                project,
                revision,
                symbol,
            } => self.ready(project).await?.signature(revision, &symbol).await,
            Request::Impact {
                project,
                revision,
                symbol,
            } => self.ready(project).await?.impact(revision, &symbol).await,
            Request::RemovalImpact {
                project,
                revision,
                symbol,
            } => self.ready(project).await?.removal_impact(revision, &symbol).await,
            Request::Reachability {
                project,
                revision,
                symbol,
            } => self.ready(project).await?.reachability(revision, &symbol).await,
            Request::ListServers { project } => self.list_servers(project).await,
            Request::InstallServer { project, language } => self.install_server(project, from, language).await,
            Request::SetServerPath {
                project,
                language,
                path,
            } => self.set_server_path(project, from, language, path).await,
        }
    }
}

/// Language servers take long to start; a new revision's first query should
/// find them running. Starting never delays the reply of the request that
/// changed the revision. For a project that has not scanned yet this is what
/// scans it, so opening does not wait for the scan.
fn start_servers_in_background(open: Arc<Open>) {
    tokio::spawn(async move {
        if let Ok(booted) = open.booted().await {
            booted.start_servers().await;
        }
    });
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
