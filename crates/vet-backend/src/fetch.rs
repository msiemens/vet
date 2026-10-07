//! Fetching a project's remotes, on demand and on a timer, and telling its
//! subscribers how the base stands afterwards.

use std::sync::Arc;
use std::time::Duration;

use vet_app::fetch::Credentials;
use vet_app::platform::Credential;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};

use crate::{Open, Orchestrator, PlatformAccounts};

/// The accounts the process holds, as the credentials a fetch uses.
struct Held<'a>(&'a PlatformAccounts);

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

/// Fetches the remotes, then tells every subscriber how far the base's branch has moved on.
async fn fetch_and_report(open: &Open) {
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
