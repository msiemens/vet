//! Opening a project: registering it, and starting what runs for as long as it is open.

use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};

use vet_infra::clock::TokioClock;
use vet_infra::git::find_root;
use vet_protocol::ProjectId;

use crate::events::Connection;
use crate::fetch::fetch_periodically;
use crate::{Open, Orchestrator, start_servers_in_background};

impl Orchestrator {
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
}
