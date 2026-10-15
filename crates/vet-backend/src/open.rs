//! Opening and closing projects: assembling the adapters one project runs on.
//!
//! Opening answers as soon as the repository is found. What needs the first
//! scan (the live session and the semantic layer built on it) is assembled by
//! the first request that needs it, or by the task `open` spawns, whichever is
//! first; the others wait for it. Meanwhile the scan reports as a task.

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime};

use std::collections::HashMap;

use tokio::sync::OnceCell;
use tokio::task::AbortHandle;
use vet_app::changeset::{RemoteKey, Source};
use vet_app::fetch::Fetch;
use vet_app::ignore::IgnoringWatcher;
use vet_app::platform::PlatformKind;
use vet_app::progress::Tasks;
use vet_app::registry::{Resident, Subscribers};
use vet_app::semantics::{Links, Semantics};
use vet_app::session::RevisionView;
use vet_app::settle::{SettleFlush, Settling};
use vet_core::content::RepoPath;
use vet_infra::budget::{Budget, Budgets, FETCH_IO};
use vet_infra::clock::{TokioClock, TokioDetach, unix_now};
use vet_infra::git::GixRepo;
use vet_infra::language;
use vet_infra::lsp::LspLauncher;
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::KeptTrees;
use vet_infra::restore::ToolRestorer;
use vet_infra::store::SqliteStore;
use vet_infra::watch::FsWatcher;
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};
use vet_semantic::pool::Supervision;
use vet_semantic::ports::Backend;

use crate::caches::PersistentAnswers;
use crate::error::{Error, Result};
use crate::events::{Connection, edits_pending, settled};
use crate::hosting::{Announcer, Hosting, Platforms};
use crate::mutex::lock;
use crate::screens::ReviewScreens;
use crate::{LanguageServers, Live, Open, Orchestrator, PlatformAccounts, Semantic, start_servers_in_background};

/// The least time between two progress announcements of one task, so a step that
/// reports per file reaches the UI a few times a second.
const TASK_UPDATES: Duration = Duration::from_millis(250);

/// What a project runs on once its first scan is done.
pub(crate) struct Booted {
    pub session: Arc<Live>,
    /// What `semantics` reads of `session`: the review state as published, and nothing that
    /// changes it.
    pub view: RevisionView<GixRepo, SqliteStore>,
    pub semantics: Semantic,
}

/// What `Wired::booted` assembles a project from.
struct Plan {
    id: ProjectId,
    repo: PathBuf,
    root: String,
    store: SqliteStore,
    budgets: Budgets,
    debounce: Duration,
    quiet: Duration,
    poll: Duration,
    servers: Arc<LanguageServers>,
    backends: Arc<[Backend]>,
    subscribers: Subscribers<Connection>,
    /// The entry this is part of, for the servers a settled snapshot starts.
    entry: Arc<OnceLock<Weak<Open>>>,
}

/// The platform of `remote`: the host Vet knows by name, or one the viewer has an account for (a
/// GitHub Enterprise installation). A host that is neither is an error fit to show.
pub(crate) async fn kind_of(remote: &RemoteKey, accounts: &PlatformAccounts) -> Result<PlatformKind> {
    match remote.kind() {
        Some(kind) => Ok(kind),
        None if accounts.has_account(&remote.host, unix_now()).await => Ok(PlatformKind::GitHub),
        None => Err(Error::Invalid(format!(
            "{} is not a hosting platform Vet knows, and the viewer has no account for it",
            remote.host
        ))),
    }
}

/// The latest background task of one kind that a project runs: a newer one replaces the older and
/// stops it.
#[derive(Default)]
struct TaskSlot(Mutex<Option<AbortHandle>>);

impl TaskSlot {
    fn keep(&self, task: AbortHandle) {
        if let Some(older) = lock(&self.0).replace(task) {
            older.abort();
        }
    }

    fn stop(&self) {
        if let Some(task) = lock(&self.0).take() {
            task.abort();
        }
    }
}

