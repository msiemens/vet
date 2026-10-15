//! The composition root: serves protocol v1 from the real components. Session
//! logic lives in `vet-app`; this crate wires adapters to it and maps its views
//! to protocol messages.

// Tests build their repositories with git.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

mod accounts;
mod analysis;
mod announce;
mod caches;
mod changesets;
mod checks;
#[cfg(test)]
mod contract;
mod error;
mod events;
mod fetch;
mod highlight;
mod hosting;
mod mutex;
mod open;
mod problems;
mod projects;
mod references;
mod review;
mod screens;
mod semantic;
mod servers;
mod symbols;
mod sync;
mod threads;
mod trust;

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use vet_app::gc::Retention;
use vet_app::platform::Accounts;
use vet_app::ports::{PreferenceStore, RecentProject, Settings, SettingsStore, StoreError};
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
use vet_infra::store::SqliteStore;
use vet_protocol::{BackendSettings, Event, Failure, Preference, ProjectId, Request, Response};
use vet_semantic::ports::Backend;
use vet_transport::{Events, Handler};

use caches::PersistentAnswers;
use error::{Error, Result};
use events::Connection;
use hosting::Platforms;
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
    /// How long to wait between two pull requests whose details are fetched ahead of the queue,
    /// so a platform that Vet only reacts to (rather than tracks the budget of) is asked gently.
    pub prefetch_pace: Duration,
    /// How long the backend waits before asking the platform about the checks of the commits on
    /// view again, while some of them are still running.
    pub checks_pace: Duration,
    /// How long after an action on a run the checks are read once more: the platform shows what it
    /// made of the action a moment later, so one read may still see the old state.
    pub checks_settle: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(1500),
            quiet: Duration::from_secs(3),
            poll: Duration::from_secs(2),
            orphan_grace: Duration::from_secs(15),
            fetch_interval: vet_app::fetch::DEFAULT_INTERVAL,
            prefetch_pace: Duration::from_millis(400),
            checks_pace: Duration::from_secs(30),
            checks_settle: Duration::from_secs(3),
        }
    }
}

type Live = LiveSession<GixRepo, SqliteStore>;
type LanguageServers = Servers<ToolResolver, SqliteStore>;
type Semantic =
    Semantics<GixRepo, SqliteStore, LspLauncher<Arc<LanguageServers>, ToolRestorer>, Materialize<GixRepo, SqliteStore>>;

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
    /// The sources of the files and highlights a row list reads, over `highlighter`.
    sources: highlight::Sources,
    /// What blocking work and probe processes draw on, shared by every project.
    budgets: Budgets,
    /// The materialized snapshots kept between launches (§8).
    trees: Arc<KeptTrees>,
    /// The answers of the language servers kept between launches (§8).
    answers: Arc<PersistentAnswers>,
    /// Held while the backend's own settings change, so that what a change applies is the
    /// difference to the settings it replaces.
    settings_changing: tokio::sync::Mutex<()>,
    /// Credentials for hosting platforms: per host and app-wide, not per project.
    accounts: Arc<PlatformAccounts>,
    /// The clients for GitHub and Azure DevOps, with what they have kept, and how they are doing,
    /// for every project.
    platforms: Arc<Platforms>,
    /// The connections that asked for `Accounts` or `RecentProjects`, which is where the events
    /// that belong to the process go.
    process_subscribers: Subscribers<Connection>,
    /// For tests: what the symbols analysis of a file fails with before it runs for real, in order,
    /// so the retry can be exercised. A debug build only, and empty except in the tests' own builds.
    #[cfg(debug_assertions)]
    analysis_failures: Arc<analysis::Failures>,
    /// For tests: how often each project's background tasks were started and how many run now, so a
    /// test can wait for a task's end instead of for time to pass. A debug build only.
    #[cfg(debug_assertions)]
    background: Arc<open::Background>,
}

#[cfg(debug_assertions)]
pub use analysis::Injected;

