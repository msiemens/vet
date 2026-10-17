//! Fetching a project's remotes, on demand and on a timer, and telling its
//! subscribers that the branches may have moved.

use std::sync::{Arc, OnceLock, Weak};

use futures::FutureExt;
use vet_app::changeset::Source;
use vet_app::fetch::Started;
use vet_app::supervise::Supervisor;
use vet_protocol::{Event, ProjectId, Response};

use crate::changesets::bring_in;
use crate::error::Result;
use crate::events::snapshot_moved;
use crate::hosting::Held;
use crate::open::Booted;
use crate::supervisor::{Asked, Fetches, Reason, Sealing};
use crate::{Open, Orchestrator};

impl Orchestrator {
    /// Fetches now, joining a fetch under way, then waits for the pull request feed to list again.
    /// A fetch that fails fails its task and not this request.
    pub(crate) async fn fetch(&self, id: ProjectId) -> Result<Response> {
        let open = self.find(id).await?;
        open.fetches.fetch(Asked::Viewer).await;
        open.feed().manual().await;
        Ok(Response::Done)
    }
}

/// The kind of the project's fetches in its supervisor.
const KIND: &str = "fetch";

/// The fetches of a project, reaching it through `entry` so a fetch that waits keeps nothing open.
pub(crate) fn fetches(supervisor: &Supervisor, entry: Arc<OnceLock<Weak<Open>>>) -> Fetches {
    Fetches::new(supervisor.clone(), KIND, move |sealing| {
        let entry = entry.clone();
        async move {
            if let Some(open) = entry.get().and_then(Weak::upgrade) {
                fetch_and_report(&open, sealing).await;
            }
        }
        .boxed()
    })
}

/// Fetches the remotes, then tells every subscriber that the branches may have moved, follows the
/// pull request on view for a push, and syncs its threads. A timer fetch takes only the remotes on
/// platform hosts with an account; the viewer's takes every remote. A remote that failed is named
/// by the failed task; what the others brought counts. A timer fetch that
/// moved no ref skips the announcement and the follow but still syncs the threads, which change
/// without a push, and has the feed list again. Cooperates with the project closing: it ends
/// before its next step instead.
async fn fetch_and_report(open: &Arc<Open>, sealing: Sealing) {
    let started = match sealing.started() {
        Asked::Timer => Started::Background,
        Asked::Viewer => Started::Viewer,
    };
    let ran = open.fetch.run(&Held(&open.accounts), started).await;
    let asked = sealing.asked();
    if let Ok(ran) = ran {
        follow_fetch(open, asked, ran.fetched, ran.updated).await;
    }
    if asked == Asked::Timer {
        open.feed().timer();
    }
}

async fn follow_fetch(open: &Arc<Open>, asked: Asked, fetched: usize, updated: usize) {
    let moved = match asked {
        Asked::Viewer => fetched > 0,
        Asked::Timer => updated > 0,
    };
    if !moved && asked == Asked::Viewer {
        return;
    }
    let Ok(booted) = open.booted_unless_failed().await else {
        return;
    };
    if open.supervisor.closed() {
        return;
    }
    if moved {
        open.subscribers
            .broadcast(Event::ChangesetsChanged { project: open.id });
        follow_pull_request(open, booted).await;
    }
    if !open.supervisor.closed() {
        open.sync.request(vec![Reason::Fetched]).wait().await;
    }
}

/// If a pull request is on view, fetches it again and, when its head or target moved, has the
/// session take the next version of it; when only how the platform names it changed, says so.
async fn follow_pull_request(open: &Arc<Open>, booted: &Booted) {
    let Source::PullRequest { remote, number } = booted.session.source().await else {
        return;
    };
    // The task and the platform's answer say what went wrong; the review on view stays as it is.
    let Ok(brought) = bring_in(open, &booted.session, &remote, number).await else {
        return;
    };
    let renamed = brought.renamed;
    if let Ok(Some(revision)) = booted.session.observe().await {
        snapshot_moved(&open.subscribers, open.id, revision, booted.session.review_version());
    } else if renamed {
        open.subscribers.broadcast(Event::TargetChanged { project: open.id });
    }
}