type Counts = HashMap<(ProjectId, &'static str), (usize, usize)>;

/// How often each background task of each project was started, and how many of them are running
/// now. The live count is what `Request::WaitForQuiet` waits on; the started count is for the
/// tests that watch one kind end.
#[derive(Default)]
pub(crate) struct Background(Mutex<Counts>);

impl Background {
    /// Counts a task of `kind` that is starting; dropping the guard says it ended, whether it
    /// returned or was aborted.
    pub(crate) fn start(self: &Arc<Self>, project: ProjectId, kind: &'static str) -> LiveTask {
        {
            let mut all = lock(&self.0);
            let counts = all.entry((project, kind)).or_default();
            counts.0 += 1;
            counts.1 += 1;
        }
        LiveTask {
            background: self.clone(),
            project,
            kind,
        }
    }

    /// How many background tasks of the project are running now.
    pub(crate) fn live(&self, project: ProjectId) -> usize {
        lock(&self.0)
            .iter()
            .filter(|((at, _), _)| *at == project)
            .map(|(_, (_, live))| live)
            .sum()
    }

    /// How often the task was started and how many are running now. For tests only.
    #[cfg(debug_assertions)]
    pub(crate) fn counts(&self, project: ProjectId, kind: &'static str) -> (usize, usize) {
        lock(&self.0).get(&(project, kind)).copied().unwrap_or_default()
    }
}

/// The end of one counted background task.
pub(crate) struct LiveTask {
    background: Arc<Background>,
    project: ProjectId,
    kind: &'static str,
}

impl Drop for LiveTask {
    fn drop(&mut self) {
        if let Some(counts) = lock(&self.background.0).get_mut(&(self.project, self.kind)) {
            counts.1 = counts.1.saturating_sub(1);
        }
    }
}

/// `fut` as a background task of `kind` that `Request::WaitForQuiet`, and the tests, can see end.
pub(crate) fn tracked<F: std::future::Future>(
    open: &Arc<Open>,
    kind: &'static str,
    fut: F,
) -> impl std::future::Future<Output = F::Output> + use<F> {
    let live = open.background.start(open.id, kind);
    async move {
        let _live = live;
        fut.await
    }
}

/// What one open project runs on.
pub(crate) struct Wired {
    /// The background work of the project, announced to its subscribers.
    pub tasks: Tasks,
    pub fetch: Fetch<GixRepo>,
    /// Where `fetch` finds the credentials of the hosting platforms.
    pub accounts: Arc<PlatformAccounts>,
    /// Where pull requests are asked for.
    pub platforms: Arc<Platforms>,
    /// Keeps this project among those that hear how the platforms are doing.
    _announcer: Arc<Announcer>,
    /// The connections that show the review screen, which the language servers run for.
    pub screens: ReviewScreens,
    plan: Plan,
    booted: OnceCell<Result<Arc<Booted>>>,
    settling: TaskSlot,
    /// Whether edits of the working tree wait to settle, as the subscribers were last told.
    edits_waiting: Arc<AtomicBool>,
    fetching: TaskSlot,
    /// The copy of the working tree's content ahead of a review; a newer tree's replaces it.
    copying: TaskSlot,
    /// The details of the queue's pull requests fetched ahead; a newer list's replaces it.
    prefetching: TaskSlot,
    /// The checks of the changeset on view, refreshed while the review screen shows it; a newer
    /// run replaces the older, and the project closing stops it.
    checking: TaskSlot,
    /// How long the checks task waits between two rounds while some checks still run.
    checks_pace: Duration,
    /// How long after an action on a run the checks are read once more.
    checks_settle: Duration,
    /// The changed symbols of the changeset on view, analysed in the background while the review
    /// screen shows it; a newer run replaces the older, and the project closing stops it.
    symbols: TaskSlot,
    /// The diagnostics of the changeset on view, analysed in the background while the review screen
    /// shows it; a newer run replaces the older, and the project closing stops it.
    problems: TaskSlot,
    /// The changed files the viewer has open, which the background analysis reads first.
    open_files: Arc<Mutex<Vec<String>>>,
    closed: AtomicBool,
    trees: Arc<KeptTrees>,
    /// The answers of the language servers kept between launches.
    answers: Arc<PersistentAnswers>,
    /// For tests: what the symbols analysis of a file fails with before it runs for real, in order.
    /// A debug build only.
    #[cfg(debug_assertions)]
    pub(crate) analysis_failures: Arc<crate::analysis::Failures>,
    /// How often each background task of this project was started and how many run now.
    pub(crate) background: Arc<Background>,
    /// A client's request to reconcile the working tree now, taken by the settle loop.
    pub(crate) settle_flush: Arc<SettleFlush>,
}

impl Wired {
    /// The session and its semantic layer, once the first scan is done. The
    /// first caller scans; the others wait for it. A scan that failed fails
    /// every caller with the same message.
    pub(crate) async fn booted(&self) -> Result<&Arc<Booted>> {
        self.booted
            .get_or_init(|| self.boot())
            .await
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Whether edits of the working tree wait to settle into a snapshot.
    pub(crate) fn edits_waiting(&self) -> bool {
        self.edits_waiting.load(Ordering::SeqCst)
    }

    /// Whether the project has no background work running or pending: no task of its own, no
    /// progress task of the session, no edit waiting to settle, no settled result the session has
    /// yet to announce, and no reconcile a client asked for and the settle loop has not made.
    pub(crate) fn quiet(&self) -> bool {
        let settling = self.booted_now().is_none_or(|booted| !booted.session.settling_busy());
        !self.tasks.any_running()
            && self.background.live(self.plan.id) == 0
            && !self.edits_waiting()
            && settling
            && !self.settle_flush.requested()
    }

    /// The session if it has scanned successfully already.
    pub(crate) fn booted_now(&self) -> Option<&Arc<Booted>> {
        self.booted.get()?.as_ref().ok()
    }

    /// The client of the platform `kind`.
    pub(crate) fn hosting(&self, kind: PlatformKind) -> Hosting {
        self.platforms.hosting(kind)
    }

    /// The platform of `remote`: the host Vet knows by name, or one the viewer has an account for
    /// (a GitHub Enterprise installation). A host that is neither is an error fit to show.
    pub(crate) async fn kind_of(&self, remote: &RemoteKey) -> Result<PlatformKind> {
        kind_of(remote, &self.accounts).await
    }

    /// The client of the platform `remote` is on.
    pub(crate) async fn hosting_for(&self, remote: &RemoteKey) -> Result<Hosting> {
        Ok(self.hosting(self.kind_of(remote).await?))
    }

    /// Hands over the task that fetches periodically, which ends with the session.
    pub(crate) fn keep_fetching(&self, task: AbortHandle) {
        self.fetching.keep(task);
    }

    /// Hands over the details fetched ahead of the queue; a newer list's replaces it, and it ends
    /// with the session.
    pub(crate) fn keep_prefetching(&self, task: AbortHandle) {
        self.prefetching.keep(task);
    }

    /// Hands over the task that refreshes the changeset's checks; a newer run replaces it, and it
    /// ends with the session.
    pub(crate) fn keep_checking(&self, task: AbortHandle) {
        self.checking.keep(task);
    }

    /// How long the checks task waits between two rounds while some checks still run.
    pub(crate) fn checks_pace(&self) -> Duration {
        self.checks_pace
    }

    /// How long after an action on a run the checks are read once more.
    pub(crate) fn checks_settle(&self) -> Duration {
        self.checks_settle
    }

    /// Hands over the task that analyses the changeset's changed symbols; a newer run replaces it,
    /// and it ends with the session.
    pub(crate) fn keep_symbols(&self, task: AbortHandle) {
        self.symbols.keep(task);
    }

    /// Stops analysing the changeset's changed symbols: the review screen is no longer shown.
    pub(crate) fn stop_symbols(&self) {
        self.symbols.stop();
    }

    /// Hands over the task that analyses the changeset's problems; a newer run replaces it, and it
    /// ends with the session.
    pub(crate) fn keep_problems(&self, task: AbortHandle) {
        self.problems.keep(task);
    }

    /// Stops analysing the changeset's problems: the review screen is no longer shown.
    pub(crate) fn stop_problems(&self) {
        self.problems.stop();
    }

    /// The changed files the viewer has open, which the background analysis reads first.
    pub(crate) fn open_files(&self) -> Vec<String> {
        lock(&self.open_files).clone()
    }

    /// Remembers the changed files the viewer has open.
    pub(crate) fn set_open_files(&self, paths: Vec<String>) {
        *lock(&self.open_files) = paths;
    }

    /// Copies the working tree's content into the store in the background, so the first mark
    /// finds it there.
    fn copy_ahead(&self, session: &Arc<Live>) {
        let session = session.clone();
        let task = tokio::spawn(async move {
            if let Err(e) = session.copy_ahead().await {
                tracing::warn!(error = %e, "could not copy the working tree's content ahead of a review");
            }
        });
        self.copying.keep(task.abort_handle());
    }

    /// Stops the copy of an older tree, unless a mark is waiting for it.
    fn stop_copying(&self) {
        self.copying.stop();
    }

    async fn boot(&self) -> Result<Arc<Booted>> {
        let Plan {
            id,
            repo,
            root,
            store,
            budgets,
            debounce,
            quiet,
            poll,
            servers,
            backends,
            subscribers,
            entry,
        } = &self.plan;
        let id = *id;
        // Watching starts before the initial scan, so no edit falls between them.
        let watcher = FsWatcher::open(repo, *poll).map_err(|e| Error::Internal(e.to_string()))?;
        let git = GixRepo::new(repo, budgets.review_io.clone());
        // The copy ahead of a review is work nobody waits for: it draws on a budget of its own,
        // never on the review's.
        let session = Arc::new(
            Live::open_reporting(git.clone(), store.clone(), root.clone(), unix_now, self.tasks.clone())
                .await?
                .copying_through(
                    git.budgeted(budgets.background_io.clone()),
                    store.budgeted(budgets.background_io.clone()),
                )
                .knowing_pull_requests_in(self.platforms.known()),
        );
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::Unavailable("the session was closed".into()));
        }

        // The content of the tree on view is copied ahead of a review once the tree has been
        // quiet for a while: the one found at open too, as the first manifest comes first.
        let settling = Settling::new(
            IgnoringWatcher::new(watcher, git.clone()),
            TokioClock::new(),
            debounce.as_millis() as u64,
            Some(session.settled_snapshot().await),
        )
        .quiet_after(quiet.as_millis() as u64)
        .reporting_edits()
        .flushable(self.settle_flush.clone());
        let task = {
            let (session, subscribers, entry) = (session.clone(), subscribers.clone(), entry.clone());
            let edit_subscribers = subscribers.clone();
            let waiting = self.edits_waiting.clone();
            let quiet_entry = entry.clone();
            let quiet_session = session.clone();
            tokio::spawn(async move {
                session
                    .follow(
                        settling,
                        |revision| {
                            settled(&subscribers, id, revision);
                            if let Some(open) = entry.get().and_then(Weak::upgrade) {
                                open.stop_copying();
                                start_servers_in_background(open);
                            }
                        },
                        || {
                            if let Some(open) = quiet_entry.get().and_then(Weak::upgrade) {
                                open.copy_ahead(&quiet_session);
                            }
                        },
                        |pending| {
                            waiting.store(pending, Ordering::SeqCst);
                            edits_pending(&edit_subscribers, id, pending);
                        },
                    )
                    .await;
            })
        };
        self.settling.keep(task.abort_handle());

        // The semantic lane reads the same repository and store through handles of its own, so
        // what it reads draws on its budget and never on the review's.
        let reading = Arc::new(session.acquisition().through(
            git.budgeted(budgets.semantic_io.clone()),
            store.budgeted(budgets.semantic_io.clone()),
        ));
        let restorer = ToolRestorer::new(budgets)
            .reusing(repo.clone())
            .reporting(self.tasks.clone());
        let mut semantics = Semantics::new(
            Materialize::new(reading.clone(), self.trees.clone()),
            LspLauncher::new(servers.clone(), restorer, budgets).reporting(self.tasks.clone()),
            repo.clone(),
            backends.to_vec(),
            Links {
                revisions: session.revisions(),
                // The queries of the live tree read its snapshot's texts from the store, so
                // what the worktree holds no longer decides what they read (D97).
                retainer: Some(session.clone()),
            },
        )
        .reading(reading)
        // Which server serves a file of a kind that several projects share is decided by the
        // checkout's own files; a snapshot's layout is taken to be the checkout's.
        .finding({
            let root = repo.clone();
            Arc::new(move |file: &RepoPath| vet_infra::is_file_in(&root, file))
        })
        .supervised(Supervision {
            detach: Arc::new(TokioDetach),
            told: {
                let subscribers = subscribers.clone();
                Arc::new(move |language, outcome| {
                    let name = language::by_id(&language.0).map_or(language.0.as_str(), |l| l.display_name);
                    subscribers.broadcast(Event::Notification {
                        project: id,
                        notification: mapping::semantic::server_died(name, outcome),
                    });
                    subscribers.broadcast(Event::ServersChanged { project: id });
                })
            },
        })
        .observing(
            {
                let subscribers = subscribers.clone();
                Arc::new(move |runtimes| {
                    subscribers.broadcast(Event::ServerRuntimes {
                        project: id,
                        runtimes: mapping::semantic::runtimes(&runtimes),
                    });
                })
            },
            {
                let subscribers = subscribers.clone();
                Arc::new(move |failures| {
                    let failures = mapping::semantic::restore_failures(&failures);
                    subscribers.broadcast(Event::RestoreFailed { project: id, failures });
                })
            },
        );
        if let Some(store) = self.answers.store().await {
            semantics = semantics.persisting(store);
        }
        Ok(Arc::new(Booted {
            view: session.view(),
            session,
            semantics,
        }))
    }
}

