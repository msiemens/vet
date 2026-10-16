//! Which connections show the review screen of a project. A changeset's language servers are
//! wanted while one does, and not while the window is on the queue, where moving over the rows
//! puts one changeset after the other on view.

use std::sync::{Arc, Mutex, MutexGuard};

use vet_app::registry::Subscriber;
use vet_protocol::{ProjectId, Response};

use crate::error::Result;
use crate::events::Connection;
use crate::mutex::lock;
use crate::{Open, Orchestrator};

/// The connections that show the review screen of one project.
#[derive(Default)]
pub(crate) struct ReviewScreens(Mutex<Vec<Connection>>);

impl ReviewScreens {
    fn connections(&self) -> MutexGuard<'_, Vec<Connection>> {
        // A list of connections is never left half-written.
        lock(&self.0)
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
    /// The connection shows the review screen of the project or leaves it. The changed symbols and
    /// problems are analysed for the review screen only, which the supervisor's policy follows.
    /// Coming up copies the working tree's dirty content ahead of a review once, since the queue
    /// copied nothing.
    pub(crate) async fn set_review_shown(
        &self,
        project: ProjectId,
        shown: bool,
        from: &Connection,
    ) -> Result<Response> {
        let open: Arc<Open> = self.find(project).await?;
        open.screens.set(from, shown);
        open.policy.screen_changed();
        if shown {
            // The queue copied nothing: the review screen that comes back copies the tree once.
            let copying = open.clone();
            let boot = async move {
                if let Ok(booted) = copying.booted().await {
                    copying.copy_ahead(&booted.session);
                }
            };
            tokio::spawn(boot);
        }
        Ok(Response::Done)
    }

    /// Remembers the changed files the window has open, which the background analysis reads first.
    pub(crate) async fn set_open_files(&self, project: ProjectId, paths: Vec<String>) -> Result<Response> {
        let open: Arc<Open> = self.find(project).await?;
        open.set_open_files(paths);
        Ok(Response::Done)
    }

    /// Analyses the changeset on view's problems again, for the Retry action where a comparison
    /// failed. Only while the review screen shows the changeset, which is what the panel is on.
    pub(crate) async fn refresh_problems(&self, project: ProjectId) -> Result<Response> {
        let open: Arc<Open> = self.find(project).await?;
        open.policy.rerun_problems();
        Ok(Response::Done)
    }
}
