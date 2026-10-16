//! A project's pull request feed as the backend wires it: the platform side over the process's
//! platforms and accounts, the project side over a weak handle on the open project, and the task
//! that runs it for as long as the project is open.

use std::sync::{Arc, OnceLock, Weak};

use vet_app::changeset::{RemoteKey, Source};
use vet_app::fetch::{Credentials, Fetch};
use vet_app::platform::{Platform, PlatformKind, PullRequestCache, Standing};
use vet_app::pull_request::PullRequestList;
use vet_app::pull_request_feed::{self, FeedEvent, Hostings, Prefetch, Project};
use vet_app::session::SessionError;
use vet_infra::clock::unix_now;
use vet_infra::git::GixRepo;
use vet_infra::store::SqliteStore;
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId};

use crate::background::{Background, LiveTask};
use crate::events::revision_changed;
use crate::hosting::{Held, Platforms};
use crate::{Open, Orchestrator, PlatformAccounts};

/// A project's pull request feed, as the backend wires it.
pub(crate) type Feed = pull_request_feed::PullRequestFeed<FeedPlatforms, SqliteStore, FeedProject>;

/// The platform side of the pull request cycle, as the backend has it: the platform clients and
/// their health, the accounts, and the project's refs.
#[derive(Clone)]
pub(crate) struct FeedPlatforms {
    platforms: Arc<Platforms>,
    accounts: Arc<PlatformAccounts>,
    fetch: Arc<Fetch<GixRepo>>,
}

impl Hostings for FeedPlatforms {
    type Fetcher = GixRepo;

    fn hosting(&self, kind: PlatformKind) -> impl Platform + Send {
        self.platforms.hosting(kind)
    }

    async fn has_account(&self, host: &str) -> bool {
        self.accounts.has_account(host, unix_now()).await
    }

    fn standing(&self, account: &str) -> Option<Standing> {
        self.platforms.standing(account)
    }

    fn known(&self) -> Arc<PullRequestCache> {
        self.platforms.known()
    }

    fn fetch(&self) -> &Fetch<GixRepo> {
        &self.fetch
    }

    fn credentials(&self) -> impl Credentials + Send + Sync + '_ {
        Held(&self.accounts)
    }
}

/// The project side of the pull request cycle, as the backend has it: the review session on view,
/// which the feed reaches through a weak handle so the project is not kept open by its own feed,
/// the window it tells, and the task counter `WaitForQuiet` reads.
#[derive(Clone)]
pub(crate) struct FeedProject {
    entry: Arc<OnceLock<Weak<Open>>>,
    counting: Arc<Background>,
    id: ProjectId,
}

impl FeedProject {
    /// The open project, if it still is.
    fn open(&self) -> Option<Arc<Open>> {
        self.entry.get().and_then(Weak::upgrade)
    }
}

impl Project for FeedProject {
    type Activity = LiveTask;

    fn activity(&self, kind: &'static str) -> LiveTask {
        self.counting.start(self.id, kind)
    }

    async fn on_view(&self) -> Option<(RemoteKey, u64)> {
        let open = self.open()?;
        let booted = open.booted().await.ok()?;
        match booted.session.source().await {
            Source::PullRequest { remote, number } => Some((remote, number)),
            _ => None,
        }
    }

    async fn prunable(
        &self,
        unlisted: Vec<(RemoteKey, u64)>,
    ) -> std::result::Result<Vec<(RemoteKey, u64)>, SessionError> {
        // The listing the cycle made booted the session, so this cannot fail.
        let Some(open) = self.open() else {
            return Ok(Vec::new());
        };
        let Ok(booted) = open.booted().await else {
            return Ok(Vec::new());
        };
        booted.session.prunable(unlisted).await
    }

    async fn heads_fetched(&self, list: &PullRequestList) {
        let Some((remote, number)) = self.on_view().await else {
            return;
        };
        if !list
            .rows
            .iter()
            .any(|listed| listed.remote == remote && listed.row.number == number)
        {
            return;
        }
        let Some(open) = self.open() else {
            return;
        };
        let Ok(booted) = open.booted().await else {
            return;
        };
        if let Ok(Some(revision)) = booted.session.observe().await {
            revision_changed(&open, revision);
        }
    }

    fn announce(&self, event: FeedEvent) {
        let Some(open) = self.open() else {
            return;
        };
        match event {
            FeedEvent::ChangesetsChanged => {
                open.subscribers
                    .broadcast(Event::ChangesetsChanged { project: open.id });
            }
            FeedEvent::PullRequestsChanged(changed) => {
                let changed = changed
                    .into_iter()
                    .map(|c| mapping::pull_request_update(&c.remote, c.number, &c.added))
                    .collect();
                open.subscribers.broadcast(Event::PullRequestsChanged {
                    project: open.id,
                    changed,
                });
            }
            FeedEvent::ListChanged => {
                open.subscribers
                    .broadcast(Event::PullRequestListChanged { project: open.id });
            }
        }
    }
}

/// Runs the project's pull request feed for as long as it is open: one task, stopped when the
/// project closes.
pub(crate) fn run_feed(open: &Arc<Open>) {
    let task = tokio::spawn(open.feed().clone().run());
    open.keep_feeding(task.abort_handle());
}

impl Orchestrator {
    /// The pull request feed of a project: over the platforms and accounts the backend holds, the
    /// project's refs, and a weak handle on its session.
    pub(crate) fn pull_request_feed(
        &self,
        id: ProjectId,
        fetch: Arc<Fetch<GixRepo>>,
        entry: Arc<OnceLock<Weak<Open>>>,
    ) -> Feed {
        Feed::new(
            FeedPlatforms {
                platforms: self.platforms.clone(),
                accounts: self.accounts.clone(),
                fetch,
            },
            self.store.clone(),
            FeedProject {
                entry,
                counting: self.background.clone(),
                id,
            },
            unix_now,
            Prefetch {
                pace: self.config.prefetch_pace,
                announce: self.config.prefetch_announce,
            },
        )
    }
}