fn recent_projects(list: std::result::Result<Vec<RecentProject>, StoreError>) -> Result<Response> {
    let projects = list?;
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
        let resolver = ToolResolver::new(store.dir().to_owned())
            .probing(budgets.probes.clone())
            .reading(budgets.semantic_io.clone());
        let referenced = Arc::new(StoreReferences(store.clone()));
        let trees = KeptTrees::new(store.dir().join("materialized"), Limits::default(), referenced)
            .budgeted(budgets.semantic_io.clone());
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
            platforms: Arc::new(Platforms::new(
                Arc::new(GitHubClient::new(accounts.clone())),
                Arc::new(AzureDevOpsClient::new(accounts.clone())),
                process_subscribers.clone(),
            )),
            accounts,
            process_subscribers,
            servers: Arc::new(Servers::new(resolver, store.clone(), languages)),
            backends,
            highlighter: highlight::Highlighter::new(budgets.review_io.clone()),
            sources: highlight::Sources::new(highlight::Highlighter::new(budgets.review_io.clone())),
            budgets,
            trees: Arc::new(trees),
            answers: Arc::new(PersistentAnswers::new(store.clone())),
            settings_changing: tokio::sync::Mutex::new(()),
            recent: RecentProjects::new(store.clone()),
            store,
            config,
            projects: Projects::default(),
            #[cfg(debug_assertions)]
            analysis_failures: Arc::default(),
            #[cfg(debug_assertions)]
            background: Arc::default(),
        }
    }

    /// The budgets every project's blocking work and probes draw on.
    pub fn budgets(&self) -> &Budgets {
        &self.budgets
    }

    /// The same orchestrator, its platforms talking to the API at `base` instead of the hosts',
    /// for the tests' fake servers. Only a debug build has it, so a release build cannot be
    /// pointed at another endpoint.
    #[cfg(debug_assertions)]
    pub fn with_platform_endpoints(mut self, github: Option<&str>, azure_devops: Option<&str>) -> Self {
        self.platforms = Arc::new(Platforms::new(
            Arc::new(GitHubClient::with_endpoints(self.accounts.clone(), github)),
            Arc::new(AzureDevOpsClient::with_endpoints(self.accounts.clone(), azure_devops)),
            self.process_subscribers.clone(),
        ));
        self
    }

    /// The same orchestrator, the symbols analysis of the file at `path` failing as `failures` says
    /// before it runs for real, one failure per try in order: for the tests of the retry. Only a
    /// debug build has it.
    #[cfg(debug_assertions)]
    pub fn with_analysis_failing(self, path: &str, failures: Vec<Injected>) -> Self {
        crate::mutex::lock(&self.analysis_failures).insert(path.to_owned(), failures.into());
        self
    }

    /// For tests: how often the project's background task of `kind` was started and how many are
    /// running now, so a test can wait for a task's end instead of for time to pass. A debug build
    /// only.
    #[cfg(debug_assertions)]
    pub fn background_counts(&self, project: ProjectId, kind: &'static str) -> (usize, usize) {
        self.background.counts(project, kind)
    }

    /// The open project `id`. The request being answered is about it from here on, and its
    /// log lines say so.
    async fn find(&self, id: ProjectId) -> Result<Arc<Open>> {
        tracing::Span::current().record("project", id);
        Ok(self.projects.find(id).await?)
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

    /// The recent list after a change, which every connection subscribed to the process hears of.
    fn recent_changed(&self, list: std::result::Result<Vec<RecentProject>, StoreError>) -> Result<Response> {
        let answer = recent_projects(list)?;
        self.process_subscribers.broadcast(Event::RecentProjectsChanged);
        Ok(answer)
    }

    async fn semantic_cache_usage(&self) -> Response {
        Response::SemanticCacheUsage {
            bytes: self.answers.usage(&self.budgets.semantic_io).await,
        }
    }

    /// The settings, and this connection subscribed to the events of the backend.
    async fn settings(&self, from: &Connection) -> Result<Response> {
        self.process_subscribers.subscribe(from.clone());
        let settings = self.store.backend_settings().await?;
        Ok(Response::Settings {
            settings: BackendSettings {
                semantic_cache_mb: settings.semantic_cache_mb,
                finished_review_days: settings.finished_review_days,
            },
        })
    }

    /// Keeps the settings, then applies them. How long finished reviews are kept applies
    /// at the next start, which is when the store is collected.
    async fn set_settings(&self, new: BackendSettings) -> Result<Response> {
        if new.semantic_cache_mb == 0 {
            return Err(Error::Invalid("the semantic cache needs at least 1 MB".into()));
        }
        if new.finished_review_days == 0 {
            return Err(Error::Invalid("finished reviews are kept for at least a day".into()));
        }
        let settings = Settings {
            semantic_cache_mb: new.semantic_cache_mb,
            finished_review_days: new.finished_review_days,
        };
        {
            let _changing = self.settings_changing.lock().await;
            let old = self.store.backend_settings().await?;
            self.store
                .set_backend_settings(settings)
                .await
                .map_err(|e| Error::Internal(format!("the settings cannot be kept: {}", e.0)))?;
            if old.semantic_cache_mb != settings.semantic_cache_mb {
                self.answers.set_budget(settings.semantic_cache_bytes()).await;
            }
        }
        self.process_subscribers.broadcast(Event::SettingsChanged);
        Ok(Response::Settings { settings: new })
    }

    /// What the window remembers in `scope`, and this connection subscribed to the events of
    /// the backend.
    async fn preferences(&self, scope: Option<String>, from: &Connection) -> Result<Response> {
        self.process_subscribers.subscribe(from.clone());
        let values = self.store.preferences(scope).await?;
        Ok(Response::Preferences {
            values: values
                .into_iter()
                .map(|(key, value)| Preference { key, value })
                .collect(),
        })
    }

    /// Keeps a value of the window, or forgets it, and tells every connection.
    async fn set_preference(&self, scope: Option<String>, key: String, value: Option<String>) -> Result<Response> {
        self.store
            .set_preference(scope.clone(), key.clone(), value.clone())
            .await?;
        self.process_subscribers
            .broadcast(Event::PreferenceChanged { scope, key, value });
        Ok(Response::Done)
    }
}

