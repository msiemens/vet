//! The composition root: serves protocol v1 from the real components. Session
//! logic lives in `vet-app`; this crate wires adapters to it and maps its views
//! to protocol messages.

mod mapping;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::AbortHandle;
use vet_app::ignore::IgnoringWatcher;
use vet_app::session::{LiveSession, ThreadView};
use vet_app::settle::Settling;
use vet_core::review::ThreadId;
use vet_infra::clock::TokioClock;
use vet_infra::git::{GixRepo, find_root};
use vet_infra::store::{SqliteStore, unix_now};
use vet_infra::watch::FsWatcher;
use vet_protocol::{Event, Request, Response, SessionId};
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

/// The connections that opened a session. Sessions are one per repository:
/// opening an open repository subscribes to the existing one.
type Subscribers = Arc<Mutex<Vec<Events>>>;

struct Open {
    id: SessionId,
    repo: PathBuf,
    session: Arc<Live>,
    subscribers: Subscribers,
    settling: AbortHandle,
    reaper: AbortHandle,
}

fn live(subscribers: &Subscribers) -> std::sync::MutexGuard<'_, Vec<Events>> {
    let mut subscribers = subscribers.lock().unwrap_or_else(PoisonError::into_inner);
    subscribers.retain(|events| !events.is_closed());
    subscribers
}

pub struct Orchestrator {
    store: SqliteStore,
    config: Config,
    sessions: Sessions,
}

type Sessions = Arc<tokio::sync::Mutex<Vec<Arc<Open>>>>;

impl Open {
    /// Stops watching, deletes the session's refs and records it closed.
    async fn shut(&self) -> Result<(), String> {
        self.settling.abort();
        self.session.close(unix_now()).await.map_err(message)
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
            if let Err(error) = open.shut().await {
                eprintln!("vet: closing orphaned session {id}: {error}");
            }
            return;
        }
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl Orchestrator {
    pub fn new(store: SqliteStore, config: Config) -> Self {
        Self {
            store,
            config,
            sessions: Sessions::default(),
        }
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
        let name = repo
            .file_name()
            .map_or_else(|| repo.display().to_string(), |n| n.to_string_lossy().into_owned());
        let session = Arc::new(
            Live::open(git.clone(), self.store.clone(), name)
                .await
                .map_err(message)?,
        );
        let id = u32::try_from(session.id.0).map_err(message)?;
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
                        for events in live(&subscribers).iter() {
                            events.send(Event::SnapshotSettled { session: id, revision });
                        }
                    })
                    .await;
            })
        };
        let reaper = tokio::spawn(reap(self.sessions.clone(), id, self.config.orphan_grace));
        sessions.push(Arc::new(Open {
            id,
            repo,
            session,
            subscribers,
            settling: task.abort_handle(),
            reaper: reaper.abort_handle(),
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
            open.shut().await?;
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
        match request {
            Request::Ping { payload } => Ok(Response::Pong { payload }),
            Request::OpenSession { repo } => {
                let (session, root) = self.open(&repo, events).await?;
                Ok(Response::Session { session, root })
            }
            Request::CloseSession { session } => {
                self.close(session, &events).await?;
                Ok(Response::Done)
            }
            Request::Manifest { session } => {
                let manifest = self.find(session).await?.session.manifest().await.map_err(message)?;
                Ok(Response::Manifest(mapping::manifest(session, &manifest)))
            }
            Request::FileContents { session, path } => {
                let text = self
                    .find(session)
                    .await?
                    .session
                    .file_text(&path)
                    .await
                    .map_err(message)?;
                Ok(Response::FileContents {
                    revision: text.revision,
                    base: text.base,
                    target: text.target,
                    marks: Vec::new(),
                })
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
                Ok(Response::Review(review_changed(&open, &events, &update)))
            }
            Request::SetBase { session, base } => {
                let open = self.find(session).await?;
                let (base, revision) = open
                    .session
                    .set_base(mapping::core_base_choice(base))
                    .await
                    .map_err(message)?;
                for events in live(&open.subscribers).iter() {
                    events.send(Event::SnapshotSettled { session, revision });
                }
                Ok(Response::Base(mapping::base(&base)))
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
                Ok(mapping::target_text(&text))
            }
            Request::Threads { session } => {
                let threads = self.find(session).await?.session.threads().await.map_err(message)?;
                Ok(mapping::threads(&threads))
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
            Request::Hover { .. } => Err("hover is not implemented yet".into()),
        }
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

pub async fn open_store(dir: &Path) -> Result<SqliteStore, String> {
    SqliteStore::open(dir, Default::default()).await.map_err(|e| e.0)
}
