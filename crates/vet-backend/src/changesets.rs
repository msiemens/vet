//! Requests about which changeset is on view and which there are.

use std::sync::Arc;

use futures::StreamExt;
use futures::stream::FuturesOrdered;
use vet_app::changeset::{RemoteKey, Source};
use vet_app::fetch::{Credentials, Fetch};
use vet_app::platform::{Platform, PlatformError, PlatformKind, PullRequestCache, Standing, commits_checks};
use vet_app::pull_request::{self, FetchError, PullRequestList};
use vet_app::pull_request_feed::{self, Background, FeedEvent, Hostings, Prefetch, Project};
use vet_app::session::SessionError;
use vet_infra::clock::unix_now;
use vet_infra::git::GixRepo;
use vet_mapping as mapping;
use vet_protocol::{ChangesetKey, Event, ProjectId, Response};

use crate::accounts::Held;
use crate::checks::refresh_checks_after_action;
use crate::error::{Error, Result};
use crate::hosting::{Hosting, Platforms};
use crate::review::revision_changed;
use crate::sync::sync_in_background;
use crate::{Live, Open, Orchestrator, PlatformAccounts, start_servers_in_background};

/// How many rows of the queue have their lines counted at once: as many as the review lane's
/// blocking pool runs, so the pool stays busy without rows queueing behind each other.
const LINES_AT_ONCE: usize = vet_infra::budget::REVIEW_IO;

impl Orchestrator {
    pub(crate) async fn list_changesets(&self, project: ProjectId) -> Result<Response> {
        let list = self.asking(project, async |session| session.changesets().await).await?;
        Ok(mapping::changesets(&list))
    }

    pub(crate) async fn list_pull_requests(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        let platforms = FeedPlatforms {
            platforms: self.platforms.clone(),
            accounts: self.accounts.clone(),
            open: open.open.clone(),
        };
        let list = pull_request_feed::refresh(
            &platforms,
            self.store.clone(),
            &FeedProject(open.open.clone()),
            &FeedBackground(open.open.clone()),
            unix_now,
            Prefetch {
                pace: self.config.prefetch_pace,
                announce: self.config.prefetch_announce,
            },
        )
        .await?;
        for source in &list.sources {
            if let Some(error) = &source.error {
                // The client told of a rate limit once, when it began.
                if matches!(error, PlatformError::RateLimited { .. }) {
                    tracing::debug!(repository = %source.remote, %error, "listing the pull requests failed");
                } else {
                    tracing::warn!(repository = %source.remote, %error, "listing the pull requests failed");
                }
            } else {
                let listed = list.rows.iter().filter(|r| r.remote == source.remote).count();
                tracing::debug!(repository = %source.remote, listed, truncated = source.truncated, "listed pull requests");
            }
        }
        let sources: Vec<_> = list
            .rows
            .iter()
            .map(|listed| mapping::pull_request_source(&listed.remote, listed.row.number))
            .collect();
        let reviews = open.session.reviews_of(&sources).await?;
        Ok(mapping::pull_requests(&list, &reviews))
    }

    pub(crate) async fn changeset_lines(&self, project: ProjectId, changesets: Vec<ChangesetKey>) -> Result<Response> {
        let sources = changesets
            .iter()
            .map(|c| mapping::core_changeset_key(c.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Invalid)?;
        let open = self.ready(project).await?;
        let mut lines = Vec::with_capacity(sources.len());
        // A few rows are counted at once, so a long queue is counted in a fraction of the time
        // one after another takes; more at once than the review pool's blocking threads would
        // only queue behind them. The pushes stay in the request's order: a row's count is held
        // until the rows before it are pushed, so the client's list fills top first.
        let mut pending = sources.iter().zip(&changesets);
        let mut counted = FuturesOrdered::new();
        loop {
            while counted.len() < LINES_AT_ONCE {
                let Some((source, _)) = pending.next() else { break };
                counted.push_back(open.session.lines_of(source));
            }
            let Some(gotten) = counted.next().await else { break };
            let gotten = gotten?;
            open.subscribers.broadcast(mapping::changeset_lines_changed(
                project,
                vec![changesets[lines.len()].clone()],
                vec![gotten],
            ));
            lines.push(gotten);
        }
        Ok(mapping::changeset_lines(lines))
    }

    /// The repository of the pull request on view, if one is.
    async fn hosted_pull_request(&self, project: ProjectId) -> Result<Option<RemoteKey>> {
        let open = self.ready(project).await?;
        Ok(match open.session.source().await {
            Source::PullRequest { remote, .. } => Some(remote),
            _ => None,
        })
    }

    pub(crate) async fn checks(&self, project: ProjectId, commits: Vec<String>) -> Result<Response> {
        let Some(remote) = self.hosted_pull_request(project).await? else {
            return Ok(mapping::checks(&[], None));
        };
        let hosting = self.platforms.hosting(self.kind_of(&remote).await?);
        let (found, error) = commits_checks(&hosting, &remote, &commits).await;
        Ok(mapping::checks(&found, error.as_ref()))
    }

    /// The repository of the pull request on view and the client of its platform; fails if the
    /// changeset on view is no pull request.
    async fn hosted_pull_request_client(&self, project: ProjectId) -> Result<(RemoteKey, Hosting)> {
        let remote = self
            .hosted_pull_request(project)
            .await?
            .ok_or_else(Error::not_a_pull_request)?;
        let hosting = self.platforms.hosting(self.kind_of(&remote).await?);
        Ok((remote, hosting))
    }

    pub(crate) async fn check_log(&self, project: ProjectId, run: String) -> Result<Response> {
        let (remote, hosting) = self.hosted_pull_request_client(project).await?;
        let log = hosting.check_log(&remote, &run).await?;
        Ok(mapping::check_log(&log))
    }

    pub(crate) async fn pipeline_steps(&self, project: ProjectId, check: String) -> Result<Response> {
        let (remote, hosting) = self.hosted_pull_request_client(project).await?;
        let steps = hosting.steps(&remote, &check).await?;
        Ok(mapping::pipeline_steps(&steps))
    }

    pub(crate) async fn change_pipeline(
        &self,
        project: ProjectId,
        action: vet_protocol::PipelineAction,
    ) -> Result<Response> {
        let (remote, hosting) = self.hosted_pull_request_client(project).await?;
        let done = hosting
            .pipeline_action(&remote, &mapping::pipeline_action(action))
            .await;
        if done.is_ok() {
            // What the platform makes of the action shows in the checks a moment later; read again
            // now and once more shortly after, without waiting for the window to ask.
            refresh_checks_after_action(self.find(project).await?);
        }
        Ok(mapping::pipeline_outcome(done)?)
    }

    pub(crate) async fn open_changeset(&self, project: ProjectId, changeset: ChangesetKey) -> Result<Response> {
        let open = self.ready(project).await?;
        let source = mapping::core_changeset_key(changeset).map_err(Error::Invalid)?;
        let mut renamed = false;
        if let Source::PullRequest { remote, number } = &source {
            renamed = bring_in(&open.open, &open.session, remote, *number)
                .await
                .map_err(|e| Error::from(e).context(format!("cannot fetch pull request #{number}")))?;
        }
        let moved = open.session.open_changeset(source.clone()).await?;
        if let Some(revision) = moved {
            open.subscribers.broadcast(Event::ChangesetOpened {
                project,
                revision,
                changeset: mapping::changeset_key(&source),
            });
            start_servers_in_background(open.open.clone());
        } else if renamed {
            open.subscribers.broadcast(Event::TargetChanged { project });
        }
        if matches!(source, Source::PullRequest { .. }) {
            sync_in_background(open.open.clone());
        }
        Ok(Response::Done)
    }
}

/// The platform side of the pull request cycle, as the backend has it: the platform clients and
/// their health, the accounts, and the project's refs.
#[derive(Clone)]
struct FeedPlatforms {
    platforms: Arc<Platforms>,
    accounts: Arc<PlatformAccounts>,
    open: Arc<Open>,
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
        &self.open.fetch
    }