impl Handler for Orchestrator {
    async fn handle(&self, request: Request, events: Events) -> std::result::Result<Response, Failure> {
        Ok(self.answer(request, &Connection(events)).await?)
    }
}

/// What a request is answered with, on the heap.
type Reply<'a> = Pin<Box<dyn Future<Output = Result<Response>> + Send + 'a>>;

fn reply<'a>(work: impl Future<Output = Result<Response>> + Send + 'a) -> Reply<'a> {
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
            Request::RecentProjects => reply(async move {
                self.process_subscribers.subscribe(from.clone());
                recent_projects(self.recent.list().await)
            }),
            Request::RecordRecentProject { path } => {
                reply(async move { self.recent_changed(self.recent.record(path, unix_now()).await) })
            }
            Request::ForgetRecentProject { path } => {
                reply(async move { self.recent_changed(self.recent.forget(&path).await) })
            }
            Request::Accounts => reply(self.accounts(from)),
            Request::SetCredentials { host, token, validate } => {
                reply(async move { self.set_credentials(&host, &token, validate).await })
            }
            Request::RemoveCredentials { host } => reply(async move { self.remove_credentials(&host).await }),
            Request::ClearSemanticCaches => reply(async move {
                self.clear_semantic_caches().await;
                Ok(self.semantic_cache_usage().await)
            }),
            Request::SemanticCacheUsage => reply(async move { Ok(self.semantic_cache_usage().await) }),
            Request::Settings => reply(self.settings(from)),
            Request::SetSettings { settings } => reply(self.set_settings(settings)),
            Request::Preferences { scope } => reply(self.preferences(scope, from)),
            Request::SetPreference { scope, key, value } => reply(self.set_preference(scope, key, value)),
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
            Request::PipelineSteps { project, check } => reply(self.pipeline_steps(project, check)),
            Request::ChangePipeline { project, action } => reply(self.change_pipeline(project, action)),
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
            } => reply(async move { self.toggle_hunk(project, view, revision, &path, hunk).await }),
            Request::ToggleFlag {
                project,
                view,
                revision,
                path,
                hunk,
            } => reply(async move { self.toggle_flag(project, view, revision, &path, hunk).await }),
            Request::ToggleItem {
                project,
                view,
                revision,
                path,
                kind,
            } => reply(async move { self.toggle_item(project, view, revision, &path, kind).await }),
            Request::ToggleFileSeen {
                project,
                view,
                revision,
                path,
            } => reply(async move { self.toggle_file_seen(project, view, revision, &path).await }),
            Request::SetFilesSeen {
                project,
                view,
                revision,
                paths,
                seen,
            } => reply(async move { self.set_files_seen(project, view, revision, &paths, seen).await }),
            Request::ReconfirmFile {
                project,
                revision,
                path,
            } => reply(async move { self.reconfirm_file(project, revision, &path).await }),
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
            Request::SetProjectFilter { project, filter } => reply(self.set_project_filter(project, filter)),
            Request::NarrowReview {
                project,
                changeset,
                filter,
            } => reply(self.narrow_review(project, changeset, filter)),
            Request::SetProjectTrusted { project, trusted } => reply(self.set_project_trusted(project, trusted)),
            Request::TrustPullRequest { project } => reply(self.trust_pull_request(project)),
            Request::ExcludePath {
                project,
                path,
                directory,
            } => reply(async move { self.exclude_path(project, &path, directory).await }),
            Request::Fetch { project } => reply(self.fetch(project)),
            Request::ListDirectory { project, path } => reply(async move { self.list_directory(project, &path).await }),
            Request::ListFiles { project } => reply(self.list_files(project)),
            Request::PatternMatches { project, filter } => reply(self.pattern_matches(project, filter)),
            Request::FileText { project, view, path } => {
                reply(async move { self.file_text(project, view, &path).await })
            }
            Request::Search {
                project,
                scope,
                pattern,
                mask,
                side,
                only_changes,
                not_seen,
                limit,
            } => {
                let filters = vet_mapping::core_search_filters(side, only_changes, not_seen);
                reply(async move { self.search(project, scope, pattern, &mask, filters, limit).await })
            }
            Request::TargetLine {
                project,
                view,
                revision,
                side,
                path,
                line,
            } => reply(async move { self.target_line(project, view, revision, side, &path, line).await }),
            Request::MapLines {
                project,
                revision,
                from,
                path,
                lines,
            } => reply(async move { self.map_lines(project, revision, from, &path, &lines).await }),
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
            } => reply(self.create_thread(project, view, revision, target, body)),
            Request::Reply { project, thread, body } => reply(self.reply(project, thread, body)),
            Request::SetThreadStatus {
                project,
                thread,
                status,
            } => reply(self.set_thread_status(project, thread, status)),
            Request::DeleteThread { project, thread } => reply(self.delete_thread(project, thread)),
            Request::EditMessage { project, message, body } => reply(self.edit_message(project, message, body)),
            Request::RemoveMessage { project, message } => reply(self.remove_message(project, message)),
            Request::KeepEdit { project, message, keep } => reply(self.keep_edit(project, message, keep)),
            Request::EditConversationComment { project, comment, body } => {
                reply(async move { self.edit_conversation_comment(project, &comment, body).await })
            }
            Request::DeleteConversationComment { project, comment } => {
                reply(async move { self.delete_conversation_comment(project, &comment).await })
            }
            Request::SyncThreads { project } => reply(self.sync_threads(project)),
            Request::SubmitReview { project, verdict, body } => {
                reply(self.submit_review(project, vet_mapping::verdict_of(verdict), body))
            }
            Request::PostDrafts { project } => reply(self.post_drafts(project)),
            Request::StageVote { project, verdict } => {
                reply(self.stage_vote(project, vet_mapping::verdict_of(verdict)))
            }
            Request::DiscardVote { project } => reply(self.discard_vote(project)),
            Request::PullRequestInfo { project } => reply(self.pull_request_info(project)),
            Request::ConfirmAnchor {
                project,
                revision,
                thread,
            } => reply(self.confirm_anchor(project, revision, thread)),
            Request::AttachAnchor {
                project,
                revision,
                thread,
                candidate,
            } => reply(self.attach_anchor(project, revision, thread, candidate)),
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
                limit,
            } => reply(async move {
                self.ready(project)
                    .await?
                    .references(&self.sources, revision, side, (&path, line, col), limit)
                    .await
            }),
            Request::Implementations {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => reply(async move {
                self.ready(project)
                    .await?
                    .implementations(&self.sources, revision, side, &path, line, col)
                    .await
            }),
            Request::Diagnostics {
                project,
                revision,
                path,
                against_base,
            } => reply(async move {
                self.ready(project)
                    .await?
                    .diagnostics(revision, &path, against_base)
                    .await
            }),
            Request::FileSymbols {
                project,
                revision,
                path,
            } => reply(async move { self.ready(project).await?.file_symbols(revision, &path).await }),
            Request::FileOutline {
                project,
                view,
                revision,
                path,
            } => reply(async move { self.ready(project).await?.file_outline(view, revision, &path).await }),
            Request::WorkspaceSymbols {
                project,
                revision,
                query,
            } => reply(async move { self.ready(project).await?.workspace_symbols(revision, &query).await }),
            Request::SymbolAt {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => reply(async move {
                self.ready(project)
                    .await?
                    .symbol_at(revision, side, &path, line, col)
                    .await
            }),
            Request::Signature {
                project,
                revision,
                symbol,
            } => reply(async move { self.ready(project).await?.signature(revision, &symbol).await }),
            Request::Calls {
                project,
                revision,
                symbol,
            } => reply(async move { self.ready(project).await?.calls(revision, &symbol).await }),
            Request::Usages {
                project,
                revision,
                symbol,
            } => reply(async move { self.ready(project).await?.usages(revision, &symbol).await }),
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
            Request::ServedBy { project, paths } => reply(self.served_by(project, paths)),
            Request::InstallServer { project, language } => reply(self.install_server(project, language)),
            Request::SetServerPath {
                project,
                language,
                path,
            } => reply(self.set_server_path(project, language, path)),
            Request::RestartServer { project, language } => reply(self.restart_server(project, language)),
            Request::SetReviewShown { project, shown } => reply(self.set_review_shown(project, shown, from)),
            Request::SetOpenFiles { project, paths } => reply(self.set_open_files(project, paths)),
            Request::RefreshProblems { project } => reply(self.refresh_problems(project)),
        }
    }
}

