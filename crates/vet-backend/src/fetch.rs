//! Fetching a project's remotes, on demand and on a timer, and telling its
//! subscribers that the branches may have moved.

use std::sync::Arc;
use std::time::Duration;

use vet_app::changeset::Source;
use vet_protocol::{Event, ProjectId, Response};

use crate::accounts::Held;
use crate::changesets::bring_in;
use crate::events::settled;
use crate::open::Booted;
use crate::sync::sync_pull_request;
use crate::{Open, Orchestrator, start_servers_in_background};

impl Orchestrator {
    /// Fetches now. A fetch that fails fails its task and not this request.
    pub(crate) async fn fetch(&self, id: ProjectId) -> Result<Response, String> {
        let open = self.find(id).await?;
        fetch_and_report(&open).await;
        Ok(Response::Done)
    }
}

/// Fetches the remotes, then tells every subscriber that the branches may have moved, and looks
/// at the pull request on view for a push. A remote that
/// failed is named by the failed task, where the footer shows it; what the others brought counts.
async fn fetch_and_report(open: &Arc<Open>) {
    let Ok(ran) = open.fetch.run(&Held(&open.accounts)).await else {
        return;
    };
    if ran.fetched == 0 {
        return;
    }
    let Ok(booted) = open.booted().await else {
        return;
    };
    open.subscribers
        .broadcast(Event::ChangesetsChanged { project: open.id });
    follow_pull_request(open, booted).await;
    sync_pull_request(open, booted).await;
}

/// If a pull request is on view, fetches it again and, when its head or target moved, has the
/// session take the next version of it; when only how the platform names it changed, says so.
async fn follow_pull_request(open: &Arc<Open>, booted: &Booted) {
    let Source::PullRequest { remote, number } = booted.session.source().await else {
        return;
    };
    // The task and the platform's answer say what went wrong; the review on view stays as it is.
    let Ok(renamed) = bring_in(open, &booted.session, &remote, number).await else {
        return;
    };
    if let Ok(Some(revision)) = booted.session.observe().await {
        settled(&open.subscribers, open.id, revision);
        start_servers_in_background(open.clone());
    } else if renamed {
        open.subscribers.broadcast(Event::TargetChanged { project: open.id });
    }
}

/// Fetches now and then every `interval`, for as long as the session is open.
pub(crate) fn fetch_periodically(open: &Arc<Open>, interval: Duration) {
    let session = Arc::downgrade(open);
    let task = tokio::spawn(vet_app::fetch::every(interval, move || {
        let session = session.clone();
        async move {
            if let Some(open) = session.upgrade() {
                fetch_and_report(&open).await;
            }
        }
    }));
    open.keep_fetching(task.abort_handle());
}
