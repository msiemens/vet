//! Opening a project: registering it, and starting what runs for as long as it is open.

use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};

use vet_infra::clock::TokioClock;
use vet_infra::git::find_root;
use vet_protocol::{Event, ProjectId};

use crate::error::{Error, Result};
use crate::events::Connection;
use crate::feed::run_feed;
use crate::{Open, Orchestrator, scan_in_background};

impl Orchestrator {
    /// The project id and the root of the repository containing `repo`. The first
    /// scan has not necessarily happened yet; requests that need it wait for it.
    pub(crate) async fn open(&self, repo: &str, events: Connection) -> Result<(ProjectId, String)> {
        let repo = find_root(Path::new(repo)).map_err(Error::Invalid)?;
        let root = repo.to_string_lossy().into_owned();
        let entry: Arc<OnceLock<Weak<Open>>> = Arc::default();
        let joining = events.clone();
        let (open, created) = self
            .projects
            .open(&repo, events, |id, subscribers| {
                self.wire(id, subscribers, repo.clone(), root.clone(), entry.clone())
            })
            .await?;
        // A connection that joins while edits wait was not there when it was told.
        if !created && open.edits_waiting() {
            joining.0.send(Event::EditsPending {
                project: open.id,
                pending: true,
            });
        }
        if created {
            let _ = entry.set(Arc::downgrade(&open));
            let (projects, id, grace) = (self.projects.clone(), open.id, self.config.orphan_grace);
            tokio::spawn(async move { projects.reap(id, grace, &TokioClock::new()).await });
            scan_in_background(open.clone());
            // The timer is not a counted run: a client waiting for quiet must not wait for it.
            let timer = open.fetches.clone().every(self.config.fetch_interval);
            open.supervisor.start("fetch timer", timer);
            run_feed(&open);
        }
        Ok((open.id, root))
    }
}