    fn credentials(&self) -> impl Credentials + Send + Sync + '_ {
        Held(&self.accounts)
    }
}

/// The project side of the pull request cycle, as the backend has it: the review session on view
/// and the window it tells.
#[derive(Clone)]
struct FeedProject(Arc<Open>);

impl Project for FeedProject {
    async fn on_view(&self) -> Option<(RemoteKey, u64)> {
        let booted = self.0.booted().await.ok()?;
        match booted.session.source().await {
            Source::PullRequest { remote, number } => Some((remote, number)),
            _ => None,
        }
    }

    async fn prunable(
        &self,
        unlisted: Vec<(RemoteKey, u64)>,
    ) -> std::result::Result<Vec<(RemoteKey, u64)>, SessionError> {
        // The request that listed the pull requests booted the session, so this cannot fail.
        let Ok(booted) = self.0.booted().await else {
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
        let Ok(booted) = self.0.booted().await else {
            return;
        };
        if let Ok(Some(revision)) = booted.session.observe().await {
            revision_changed(&self.0, revision);
        }
    }

    fn announce(&self, event: FeedEvent) {
        match event {
            FeedEvent::ChangesetsChanged => {
                self.0
                    .subscribers
                    .broadcast(Event::ChangesetsChanged { project: self.0.id });
            }
            FeedEvent::PullRequestsChanged(changed) => {
                let changed = changed
                    .into_iter()
                    .map(|c| mapping::pull_request_update(&c.remote, c.number, &c.added))
                    .collect();
                self.0.subscribers.broadcast(Event::PullRequestsChanged {
                    project: self.0.id,
                    changed,
                });
            }
            // The per-request listing answers its own caller, so it never publishes a whole list.
            FeedEvent::ListChanged => {}
        }
    }
}

/// Where the pull request cycle starts its background work, as the backend starts it: the listed
/// heads as a bare task, the details fetched ahead as the project's replaceable prefetch.
#[derive(Clone)]
struct FeedBackground(Arc<Open>);

impl Background for FeedBackground {
    fn fetch_heads(&self, work: impl Future<Output = ()> + Send + 'static) {
        tokio::spawn(work);
    }

    fn prefetch(&self, work: impl Future<Output = ()> + Send + 'static) {
        let task = tokio::spawn(crate::open::tracked(&self.0, "prefetch", work));
        self.0.keep_prefetching(task.abort_handle());
    }
}

/// Fetches the head of pull request `number` of `remote`, its target branch and its earlier
/// pushes, tells the session the pushes, and stores how the platform names it. Answers whether
/// that is not what was stored.
pub(crate) async fn bring_in(
    open: &Open,
    session: &Live,
    remote: &RemoteKey,
    number: u64,
) -> std::result::Result<bool, FetchError> {
    let held = Held(&open.accounts);
    let hosting = open
        .hosting_for(remote)
        .await
        .map_err(|error| FetchError::NoRemote(error.to_string()))?;
    let brought = pull_request::fetch_pull_request(&hosting, &open.fetch, &held, remote, number).await?;
    let source = Source::PullRequest {
        remote: remote.clone(),
        number,
    };
    session.remember_pushes(&source, brought.pushes);
    // The fetch is done; a title that cannot be stored is shown again after the next one.
    Ok(session
        .remember_pull_request_head(&source, brought.head)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "storing the title of a pull request failed");
            false
        }))
}
