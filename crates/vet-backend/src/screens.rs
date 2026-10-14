//! Which connections show the review screen of a project. A changeset's language servers are
//! wanted while one does, and not while the window is on the queue, where moving over the rows
//! puts one changeset after the other on view.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use vet_app::registry::Subscriber;
use vet_protocol::{ProjectId, Response};

use crate::error::Result;
use crate::events::Connection;
use crate::{Open, Orchestrator, start_servers_in_background};

/// The connections that show the review screen of one project.
#[derive(Default)]
pub(crate) struct ReviewScreens(Mutex<Vec<Connection>>);

impl ReviewScreens {
    fn connections(&self) -> MutexGuard<'_, Vec<Connection>> {
        // A list of connections is never left half-written.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn set(&self, connection: &Connection, shown: bool) {
        let mut connections = self.connections();
        connections.retain(|c| !c.same_connection(connection));
        if shown {
            connections.push(connection.clone());
        }
    }

    /// Whether a connection that is still there shows the review screen.
    pub(crate) fn shown(&self) -> bool {
        let mut connections = self.connections();
        connections.retain(|c| !c.is_closed());
        !connections.is_empty()
    }
}

impl Orchestrator {
    /// The connection shows the review screen of the project or leaves it. Coming up starts the
    /// servers the changeset on view needs.
    pub(crate) async fn set_review_shown(
        &self,
        project: ProjectId,
        shown: bool,
        from: &Connection,
    ) -> Result<Response> {
        let open: Arc<Open> = self.find(project).await?;
        open.screens.set(from, shown);
        if shown {
            start_servers_in_background(open);
        } else {
            // The checks of the changeset on view are refreshed for the review screen only; the
            // next `SetReviewShown` starts them again.
            open.stop_checking();
        }
        Ok(Response::Done)
    }
}
