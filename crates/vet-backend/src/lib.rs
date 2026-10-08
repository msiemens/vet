//! The composition root: serves protocol v1 from the real components. Session
//! logic lives in `vet-app`; this crate wires adapters to it and maps its views
//! to protocol messages.

// Tests build their repositories with git.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

mod accounts;
mod announce;
mod caches;
mod changesets;
mod events;
mod fetch;
mod highlight;
mod hosting;
mod open;
mod projects;
mod references;
mod review;
mod semantic;
mod servers;
mod sync;
mod threads;

use std::path::Path;
use std::pin::Pin;
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
use vet_infra::budget::Budgets;
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
use vet_infra::settings::Settings;
use vet_infra::store::SqliteStore;
use vet_protocol::{BackendSettings, Event, ProjectId, Request, Response};
use vet_semantic::ports::Backend;
use vet_transport::{Events, Handler};

use caches::PersistentAnswers;
use events::Connection;
use open::Wired;
use references::StoreReferences;

pub struct Config {
    /// How long the tree must be quiet before a snapshot settles (§6.2).
    pub debounce: Duration,
    /// How long a settled tree must stay untouched before its content is copied into the store
    /// ahead of a review, so that the copy does not compete with an agent still at work.
    pub quiet: Duration,
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
            quiet: Duration::from_secs(3),
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
    /// What blocking work and probe processes draw on, shared by every project.
    budgets: Budgets,
    /// The materialized snapshots kept between launches (§8).
    trees: Arc<KeptTrees>,
    /// The answers of the language servers kept between launches (§8).
    answers: Arc<PersistentAnswers>,
    /// The backend's own settings, as kept in the data directory.
    settings: tokio::sync::Mutex<Settings>,
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
        let budgets = Budgets::default();
        let store = store.budgeted(budgets.review_io.clone());
        let backends: Arc<[Backend]> = lsp_config::defaults().into();
        let languages = backends.iter().map(|b| b.config().backend.clone()).collect();
        let resolver = ToolResolver::new(store.dir().to_owned()).probing(budgets.probes.clone());
        let referenced = Arc::new(StoreReferences(store.clone()));
        let trees = KeptTrees::new(store.dir().join("materialized"), Limits::default(), referenced)
            .budgeted(budgets.semantic_io.clone());
        let settings = vet_infra::settings::load(store.dir());
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
            highlighter: highlight::Highlighter::new(budgets.review_io.clone()),
            budgets,
            trees: Arc::new(trees),
            answers: Arc::new(PersistentAnswers::new(
                store.dir().to_owned(),
                settings.semantic_cache_bytes(),
            )),
            settings: tokio::sync::Mutex::new(settings),
            recent: RecentProjects::new(store.clone()),
            store,
            config,
            projects: Projects::default(),
        }
    }

    /// The budgets every project's blocking work and probes draw on.
    pub fn budgets(&self) -> &Budgets {
        &self.budgets
    }

    /// The open project `id`. The request being answered is about it from here on, and its
    /// log lines say so.
    async fn find(&self, id: ProjectId) -> Result<Arc<Open>, String> {
        tracing::Span::current().record("project", id);
        self.projects.find(id).await.map_err(message)
    }

    /// At startup: removes what a launch that died left, then the kept trees
    /// that are unused for too long or over the disk budget.
    pub async fn collect_trees(&self) {
        let swept = self.trees.sweep().await;
        let collected = self.trees.collect(SystemTime::now()).await;
        if swept + collected.removed > 0 {
            tracing::info!(
                swept,
                removed = collected.removed,
                freed_mb = collected.freed >> 20,
                "removed leftovers and kept trees"
            );
        }
    }

    /// "Clear semantic caches": drops the answers of the language servers, in memory and
    /// in the file, and removes every kept tree that no running Vet uses.
    pub async fn clear_semantic_caches(&self) -> Collected {
        self.answers.clear().await;
        for open in self.projects.all().await {
            if let Some(booted) = open.booted_now() {
                booted.semantics.forget_answers();
            }
        }
        self.trees.clear().await
    }

    fn semantic_cache_usage(&self) -> Response {
        Response::SemanticCacheUsage {
            bytes: self.answers.usage(),
        }
    }

    async fn settings(&self) -> Response {
        let settings = *self.settings.lock().await;
        Response::Settings {
            settings: BackendSettings {
                semantic_cache_mb: settings.semantic_cache_mb,
                finished_review_days: settings.finished_review_days,
            },
        }
    }

    /// Keeps the settings, then applies them. How long finished reviews are kept applies
    /// at the next start, which is when the store is collected.
    async fn set_settings(&self, new: BackendSettings) -> Result<Response, String> {
        if new.semantic_cache_mb == 0 {
            return Err("the semantic cache needs at least 1 MB".into());
        }
        if new.finished_review_days == 0 {
            return Err("finished reviews are kept for at least a day".into());
        }
        let settings = Settings {
            semantic_cache_mb: new.semantic_cache_mb,
            finished_review_days: new.finished_review_days,
        };
        let old = {
            let mut current = self.settings.lock().await;
            vet_infra::settings::save(self.store.dir(), &settings)
                .map_err(|e| format!("the settings cannot be kept: {e}"))?;
            std::mem::replace(&mut *current, settings)
        };
        if old.semantic_cache_mb != settings.semantic_cache_mb {
            self.answers.set_budget(settings.semantic_cache_bytes()).await;
        }
        Ok(self.settings().await)
    }
}