impl Resident for Wired {
    async fn shut(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.fetching.stop();
        self.settling.stop();
        self.copying.stop();
        self.prefetching.stop();
        self.checking.stop();
        self.symbols.stop();
        self.problems.stop();
        if let Some(Ok(booted)) = self.booted.get() {
            booted.semantics.shutdown().await;
        }
        // The servers are gone, so their trees can be collected.
        self.trees.collect(SystemTime::now()).await;
    }
}

/// A project whose first scan is done: what the requests about its review run on.
pub(crate) struct Ready {
    pub id: ProjectId,
    pub subscribers: Subscribers<Connection>,
    /// The entry, for starting the servers of a new revision.
    pub open: Arc<Open>,
    booted: Arc<Booted>,
}

impl Deref for Ready {
    type Target = Booted;

    fn deref(&self) -> &Booted {
        &self.booted
    }
}

impl Ready {
    /// The client of the platform that hosts the pull request on view. For a changeset that is no
    /// pull request, which nothing is asked of a platform about, GitHub's. Fails for a host that is
    /// neither one Vet knows nor one the viewer has an account for.
    pub(crate) async fn hosting(&self) -> Result<Hosting> {
        match self.session.source().await {
            Source::PullRequest { remote, .. } => self.open.hosting_for(&remote).await,
            _ => Ok(self.open.hosting(PlatformKind::GitHub)),
        }
    }
}