/// Language servers take long to start; a new revision's first query should
/// find them running. Starting never delays the reply of the request that
/// changed the revision. Servers are for the review screen: while no connection
/// shows it (`SetReviewShown`), nothing starts, and coming up starts them. The
/// checks of the changeset on view are for both screens — the queue shows the
/// selected pull request's checks too — so they start whatever screen shows it.
fn start_servers_in_background(open: Arc<Open>) {
    checks::refresh_checks_in_background(open.clone());
    if !open.screens.shown() {
        return;
    }
    symbols::refresh_symbols_in_background(open.clone());
    problems::refresh_problems_in_background(open.clone());
    let starting = open.clone();
    let task = async move {
        if let Ok(booted) = starting.booted().await {
            booted.start_servers().await;
        }
    };
    #[cfg(debug_assertions)]
    let task = crate::open::tracked(&open, "servers", task);
    tokio::spawn(task);
}

/// Scans a project that has not scanned yet, so opening does not wait for the scan.
fn scan_in_background(open: Arc<Open>) {
    tokio::spawn(async move {
        let _ = open.booted().await;
    });
}

/// Opens the store and collects what its retention, as the settings in the store have it, no
/// longer keeps. A failed collection only leaves the garbage for the next start.
pub async fn open_store(dir: &Path) -> std::result::Result<SqliteStore, String> {
    let store = SqliteStore::open(dir).await.map_err(|e| e.0)?;
    let retention = Retention {
        closed_changesets: store
            .backend_settings()
            .await
            .map_err(|e| e.0)?
            .finished_review_retention(),
        ..Retention::default()
    };
    if let Err(e) = vet_app::gc::collect(&store, unix_now(), &retention).await {
        tracing::warn!(error = %e.0, "garbage collection of the store failed");
    }
    Ok(store)
}
