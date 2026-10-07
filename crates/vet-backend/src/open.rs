//! Opening and closing sessions: assembling the adapters one session runs on.

use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};
use std::time::SystemTime;

use tokio::task::AbortHandle;
use vet_app::ignore::IgnoringWatcher;
use vet_app::limits::{Limiting, RestoreLimits};
use vet_app::registry::{Resident, Subscribers};
use vet_app::semantics::Semantics;
use vet_app::settle::Settling;
use vet_app::trust::Trust;
use vet_infra::clock::{TokioClock, unix_now};
use vet_infra::git::{GixRepo, find_root};
use vet_infra::lsp::LspLauncher;
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::KeptTrees;
use vet_infra::restore::ToolRestorer;
use vet_infra::watch::FsWatcher;
use vet_mapping as mapping;
use vet_protocol::{Event, SessionId};

use crate::events::{Connection, settled};
use crate::{Live, Open, Orchestrator, Semantic, message, start_servers_in_background};

/// What one open session runs on.
pub struct Wired {
    pub session: Arc<Live>,
    pub semantics: Semantic,
    settling: AbortHandle,
    trees: Arc<KeptTrees>,
}

impl Resident for Wired {
    async fn shut(&self) {
        self.settling.abort();
        self.semantics.shutdown().await;
        // The servers are gone, so their trees can be collected.
        self.trees.collect(SystemTime::now()).await;
    }
}

impl Orchestrator {
    /// The session id and the root of the repository containing `repo`.
    pub(crate) async fn open(&self, repo: &str, events: Connection) -> Result<(SessionId, String), String> {
        let repo = find_root(Path::new(repo))?;
        let root = repo.to_string_lossy().into_owned();
        let this: Arc<OnceLock<Weak<Open>>> = Arc::default();
        let (open, created) = self
            .sessions
            .open(&repo, events, |id, subscribers| {
                self.wire(id, subscribers, repo.clone(), root.clone(), this.clone())
            })
            .await?;
        if created {
            let _ = this.set(Arc::downgrade(&open));
            let (sessions, id, grace) = (self.sessions.clone(), open.id, self.config.orphan_grace);
            tokio::spawn(async move { sessions.reap(id, grace, &TokioClock::new()).await });
            start_servers_in_background(open.clone());
        }
        Ok((open.id, root))
    }

    /// Unsubscribes the connection; the last one to leave closes the session.
    pub(crate) async fn close(&self, id: SessionId, from: &Connection) -> Result<(), String> {
        self.sessions.close(id, from).await.map_err(message)
    }

    async fn wire(
        &self,
        id: SessionId,
        subscribers: Subscribers<Connection>,
        repo: std::path::PathBuf,
        root: String,
        this: Arc<OnceLock<Weak<Open>>>,
    ) -> Result<Wired, String> {
        // Watching starts before the initial scan, so no edit falls between them.
        let watcher = FsWatcher::open(&repo, self.config.poll).map_err(message)?;
        let git = GixRepo::new(&repo);
        let session = Arc::new(
            Live::open(git.clone(), self.store.clone(), root, unix_now)
                .await
                .map_err(message)?,
        );

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
                        settled(&subscribers, id, revision);
                        if let Some(open) = this.get().and_then(Weak::upgrade) {
                            start_servers_in_background(open);
                        }
                    })
                    .await;
            })
        };
        let limits = RestoreLimits::default();
        let restorer = Limiting::new(
            ToolRestorer::new().reusing(repo.clone()),
            limits.clone(),
            Box::new(move |failures| {
                let failures = mapping::semantic::restore_failures(&failures);
                subscribers.broadcast(Event::RestoreFailed { session: id, failures });
            }),
        );
        let semantics = Semantics::new(
            Materialize::new(session.acquisition(), self.trees.clone()),
            LspLauncher::new(self.servers.clone(), restorer),
            repo,
            self.backends.to_vec(),
            limits,
            Trust::Local,
        );
        Ok(Wired {
            session,
            semantics,
            settling: task.abort_handle(),
            trees: self.trees.clone(),
        })
    }
}