impl Orchestrator {
    /// The project `id` once its first scan is done, waiting for it if need be.
    pub(crate) async fn ready(&self, id: ProjectId) -> Result<Ready> {
        let open = self.find(id).await?;
        let booted = open.booted().await?.clone();
        Ok(Ready {
            id: open.id,
            subscribers: open.subscribers.clone(),
            open,
            booted,
        })
    }

    /// The platform of `remote`, for a request about the whole process (which is not one project's).
    pub(crate) async fn kind_of(&self, remote: &RemoteKey) -> Result<PlatformKind> {
        kind_of(remote, &self.accounts).await
    }

    /// What `ask` makes of the live session of the project `id`, once its first scan is done; a
    /// failure of either is the message of the response.
    pub(crate) async fn asking<T, E: Into<Error>>(
        &self,
        id: ProjectId,
        ask: impl AsyncFnOnce(&Live) -> std::result::Result<T, E>,
    ) -> Result<T> {
        let ready = self.ready(id).await?;
        ask(&ready.session).await.map_err(Into::into)
    }

    /// The background work of the project running now. Needs no scan.
    pub(crate) async fn tasks(&self, id: ProjectId) -> Result<Response> {
        let open = self.find(id).await?;
        let tasks = open.tasks.running();
        Ok(Response::Tasks {
            tasks: tasks.iter().map(|t| mapping::progress::task(id, t)).collect(),
        })
    }

