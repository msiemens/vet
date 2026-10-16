//! Fetching a project's remotes, on demand and on a timer, and telling its
//! subscribers that the branches may have moved.

use std::sync::Arc;
use std::time::Duration;

use vet_app::changeset::Source;
use vet_protocol::{Event, ProjectId, Response};

use crate::accounts::Held;
use crate::changesets::bring_in;
use crate::error::Result;
use crate::open::Booted;
use crate::review::revision_changed;
use crate::sync::sync_pull_request;
use crate::{Open, Orchestrator};

impl Orchestrator {
    /// Fetches now. A fetch that fails fails its task and not this request.
    pub(crate) async fn fetch(&self, id: ProjectId) -> Result<Response> {
        let open = self.find(id).await?;
        fetch_and_report(&open, Asked::Viewer).await;
        Ok(Response::Done)
    }
}

/// Who asked for a fetch: what its follow-ups still do when no ref moved.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Asked {
    /// The viewer, through `Request::Fetch`: the fetch is reported whenever a remote was reached.
    Viewer,
    /// The timer: a fetch that moved a ref is announced and followed; either way the comments of
    /// the pull request on view are synced, which change without a push.
    Timer,
}

/// Fetches the remotes, then tells every subscriber that the branches may have moved, follows the
/// pull request on view for a push, and syncs its threads. A remote that failed is named by the
/// failed task, where the footer shows it; what the others brought counts. A timer fetch that
/// moved no ref skips the announcement and the follow but still syncs the threads, which change
/// without a push.
async fn fetch_and_report(open: &Arc<Open>, asked: Asked) {
    let Ok(ran) = open.fetch.run(&Held(&open.accounts)).await else {
        return;
    };
    let moved = match asked {
        Asked::Viewer => ran.fetched > 0,
        Asked::Timer => ran.updated > 0,
    };
    if !moved && asked == Asked::Viewer {
        return;
    }
    let Ok(booted) = open.booted().await else {
        return;
    };
    if moved {
        open.subscribers
            .broadcast(Event::ChangesetsChanged { project: open.id });
        follow_pull_request(open, booted).await;
    }
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
        revision_changed(open, revision);
    } else if renamed {
        open.subscribers.broadcast(Event::TargetChanged { project: open.id });
    }
}

/// Fetches now and then every `interval`, for as long as the session is open.
pub(crate) fn fetch_periodically(open: &Arc<Open>, interval: Duration) {
    let session = Arc::downgrade(open);
    // One tick's work is the background task, not the loop that waits between ticks: a client
    // waiting for quiet must not wait for the loop, which lives as long as the session.
    let fetching = vet_app::fetch::every(interval, move || {
        let session = session.clone();
        async move {
            if let Some(open) = session.upgrade() {
                crate::open::tracked(&open, "fetch", fetch_and_report(&open, Asked::Timer)).await;
            }
        }
    });
    let task = tokio::spawn(fetching);
    open.keep_fetching(task.abort_handle());
}
