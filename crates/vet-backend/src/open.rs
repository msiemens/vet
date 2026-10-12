//! Opening and closing projects: assembling the adapters one project runs on.
//!
//! Opening answers as soon as the repository is found. What needs the first
//! scan (the live session and the semantic layer built on it) is assembled by
//! the first request that needs it, or by the task `open` spawns, whichever is
//! first; the others wait for it. Meanwhile the scan reports as a task.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use tokio::sync::OnceCell;
use tokio::task::AbortHandle;
use vet_app::changeset::Source;
use vet_app::fetch::Fetch;
use vet_app::ignore::IgnoringWatcher;
use vet_app::limits::{Limiting, RestoreLimits};
use vet_app::platform::PlatformKind;
use vet_app::progress::Tasks;
use vet_app::registry::{Resident, Subscribers};
use vet_app::semantics::{Links, Semantics};
use vet_app::session::RevisionView;
use vet_app::settle::Settling;
use vet_app::trust::Trust;
use vet_infra::budget::Budgets;
use vet_infra::clock::{TokioClock, TokioDetach, TokioLinger, unix_now};
use vet_infra::git::GixRepo;
use vet_infra::language;
use vet_infra::lsp::LspLauncher;
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::KeptTrees;
use vet_infra::restore::ToolRestorer;
use vet_infra::store::SqliteStore;
use vet_infra::watch::FsWatcher;
use vet_mapping as mapping;
use vet_protocol::{Event, PlatformConnection, ProjectId, Response};
use vet_semantic::pool::Supervision;
use vet_semantic::ports::Backend;

use crate::caches::PersistentAnswers;
use crate::events::{Connection, settled};
use crate::hosting::Hosting;
use crate::{
    AzureDevOps, GitHub, LanguageServers, Live, Open, Orchestrator, PlatformAccounts, Semantic, message,
    start_servers_in_background,
};

/// The least time between two progress announcements of one task, so a step that
/// reports per file reaches the UI a few times a second.
const TASK_UPDATES: Duration = Duration::from_millis(250);

/// What a project runs on once its first scan is done.
pub struct Booted {
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

/// What one open project runs on.
pub struct Wired {
    /// The background work of the project, announced to its subscribers.
    pub tasks: Tasks,
    pub fetch: Fetch<GixRepo>,
    /// Where `fetch` finds the credentials of the hosting platforms.
    pub accounts: Arc<PlatformAccounts>,
    /// Where pull requests are asked for.
    pub github: Arc<GitHub>,
    pub azure_devops: Arc<AzureDevOps>,
    /// How the last sync with each platform host went, to announce a stop once and not on every try.
    connections: Mutex<HashMap<String, PlatformConnection>>,
    plan: Plan,
    booted: OnceCell<Result<Arc<Booted>, String>>,
    settling: Mutex<Option<AbortHandle>>,
    fetching: Mutex<Option<AbortHandle>>,
    /// The copy of the working tree's content ahead of a review; a newer tree's replaces it.
    copying: Mutex<Option<AbortHandle>>,
    closed: AtomicBool,
    trees: Arc<KeptTrees>,
    /// The answers of the language servers kept between launches.
    answers: Arc<PersistentAnswers>,
}

impl Wired {
    /// The session and its semantic layer, once the first scan is done. The
    /// first caller scans; the others wait for it. A scan that failed fails
    /// every caller with the same message.
    pub async fn booted(&self) -> Result<&Arc<Booted>, String> {
        self.booted
            .get_or_init(|| self.boot())
            .await
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The session if it has scanned successfully already.
    pub fn booted_now(&self) -> Option<&Arc<Booted>> {
        self.booted.get()?.as_ref().ok()
    }

    /// The client of the platform `kind`.
    pub fn hosting(&self, kind: PlatformKind) -> Hosting {
        Hosting::of(kind, &self.github, &self.azure_devops)
    }

    /// Records how syncing with `host` went and answers how it went before.
    pub fn set_connection(&self, host: &str, now: PlatformConnection) -> Option<PlatformConnection> {
        let mut all = self.connections.lock().unwrap_or_else(PoisonError::into_inner);
        all.insert(host.to_owned(), now)
    }

    /// How syncing with `host` went last, if it was tried.
    pub fn connection(&self, host: &str) -> Option<PlatformConnection> {
        let all = self.connections.lock().unwrap_or_else(PoisonError::into_inner);
        all.get(host).cloned()
    }

    /// Hands over the task that fetches periodically, which ends with the session.
    pub fn keep_fetching(&self, task: AbortHandle) {
        *self.fetching.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
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
        let mut copying = self.copying.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(older) = copying.replace(task.abort_handle()) {
            older.abort();
        }
    }

    /// Stops the copy of an older tree, unless a mark is waiting for it.
    fn stop_copying(&self) {
        if let Some(older) = self.copying.lock().unwrap_or_else(PoisonError::into_inner).take() {
            older.abort();
        }
    }

    async fn boot(&self) -> Result<Arc<Booted>, String> {
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
        let watcher = FsWatcher::open(repo, *poll).map_err(message)?;
        let git = GixRepo::new(repo).budgeted(budgets.review_io.clone());
        // The copy ahead of a review is work nobody waits for: it draws on a budget of its own,
        // never on the review's.
        let session = Arc::new(
            Live::open_reporting(git.clone(), store.clone(), root.clone(), unix_now, self.tasks.clone())
                .await
                .map_err(message)?
                .copying_through(
                    git.budgeted(budgets.background_io.clone()),
                    store.budgeted(budgets.background_io.clone()),
                ),
        );
        if self.closed.load(Ordering::SeqCst) {
            return Err("the session was closed".into());
        }

        // The content of the tree on view is copied ahead of a review once the tree has been
        // quiet for a while: the one found at open too, as the first manifest comes first.
        let settling = Settling::new(
            IgnoringWatcher::new(watcher, git.clone()),
            TokioClock::new(),
            debounce.as_millis() as u64,
            Some(session.settled_snapshot().await),
        )
        .quiet_after(quiet.as_millis() as u64);
        let task = {
            let (session, subscribers, entry) = (session.clone(), subscribers.clone(), entry.clone());
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
                    )
                    .await;
            })
        };
        *self.settling.lock().unwrap_or_else(PoisonError::into_inner) = Some(task.abort_handle());

