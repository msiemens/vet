//! Opening and closing projects: assembling the adapters one project runs on.
//!
//! Opening answers as soon as the repository is found. What needs the first
//! scan (the live session and the semantic layer built on it) is assembled by
//! the first request that needs it, or by the task `open` spawns, whichever is
//! first; the others wait for it. Meanwhile the scan reports as a task.

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime};

use tokio::sync::OnceCell;
use vet_app::changeset::{RemoteKey, Source};
use vet_app::fetch::Fetch;
use vet_app::ignore::IgnoringWatcher;
use vet_app::platform::PlatformKind;
use vet_app::progress::Tasks;
use vet_app::registry::{Resident, Subscribers};
use vet_app::semantics::{Links, Semantics};
use vet_app::session::RevisionView;
use vet_app::settle::{SettleFlush, Settling};
use vet_app::supervise::Supervisor;
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
use crate::feed::Feed;
use crate::hosting::{Announcer, Hosting, Platforms};
use crate::mutex::lock;
use crate::screens::ReviewScreens;
use crate::supervisor::{Fetches, Policy, Syncs};
use crate::{LanguageServers, Live, Open, Orchestrator, PlatformAccounts, Semantic};

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

/// What one open project runs on.
pub(crate) struct Wired {
    /// The background work of the project, announced to its subscribers.
    pub tasks: Tasks,
    /// The project's refs, shared with the pull request feed.
    pub fetch: Arc<Fetch<GixRepo>>,
    /// Where `fetch` finds the credentials of the hosting platforms.
    pub accounts: Arc<PlatformAccounts>,
    /// Where pull requests are asked for.
    pub platforms: Arc<Platforms>,
    /// Keeps this project among those that hear how the platforms are doing.
    _announcer: Arc<Announcer>,
    /// The connections that show the review screen, which the language servers run for.
    pub screens: ReviewScreens,
    plan: Plan,
    booted: OnceCell<Arc<Booted>>,
    /// Whether edits of the working tree wait to settle, as the subscribers were last told.
    edits_waiting: Arc<AtomicBool>,
    /// Whether a ref write waits for its window before the branch on view is looked at again.
    refs_waiting: Arc<AtomicBool>,
    /// The pull request feed: the last listing, and the cycle that keeps it. Its run ends with the
    /// session.
    feed: Feed,
    /// How long the checks task waits between two rounds while some checks still run.
    checks_pace: Duration,
    /// How long after an action on a run the checks are read once more.
    checks_settle: Duration,
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
    /// The project's background runs: the settle loop, the fetch timer, the feed, the analysis, the
    /// checks, the copy ahead and the server starts. They end with the project.
    pub(crate) supervisor: Supervisor,
    /// What the supervisor keeps running for the state of the project: the servers, their
    /// retiring, the analysis, the checks and the copy ahead of the changeset on view.
    pub(crate) policy: Arc<Policy>,
    /// The fetches of the project's remotes, whoever asks for them.
    pub(crate) fetches: Fetches,
    /// The syncs of the pull request on view with its platform, which every cause that asks for
    /// one shares.
    pub(crate) sync: Syncs,
    /// For tests: how many boots this project attempted and how many settle loops it started, so
    /// a failed boot can be told from a boot that never happened, and a retry from a second loop.
    /// A debug build only.
    #[cfg(debug_assertions)]
    pub(crate) boots: Arc<AtomicUsize>,
    #[cfg(debug_assertions)]
    pub(crate) settles: Arc<AtomicUsize>,
    /// A client's request to reconcile the working tree now, taken by the settle loop.
    pub(crate) settle_flush: Arc<SettleFlush>,
}

impl Wired {
    /// The session and its semantic layer, once the first scan is done. The
    /// first caller scans; the others wait for it. Only a scan that succeeded is
    /// kept: a boot that failed is given to the caller that asked for it, and
    /// the next caller that needs the project boots again.
    pub(crate) async fn booted(&self) -> Result<&Arc<Booted>> {
        self.booted.get_or_try_init(|| self.boot()).await
    }

    /// Whether edits of the working tree wait to settle into a snapshot.
    pub(crate) fn edits_waiting(&self) -> bool {
        self.edits_waiting.load(Ordering::SeqCst)
    }

    /// Whether a ref write waits for its window before the branch on view is looked at again.
    pub(crate) fn refs_waiting(&self) -> bool {
        self.refs_waiting.load(Ordering::SeqCst)
    }

