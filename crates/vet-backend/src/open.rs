//! Opening and closing projects: assembling the adapters one project runs on.
//!
//! Opening answers as soon as the repository is found. What needs the first
//! scan (the live session and the semantic layer built on it) is assembled by
//! the first request that needs it, or by the task `open` spawns, whichever is
//! first; the others wait for it. Meanwhile the scan reports as a task.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use tokio::sync::OnceCell;
use tokio::task::AbortHandle;
use vet_app::fetch::Fetch;
use vet_app::ignore::IgnoringWatcher;
use vet_app::limits::{Limiting, RestoreLimits};
use vet_app::progress::Tasks;
use vet_app::pull_request::KnownPullRequests;
use vet_app::registry::{Resident, Subscribers};
use vet_app::semantics::{Links, Semantics};
use vet_app::settle::Settling;
use vet_app::trust::Trust;
use vet_infra::clock::{TokioClock, unix_now};
use vet_infra::git::{GixRepo, find_root};
use vet_infra::lsp::LspLauncher;
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::KeptTrees;
use vet_infra::restore::ToolRestorer;
use vet_infra::store::SqliteStore;
use vet_infra::watch::FsWatcher;
use vet_mapping as mapping;
use vet_protocol::{Event, PlatformConnection, ProjectId, Response};
use vet_semantic::ports::Backend;

use crate::events::{Connection, settled};
use crate::fetch::fetch_periodically;
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
    pub semantics: Semantic,
}

/// What `Wired::booted` assembles a project from.
struct Plan {
    id: ProjectId,
    repo: PathBuf,
    root: String,
    store: SqliteStore,
    debounce: Duration,
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
    /// What the platform called the pull requests it has told of.
    pub pull_requests: KnownPullRequests,
    /// How the last sync with each platform host went, to announce a stop once and not on every try.
    connections: Mutex<HashMap<String, PlatformConnection>>,
    plan: Plan,
    booted: OnceCell<Result<Arc<Booted>, String>>,
    settling: Mutex<Option<AbortHandle>>,
    fetching: Mutex<Option<AbortHandle>>,
    closed: AtomicBool,
    trees: Arc<KeptTrees>,
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

    async fn boot(&self) -> Result<Arc<Booted>, String> {
        let Plan {
            id,
            repo,
            root,
            store,
            debounce,
            poll,
            servers,
            backends,
            subscribers,
            entry,
        } = &self.plan;
        let id = *id;
        // Watching starts before the initial scan, so no edit falls between them.
        let watcher = FsWatcher::open(repo, *poll).map_err(message)?;
        let git = GixRepo::new(repo);
        let session = Arc::new(
            Live::open_reporting(git.clone(), store.clone(), root.clone(), unix_now, self.tasks.clone())
                .await
                .map_err(message)?,
        );
        if self.closed.load(Ordering::SeqCst) {
            return Err("the session was closed".into());
        }

        let settling = Settling::new(
            IgnoringWatcher::new(watcher, git),
            TokioClock::new(),
            debounce.as_millis() as u64,
            Some(session.settled_snapshot().await),
        );
        let task = {
            let (session, subscribers, entry) = (session.clone(), subscribers.clone(), entry.clone());
            tokio::spawn(async move {
                session
                    .follow(settling, |revision| {
                        settled(&subscribers, id, revision);
                        if let Some(open) = entry.get().and_then(Weak::upgrade) {
                            start_servers_in_background(open);
                        }
                    })
                    .await;
            })
        };
        *self.settling.lock().unwrap_or_else(PoisonError::into_inner) = Some(task.abort_handle());

        let limits = RestoreLimits::default();
        let restorer = Limiting::new(
            ToolRestorer::new().reusing(repo.clone()).reporting(self.tasks.clone()),
            limits.clone(),
            {
                let subscribers = subscribers.clone();
                Box::new(move |failures| {
                    let failures = mapping::semantic::restore_failures(&failures);
                    subscribers.broadcast(Event::RestoreFailed { project: id, failures });
                })
            },
        );
        let semantics = Semantics::new(
            Materialize::new(session.acquisition(), self.trees.clone()),
            LspLauncher::new(servers.clone(), restorer).reporting(self.tasks.clone()),
            repo.clone(),
            backends.to_vec(),
            Trust::Local,
            Links {
                revisions: session.revisions(),
                limits,
                tasks: self.tasks.clone(),
            },
        );
        Ok(Arc::new(Booted { session, semantics }))
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

    /// The project id and the root of the repository containing `repo`. The first
    /// scan has not necessarily happened yet; requests that need it wait for it.
    pub(crate) async fn open(&self, repo: &str, events: Connection) -> Result<(ProjectId, String), String> {
        let repo = find_root(Path::new(repo))?;
        let root = repo.to_string_lossy().into_owned();
        let entry: Arc<OnceLock<Weak<Open>>> = Arc::default();
        let (open, created) = self
            .projects
            .open(&repo, events, |id, subscribers| {
                self.wire(id, subscribers, repo.clone(), root.clone(), entry.clone())
            })
            .await?;
        if created {
            let _ = entry.set(Arc::downgrade(&open));
            let (projects, id, grace) = (self.projects.clone(), open.id, self.config.orphan_grace);
            tokio::spawn(async move { projects.reap(id, grace, &TokioClock::new()).await });
            start_servers_in_background(open.clone());
            fetch_periodically(&open, self.config.fetch_interval);
        }
        Ok((open.id, root))
    }

    /// Unsubscribes the connection; the last one to leave closes the project.
    pub(crate) async fn close(&self, id: ProjectId, from: &Connection) -> Result<(), String> {
        self.projects.close(id, from).await.map_err(message)
    }

    async fn wire(
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
            fetch: Fetch::new(GixRepo::new(&repo), tasks.clone()),
            accounts: self.accounts.clone(),
            github: self.github.clone(),
            azure_devops: self.azure_devops.clone(),
            pull_requests: KnownPullRequests::default(),
            tasks,
            connections: Mutex::default(),
            plan: Plan {
                id,
                repo,
                root,
                store: self.store.clone(),
                debounce: self.config.debounce,
                poll: self.config.poll,
                servers: self.servers.clone(),
                backends: self.backends.clone(),
                subscribers,
                entry,
            },
            booted: OnceCell::new(),
            settling: Mutex::new(None),
            fetching: Mutex::new(None),
            closed: AtomicBool::new(false),
            trees: self.trees.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use vet_app::progress::TaskKind;
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

    #[tokio::test]
    async fn the_running_tasks_are_listed_and_a_subscriber_hears_their_progress_a_few_times_not_every_time() {
        let repo = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .unwrap();
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
