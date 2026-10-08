//! Fetching a project's remotes, on demand and on a timer, and telling its
//! subscribers how the base stands afterwards.

use std::sync::Arc;
use std::time::Duration;

use vet_app::changeset::Source;
use vet_app::fetch::Credentials;
use vet_app::platform::Credential;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};

use crate::changesets::bring_in;
use crate::events::settled;
use crate::open::Booted;
use crate::sync::sync_pull_request;
use crate::{Open, Orchestrator, PlatformAccounts, start_servers_in_background};

/// The accounts the process holds, as the credentials a fetch uses.
pub(crate) struct Held<'a>(pub(crate) &'a PlatformAccounts);

impl Credentials for Held<'_> {
    async fn credential(&self, host: &str) -> Option<Credential> {
        self.0.credential(host, unix_now()).await
    }
}

impl Orchestrator {
    /// Fetches now. A fetch that fails fails its task and not this request.
    pub(crate) async fn fetch(&self, id: ProjectId) -> Result<Response, String> {
        let open = self.find(id).await?;
        fetch_and_report(&open).await;
        Ok(Response::Done)
    }
}

/// Fetches the remotes, then tells every subscriber how far the base's branch has moved on, and
/// looks at the pull request on view for a push.
async fn fetch_and_report(open: &Arc<Open>) {
    // A failure is on the task, where the footer shows it.
    if open.fetch.run(&Held(&open.accounts)).await != Ok(true) {
        return;
    }
    let Ok(booted) = open.booted().await else {
        return;
    };
    if let Ok(base) = booted.session.base().await {
        open.subscribers.broadcast(Event::BaseChanged {
            project: open.id,
            base: mapping::base(&base),
        });
    }
    follow_pull_request(open, booted).await;
    sync_pull_request(open, booted).await;
}

/// If a pull request is on view, fetches it again and, when its head or target moved, has the
/// session take the next version of it.
async fn follow_pull_request(open: &Arc<Open>, booted: &Booted) {
    let Source::PullRequest { remote, number } = booted.session.source().await else {
        return;
    };
    // The task and the platform's answer say what went wrong; the review on view stays as it is.
    if bring_in(open, &remote, number).await.is_err() {
        return;
    }
    if let Ok(Some(revision)) = booted.session.observe().await {
        settled(&open.subscribers, open.id, revision);
        start_servers_in_background(open.clone());
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