impl Handler for Orchestrator {
    async fn handle(&self, request: Request, events: Events) -> Result<Response, String> {
        self.answer(request, &Connection(events)).await
    }
}

/// What a request is answered with, on the heap.
type Reply<'a> = Pin<Box<dyn Future<Output = Result<Response, String>> + Send + 'a>>;

fn reply<'a>(work: impl Future<Output = Result<Response, String>> + Send + 'a) -> Reply<'a> {
    Box::pin(work)
}

impl Orchestrator {
    /// Hands a request to its handler. This is not an `async fn`, and every arm
    /// is a [`Reply`]: the futures of the handlers live on the heap and none is
    /// held by the stack of whoever awaits the answer, so a handler whose future is
    /// large costs the handlers beside it nothing. An arm that is not boxed does not compile.
    fn answer<'a>(&'a self, request: Request, from: &'a Connection) -> Reply<'a> {
        match request {
            Request::Ping { payload } => reply(async move { Ok(Response::Pong { payload }) }),
            Request::RecentProjects => reply(async move { recent_projects(self.recent.list().await) }),
            Request::RecordRecentProject { path } => {
                reply(async move { recent_projects(self.recent.record(path, unix_now()).await) })
            }
            Request::ForgetRecentProject { path } => {
                reply(async move { recent_projects(self.recent.forget(&path).await) })
            }
            Request::Accounts => reply(self.accounts(from)),
            Request::SetCredentials { host, token, validate } => {
                reply(async move { self.set_credentials(&host, &token, validate).await })
            }
            Request::RemoveCredentials { host } => reply(async move { self.remove_credentials(&host).await }),
            Request::ClearSemanticCaches => reply(async move {
                self.clear_semantic_caches().await;
                Ok(self.semantic_cache_usage())
            }),
            Request::SemanticCacheUsage => reply(async move { Ok(self.semantic_cache_usage()) }),
            Request::Settings => reply(async move { Ok(self.settings().await) }),
            Request::SetSettings { settings } => reply(self.set_settings(settings)),
            Request::LogDirectory => reply(async move {
                Ok(Response::LogDirectory {
                    path: self.store.dir().join(vet_log::LOGS).to_string_lossy().into_owned(),
                })
            }),
            Request::OpenProject { repo } => reply(async move {
                let (project, root) = self.open(&repo, from.clone()).await?;
                Ok(Response::Project { project, root })
            }),
            Request::CloseProject { project } => reply(async move {
                self.close(project, from).await?;
                Ok(Response::Done)
            }),
            Request::ListChangesets { project } => reply(self.list_changesets(project)),
            Request::ListPullRequests { project } => reply(self.list_pull_requests(project)),
            Request::ChangesetLines { project, changeset } => reply(self.changeset_lines(project, changeset)),
            Request::Checks { project, commits } => reply(self.checks(project, commits)),
            Request::CheckLog { project, run } => reply(self.check_log(project, run)),
            Request::OpenChangeset { project, changeset } => reply(self.open_changeset(project, changeset)),
            Request::Tasks { project } => reply(self.tasks(project)),
            Request::FinishReview { project } => reply(self.finish_review(project)),
            Request::Versions { project } => reply(self.versions(project)),
            Request::Commits { project } => reply(self.commits(project)),
            Request::SetLens { project, lens } => reply(self.set_lens(project, lens)),
            Request::Manifest { project, view } => reply(self.manifest(project, view)),
            Request::FileContents { project, view, path } => {
                reply(async move { self.file_contents(project, view, &path).await })
            }
            Request::ToggleHunk {
                project,
                view,
                revision,
                path,
                hunk,
            } => reply(async move { self.toggle_hunk(project, from, view, revision, &path, hunk).await }),
            Request::ToggleItem {
                project,
                view,
                revision,
                path,
                kind,
            } => reply(async move { self.toggle_item(project, from, view, revision, &path, kind).await }),
            Request::ToggleFileSeen {
                project,
                view,
                revision,
                path,
            } => reply(async move { self.toggle_file_seen(project, from, view, revision, &path).await }),
            Request::ReconfirmFile {
                project,
                revision,
                path,
            } => reply(async move { self.reconfirm_file(project, from, revision, &path).await }),
            Request::BaseDiff {
                project,
                revision,
                path,
                hunk,
            } => reply(async move { self.base_diff(project, revision, &path, hunk).await }),
            Request::ItemBlob {
                project,
                view,
                revision,
                path,
                kind,
                side,
            } => reply(async move { self.item_blob(project, view, revision, &path, kind, side).await }),
            Request::SetBase { project, base } => reply(self.set_base(project, base)),
            Request::SetProjectFilter { project, filter } => reply(self.set_project_filter(project, filter)),
            Request::ExcludePath {
                project,
                path,
                directory,
            } => reply(async move { self.exclude_path(project, &path, directory).await }),
            Request::ListBaseCandidates { project } => reply(self.list_base_candidates(project)),
            Request::Fetch { project } => reply(self.fetch(project)),
            Request::ListDirectory { project, path } => reply(async move { self.list_directory(project, &path).await }),
            Request::FileText { project, path } => reply(async move { self.file_text(project, &path).await }),
            Request::TargetLine {
                project,
                view,
                revision,
                side,
                path,
                line,
            } => reply(async move { self.target_line(project, view, revision, side, &path, line).await }),
            Request::Threads { project, view } => reply(self.threads(project, view)),
            Request::CommentTarget {
                project,
                view,
                revision,
                path,
                at,
            } => reply(async move { self.comment_target(project, view, revision, &path, at).await }),
            Request::CreateThread {
                project,
                view,
                revision,
                target,
                body,
            } => reply(self.create_thread(project, from, view, revision, target, body)),
            Request::Reply { project, thread, body } => reply(self.reply(project, from, thread, body)),
            Request::SetThreadStatus {
                project,
                thread,
                status,
            } => reply(self.set_thread_status(project, from, thread, status)),
            Request::DeleteThread { project, thread } => reply(self.delete_thread(project, from, thread)),
            Request::EditMessage { project, message, body } => reply(self.edit_message(project, from, message, body)),
            Request::RemoveMessage { project, message } => reply(self.remove_message(project, from, message)),
            Request::KeepEdit { project, message, keep } => reply(self.keep_edit(project, from, message, keep)),
            Request::SyncThreads { project } => reply(self.sync_threads(project)),
            Request::SubmitReview { project, verdict, body } => reply(self.submit_review(project, verdict, body)),
            Request::PostDrafts { project } => reply(self.post_drafts(project)),
            Request::SetVote { project, verdict } => reply(self.set_vote(project, verdict)),
            Request::PullRequestInfo { project } => reply(self.pull_request_info(project)),
            Request::ConfirmAnchor {
                project,
                revision,
                thread,
            } => reply(self.confirm_anchor(project, from, revision, thread)),
            Request::AttachAnchor {
                project,
                revision,
                thread,
                candidate,
            } => reply(self.attach_anchor(project, from, revision, thread, candidate)),
            Request::Hover {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => reply(async move { self.ready(project).await?.hover(revision, side, &path, line, col).await }),
            Request::Definition {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => reply(async move {
                self.ready(project)
                    .await?
                    .definition(revision, side, &path, line, col)
                    .await
            }),
            Request::References {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => reply(async move {
                self.ready(project)
                    .await?
                    .references(revision, side, &path, line, col)
                    .await
            }),
            Request::Diagnostics {
                project,
                revision,
                path,
            } => reply(async move { self.ready(project).await?.diagnostics(revision, &path).await }),
            Request::ChangedSymbols { project, revision } => {
                reply(async move { self.ready(project).await?.changed_symbols(revision).await })
            }
            Request::Signature {
                project,
                revision,
                symbol,
            } => reply(async move { self.ready(project).await?.signature(revision, &symbol).await }),
            Request::Impact {
                project,
                revision,
                symbol,
            } => reply(async move { self.ready(project).await?.impact(revision, &symbol).await }),
            Request::RemovalImpact {
                project,
                revision,
                symbol,
            } => reply(async move { self.ready(project).await?.removal_impact(revision, &symbol).await }),
            Request::Reachability {
                project,
                revision,
                symbol,
            } => reply(async move { self.ready(project).await?.reachability(revision, &symbol).await }),
            Request::ListServers { project } => reply(self.list_servers(project)),
            Request::InstallServer { project, language } => reply(self.install_server(project, from, language)),
            Request::SetServerPath {
                project,
                language,
                path,
            } => reply(self.set_server_path(project, from, language, path)),
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

/// Opens the store and collects what its retention, as the settings in `dir` have it, no
/// longer keeps. A failed collection only leaves the garbage for the next start.
pub async fn open_store(dir: &Path) -> Result<SqliteStore, String> {
    let store = SqliteStore::open(dir).await.map_err(|e| e.0)?;
    let retention = Retention {
        closed_changesets: vet_infra::settings::load(store.dir()).finished_review_retention(),
        ..Retention::default()
    };
    if let Err(e) = vet_app::gc::collect(&store, unix_now(), &retention).await {
        tracing::warn!(error = %e.0, "garbage collection of the store failed");
    }
    Ok(store)
}
