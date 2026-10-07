//! Requests about which changeset is on view and which there are.

use vet_mapping as mapping;
use vet_protocol::{ChangesetKey, Event, ProjectId, Response};

use crate::{Orchestrator, message, start_servers_in_background};

impl Orchestrator {
    pub(crate) async fn list_changesets(&self, project: ProjectId) -> Result<Response, String> {
        let list = self.ready(project).await?.session.changesets().await.map_err(message)?;
        Ok(mapping::changesets(&list))
    }

    pub(crate) async fn open_changeset(&self, project: ProjectId, changeset: ChangesetKey) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let source = mapping::core_changeset_key(changeset);
        let moved = open.session.open_changeset(source.clone()).await.map_err(message)?;
        if let Some(revision) = moved {
            open.subscribers.broadcast(Event::ChangesetOpened {
                project,
                revision,
                changeset: mapping::changeset_key(&source),
            });
            start_servers_in_background(open.open.clone());
        }
        Ok(Response::Done)
    }
}