        // The semantic lane reads the same repository and store through handles of its own, so
        // what it reads draws on its budget and never on the review's.
        let reading = Arc::new(session.acquisition().through(
            git.budgeted(budgets.semantic_io.clone()),
            store.budgeted(budgets.semantic_io.clone()),
        ));
        let limits = RestoreLimits::default();
        let restorer = Limiting::new(
            ToolRestorer::new()
                .budgeted(budgets)
                .reusing(repo.clone())
                .reporting(self.tasks.clone()),
            limits.clone(),
            {
                let subscribers = subscribers.clone();
                Box::new(move |failures| {
                    let failures = mapping::semantic::restore_failures(&failures);
                    subscribers.broadcast(Event::RestoreFailed { project: id, failures });
                })
            },
        );
        let mut semantics = Semantics::new(
            Materialize::new(reading.clone(), self.trees.clone()),
            LspLauncher::new(servers.clone(), restorer)
                .budgeted(budgets)
                .reporting(self.tasks.clone()),
            repo.clone(),
            backends.to_vec(),
            Trust::Local,
            Links {
                revisions: session.revisions(),
                limits,
                tasks: self.tasks.clone(),
                linger: Arc::new(TokioLinger),
            },
        )
        .reading(reading)
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
        });
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
        if let Some(fetching) = self.fetching.lock().unwrap_or_else(PoisonError::into_inner).take() {
            fetching.abort();
        }
        if let Some(settling) = self.settling.lock().unwrap_or_else(PoisonError::into_inner).take() {
            settling.abort();
        }
        if let Some(copying) = self.copying.lock().unwrap_or_else(PoisonError::into_inner).take() {
            copying.abort();
        }
        if let Some(Ok(booted)) = self.booted.get() {
            booted.semantics.shutdown().await;
        }
        // The servers are gone, so their trees can be collected.
        self.trees.collect(SystemTime::now()).await;
    }
}

/// A project whose first scan is done: what the requests about its review run on.
pub struct Ready {
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
    /// pull request, which nothing is asked of a platform about, GitHub's.
    pub(crate) async fn hosting(&self) -> Hosting {
        let kind = match self.session.source().await {
            Source::PullRequest { remote, .. } => remote.kind(),
            _ => PlatformKind::GitHub,
        };
        self.open.hosting(kind)
    }
}

impl Orchestrator {
    /// The project `id` once its first scan is done, waiting for it if need be.
    pub(crate) async fn ready(&self, id: ProjectId) -> Result<Ready, String> {
        let open = self.find(id).await?;
        let booted = open.booted().await?.clone();
        Ok(Ready {
            id: open.id,
            subscribers: open.subscribers.clone(),
            open,
            booted,
        })
    }

    /// The background work of the project running now. Needs no scan.
    pub(crate) async fn tasks(&self, id: ProjectId) -> Result<Response, String> {
        let open = self.find(id).await?;
        let tasks = open.tasks.running();
        Ok(Response::Tasks {
            tasks: tasks.iter().map(|t| mapping::progress::task(id, t)).collect(),
        })
    }

    /// Unsubscribes the connection; the last one to leave closes the project.
    pub(crate) async fn close(&self, id: ProjectId, from: &Connection) -> Result<(), String> {
        self.projects.close(id, from).await.map_err(message)
    }

    pub(crate) async fn wire(
        &self,
        id: ProjectId,
        subscribers: Subscribers<Connection>,
        repo: PathBuf,
        root: String,
        entry: Arc<OnceLock<Weak<Open>>>,
    ) -> Result<Wired, String> {
        let announced = subscribers.clone();
        let tasks = Tasks::new(
            TASK_UPDATES,
            Box::new(move |task| announced.broadcast(Event::Task(mapping::progress::task(id, &task)))),
        );
        Ok(Wired {
            // A fetch holds its thread for as long as the network takes, so it has a budget of
            // its own: it must not take the review's threads, nor the semantic lane's.
            fetch: Fetch::new(GixRepo::new(&repo), tasks.clone()),
            accounts: self.accounts.clone(),
            github: self.github.clone(),
            azure_devops: self.azure_devops.clone(),
            tasks,
            connections: Mutex::default(),
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
            settling: Mutex::new(None),
            fetching: Mutex::new(None),
            copying: Mutex::new(None),
            closed: AtomicBool::new(false),
            trees: self.trees.clone(),
            answers: self.answers.clone(),
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