    /// Whether the project has no background work running or pending: no task of its own, no
    /// progress task of the session, no edit or ref write waiting to settle, no settled result the
    /// session has yet to announce, and no reconcile a client asked for and the settle loop has not
    /// made.
    pub(crate) fn quiet(&self) -> bool {
        let settling = self.booted_now().is_none_or(|booted| !booted.session.settling_busy());
        !self.tasks.any_running()
            && self.supervisor.live() == 0
            && !self.edits_waiting()
            && !self.refs_waiting()
            && settling
            && !self.settle_flush.requested()
    }

    /// The session if it has scanned successfully already.
    pub(crate) fn booted_now(&self) -> Option<&Arc<Booted>> {
        self.booted.get()
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

    /// The project's pull request feed, keeping the last listing.
    pub(crate) fn feed(&self) -> &Feed {
        &self.feed
    }

    /// How long the checks task waits between two rounds while some checks still run.
    pub(crate) fn checks_pace(&self) -> Duration {
        self.checks_pace
    }

    /// How long after an action on a run the checks are read once more.
    pub(crate) fn checks_settle(&self) -> Duration {
        self.checks_settle
    }

    /// The changed files the viewer has open, which the background analysis reads first.
    pub(crate) fn open_files(&self) -> Vec<String> {
        lock(&self.open_files).clone()
    }

    /// Remembers the changed files the viewer has open.
    pub(crate) fn set_open_files(&self, paths: Vec<String>) {
        *lock(&self.open_files) = paths;
    }

    async fn boot(&self) -> Result<Arc<Booted>> {
        #[cfg(debug_assertions)]
        self.boots.fetch_add(1, Ordering::SeqCst);
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
        .refs_waiting(self.refs_waiting.clone())
        .flushable(self.settle_flush.clone())
        // Off view and without a review, the working tree is not scanned: an edit only marks it
        // changed, and coming on view settles it once.
        .wanted(session.settle_wanted());
        // Attached once, after the last step that can fail, so the analysis never follows a boot
        // that did not succeed.
        crate::reconcile::attach(
            &self.supervisor,
            &self.policy,
            entry.clone(),
            session.revisions().subscribe(),
        );
        {
            let (session, subscribers, policy) = (session.clone(), subscribers.clone(), self.policy.clone());
            let edit_subscribers = subscribers.clone();
            let waiting = self.edits_waiting.clone();
            let quiet_policy = policy.clone();
            self.supervisor.replace("settling", async move {
                session
                    .follow(
                        settling,
                        |revision| {
                            settled(&subscribers, id, revision);
                            policy.tree_settled();
                        },
                        || quiet_policy.tree_quiet(),
                        |pending| {
                            waiting.store(pending, Ordering::SeqCst);
                            edits_pending(&edit_subscribers, id, pending);
                        },
                    )
                    .await;
            });
        }
        #[cfg(debug_assertions)]
        self.settles.fetch_add(1, Ordering::SeqCst);

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
        self.supervisor.shutdown().await;
        if let Some(booted) = self.booted.get() {
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
        // The work a change of state calls for is started before it is asked whether any runs.
        loop {
            crate::reconcile::reconcile(&open).await;
            if open.quiet() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(Response::Done)
    }

    /// Unsubscribes the connection; the last one to leave closes the project.
    pub(crate) async fn close(&self, id: ProjectId, from: &Connection) -> Result<()> {
        if let Ok(open) = self.projects.find(id).await {
            open.screens.set(from, false);
            open.policy.screen_changed(false);
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
        // A fetch holds its thread for as long as the network takes, so it has a budget of
        // its own: it must not take the review's threads, nor the semantic lane's. The pull
        // request feed shares it, over the platforms and the refs.
        let fetch = Arc::new(Fetch::new(GixRepo::new(&repo, Budget::new(FETCH_IO)), tasks.clone()));
        let supervisor = self.background.supervisor(id);
        let feed = self.pull_request_feed(fetch.clone(), entry.clone(), supervisor.clone());
        let sync = crate::sync::trigger(&supervisor, entry.clone());
        let fetches = crate::fetch::fetches(&supervisor, entry.clone());
        Ok(Wired {
            fetch,
            feed,
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
            edits_waiting: Arc::default(),
            refs_waiting: Arc::default(),
            checks_pace: self.config.checks_pace,
            checks_settle: self.config.checks_settle,
            open_files: Arc::default(),
            closed: AtomicBool::new(false),
            trees: self.trees.clone(),
            answers: self.answers.clone(),
            #[cfg(debug_assertions)]
            analysis_failures: self.analysis_failures.clone(),
            supervisor,
            policy: Arc::default(),
            fetches,
            sync,
            #[cfg(debug_assertions)]
            boots: Arc::default(),
            #[cfg(debug_assertions)]
            settles: Arc::default(),
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
    fn announced(inbox: &mut tokio::sync::mpsc::Receiver<ServerMessage>, id: u32) -> Vec<vet_protocol::Task> {
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_copy_ahead_runs_only_while_the_review_screen_shows_the_tree() {
        // The tree is never quiet long enough for the settle loop to copy it by itself.
        let config = Config {
            quiet: Duration::from_secs(3600),
            ..Config::default()
        };
        let (repo, texts) = edited_repo(4);
        let data = tempfile::tempdir().unwrap();
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), config);
        let (events, _inbox) = Events::channel();
        let connection = Connection(events.clone());
        let (project, _) = orchestrator
            .open(repo.path().to_str().unwrap(), connection.clone())
            .await
            .unwrap();
        let open = orchestrator.find(project).await.unwrap();
        orchestrator.ready(project).await.unwrap();
        let missing = async || {
            let ids = texts.iter().map(|t| BlobId::of(t.as_bytes())).collect();
            orchestrator.store.missing_blobs(ids).await.unwrap()
        };

        // The queue is on screen: the quiet copy copies nothing.
        open.policy.tree_quiet();
        crate::reconcile::reconcile(&open).await;
        assert_eq!(
            open.supervisor.counts("copy").0,
            0,
            "nothing is copied while the queue is on screen"
        );

        // The review screen that comes back copies the tree once.
        orchestrator.set_review_shown(project, true, &connection).await.unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            while !missing().await.is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the review screen copies the tree");
        assert_eq!(open.supervisor.counts("copy").0, 1, "the screen copied once");
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

    /// Puts a repository with one committed file back at `repo`, for a boot after the directory
    /// was removed.
    fn restore_repo(repo: &std::path::Path) {
        std::fs::create_dir_all(repo).unwrap();
        git(repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("f0.txt"), "file 0\n").unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", "base"]);
    }

    /// An orchestrator over a fresh data directory, with `repo` open and its subscriber held.
    /// The data directory is returned to keep the store alive.
    async fn opened_repo(repo: &tempfile::TempDir) -> (tempfile::TempDir, Orchestrator, ProjectId, Arc<Open>) {
        let data = tempfile::tempdir().unwrap();
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), Config::default());
        let (events, _inbox) = Events::channel();
        let (project, _root) = orchestrator
            .open(repo.path().to_str().unwrap(), Connection(events))
            .await
            .unwrap();
        let open = orchestrator.find(project).await.unwrap();
        (data, orchestrator, project, open)
    }

    #[tokio::test]
    async fn a_failed_boot_is_told_to_its_request_and_the_next_request_boots_it() {
        let (repo, _texts) = edited_repo(1);
        let (_data, orchestrator, project, open) = opened_repo(&repo).await;

        // The repository is gone, so the boot the request triggers cannot scan.
        std::fs::remove_dir_all(repo.path()).unwrap();
        let failed = orchestrator.ready(project).await;
        assert!(
            failed.is_err(),
            "the request that triggered the failed boot is answered with its failure"
        );
        assert_eq!(open.boots.load(Ordering::SeqCst), 1, "one boot was attempted");

        // The repository is back, so the next request boots again and succeeds.
        restore_repo(repo.path());
        let ready = orchestrator.ready(project).await;
        assert!(ready.is_ok(), "the next request boots again: {:?}", ready.err());
        assert_eq!(open.boots.load(Ordering::SeqCst), 2, "the next request booted again");
    }

    #[tokio::test]
    async fn the_retry_after_a_failed_boot_starts_exactly_one_settle_loop() {
        let (repo, _texts) = edited_repo(1);
        let (_data, orchestrator, project, open) = opened_repo(&repo).await;

        std::fs::remove_dir_all(repo.path()).unwrap();
        let failed = orchestrator.ready(project).await;
        assert!(failed.is_err(), "the boot fails");
        assert_eq!(
            open.settles.load(Ordering::SeqCst),
            0,
            "a boot that failed before wiring starts no settle loop"
        );

        restore_repo(repo.path());
        let ready = orchestrator.ready(project).await;
        assert_eq!(
            open.settles.load(Ordering::SeqCst),
            1,
            "the retry starts exactly one settle loop"
        );
        assert!(ready.is_ok(), "the retry succeeds: {:?}", ready.err());
    }

    #[tokio::test]
    async fn two_requests_during_one_boot_share_it() {
        let (repo, _texts) = edited_repo(1);
        let (_data, orchestrator, project, open) = opened_repo(&repo).await;

        let (first, second) = tokio::join!(orchestrator.ready(project), orchestrator.ready(project));
        assert!(first.is_ok() && second.is_ok(), "both requests are answered");
        assert_eq!(
            open.boots.load(Ordering::SeqCst),
            1,
            "the requests shared the boot in flight"
        );
        assert_eq!(open.settles.load(Ordering::SeqCst), 1, "exactly one settle loop runs");
    }
}