    /// Waits until the project has no background work running or pending, then answers. The answer
    /// is a point in time: work a later request or an edit starts is not waited for.
    pub(crate) async fn wait_for_quiet(&self, id: ProjectId) -> Result<Response> {
        let open = self.find(id).await?;
        // The first scan is background work too, and starting the watcher is what makes a later
        // edit visible; wait for it before asking whether the project is quiet.
        let _ = open.booted().await;
        // An edit the watcher has not delivered yet is work the client would otherwise miss: have
        // the settle loop look at the working tree as it is now.
        open.settle_flush.request();
        while !open.quiet() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(Response::Done)
    }

    /// Unsubscribes the connection; the last one to leave closes the project.
    pub(crate) async fn close(&self, id: ProjectId, from: &Connection) -> Result<()> {
        if let Ok(open) = self.projects.find(id).await {
            open.screens.set(from, false);
        }
        Ok(self.projects.close(id, from).await?)
    }

    pub(crate) async fn wire(
        &self,
        id: ProjectId,
        subscribers: Subscribers<Connection>,
        repo: PathBuf,
        root: String,
        entry: Arc<OnceLock<Weak<Open>>>,
    ) -> Result<Wired> {
        let announced = subscribers.clone();
        let tasks = Tasks::new(
            TASK_UPDATES,
            Box::new(move |task| announced.broadcast(Event::Task(mapping::progress::task(id, &task)))),
        );
        let announcer = Arc::new(Announcer {
            project: id,
            subscribers: subscribers.clone(),
        });
        self.platforms.tell(&announcer);
        Ok(Wired {
            // A fetch holds its thread for as long as the network takes, so it has a budget of
            // its own: it must not take the review's threads, nor the semantic lane's.
            fetch: Fetch::new(GixRepo::new(&repo, Budget::new(FETCH_IO)), tasks.clone()),
            accounts: self.accounts.clone(),
            platforms: self.platforms.clone(),
            _announcer: announcer,
            tasks,
            screens: ReviewScreens::default(),
            plan: Plan {
                id,
                repo,
                root,
                store: self.store.clone(),
                budgets: self.budgets.clone(),
                debounce: self.config.debounce,
                quiet: self.config.quiet,
                poll: self.config.poll,
                servers: self.servers.clone(),
                backends: self.backends.clone(),
                subscribers,
                entry,
            },
            booted: OnceCell::new(),
            settling: TaskSlot::default(),
            edits_waiting: Arc::default(),
            fetching: TaskSlot::default(),
            copying: TaskSlot::default(),
            prefetching: TaskSlot::default(),
            checking: TaskSlot::default(),
            checks_pace: self.config.checks_pace,
            checks_settle: self.config.checks_settle,
            symbols: TaskSlot::default(),
            problems: TaskSlot::default(),
            open_files: Arc::default(),
            closed: AtomicBool::new(false),
            trees: self.trees.clone(),
            answers: self.answers.clone(),
            #[cfg(debug_assertions)]
            analysis_failures: self.analysis_failures.clone(),
            background: self.background.clone(),
            settle_flush: Arc::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use vet_app::ports::Store;
    use vet_app::progress::TaskKind;
    use vet_app::session::View;
    use vet_core::content::BlobId;
    use vet_protocol::{ServerMessage, TaskState};
    use vet_transport::Events;

    use super::*;
    use crate::{Config, open_store};

    /// The announcements of task `id` that are waiting for the connection.
    fn announced(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<ServerMessage>, id: u32) -> Vec<vet_protocol::Task> {
        let mut found = Vec::new();
        while let Ok(message) = inbox.try_recv() {
            if let ServerMessage::Event {
                event: Event::Task(task),
            } = message
                && task.id == id
            {
                found.push(task);
            }
        }
        found
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repository with one commit and `files` edited in the working tree; the edited texts.
    fn edited_repo(files: usize) -> (tempfile::TempDir, Vec<String>) {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q", "-b", "main"]);
        for n in 0..files {
            std::fs::write(repo.path().join(format!("f{n}.txt")), format!("file {n}\n")).unwrap();
        }
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "base"]);
        let texts: Vec<String> = (0..files).map(|n| format!("file {n}\nedited\n")).collect();
        for (n, text) in texts.iter().enumerate() {
            std::fs::write(repo.path().join(format!("f{n}.txt")), text).unwrap();
        }
        (repo, texts)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_copy_ahead_draws_on_the_background_budget_and_never_on_the_reviews() {
        let data = tempfile::tempdir().unwrap();
        // The tree is never quiet long enough for the session to copy it by itself.
        let config = Config {
            quiet: Duration::from_secs(3600),
            ..Config::default()
        };
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), config);
        let budgets = orchestrator.budgets().clone();
        let within = Duration::from_secs(20);
        let (events, _inbox) = Events::channel();
        let session = async |repo: &std::path::Path| {
            let (project, _) = orchestrator
                .open(repo.to_str().unwrap(), Connection(events.clone()))
                .await
                .unwrap();
            orchestrator.ready(project).await.unwrap().session.clone()
        };
        let missing = async |texts: &[String]| {
            let ids = texts.iter().map(|t| BlobId::of(t.as_bytes())).collect();
            orchestrator.store.missing_blobs(ids).await.unwrap()
        };

        // With every unit of the review's budget taken, a copy is made.
        let (repo, texts) = edited_repo(8);
        let first = session(repo.path()).await;
        first.manifest(View::Changeset).await.unwrap();
        let review = budgets.review_io.acquire_all().await;
        let copied = tokio::time::timeout(within, first.copy_ahead()).await;
        copied.expect("the copy finishes").unwrap();
        drop(review);
        assert_eq!(missing(&texts).await, []);

        // While a copy waits for the background budget, a project opens and shows its changes.
        let background = budgets.background_io.acquire_all().await;
        let (repo, texts) = edited_repo(5);
        let second = session(repo.path()).await;
        let copy = tokio::spawn({
            let second = second.clone();
            async move { second.copy_ahead().await }
        });
        let manifest = tokio::time::timeout(within, second.manifest(View::Changeset)).await;
        assert_eq!(manifest.expect("the manifest is answered").unwrap().files.len(), 5);
        assert!(!copy.is_finished());
        drop(background);
        tokio::time::timeout(within, copy).await.unwrap().unwrap().unwrap();
        assert_eq!(missing(&texts).await, []);
    }

    #[tokio::test]
    async fn the_running_tasks_are_listed_and_a_subscriber_hears_their_progress_a_few_times_not_every_time() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        let data = tempfile::tempdir().unwrap();
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), Config::default());
        let (events, mut inbox) = Events::channel();
        let (project, _) = orchestrator
            .open(repo.path().to_str().unwrap(), Connection(events))
            .await
            .unwrap();
        let tasks = orchestrator.find(project).await.unwrap().tasks.clone();

        let index = tasks.start(TaskKind::ServerIndex, "Indexing rust-analyzer");
        let id = tasks.running()[0].id;
        for done in 0..200 {
            index.progress(done, 200);
        }
        let Ok(Response::Tasks { tasks: listed }) = orchestrator.tasks(project).await else {
            panic!("no tasks listed");
        };
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].title.as_str(), listed[0].project),
            ("Indexing rust-analyzer", project)
        );
        assert_eq!((listed[0].done, listed[0].total), (Some(199), Some(200)));

        index.finish();
        let heard = announced(&mut inbox, id);
        assert!(heard.len() < 10, "{} announcements", heard.len());
        assert_eq!(heard.first().map(|t| &t.state), Some(&TaskState::Running));
        assert_eq!(
            heard.last().map(|t| (&t.state, t.done)),
            Some((&TaskState::Finished, Some(199)))
        );
        let Ok(Response::Tasks { tasks: listed }) = orchestrator.tasks(project).await else {
            panic!("no tasks listed");
        };
        assert!(listed.is_empty());
    }
}
