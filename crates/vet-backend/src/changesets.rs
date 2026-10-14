//! Requests about which changeset is on view and which there are.

use std::sync::Arc;
use std::time::Duration;

use vet_app::changeset::{RemoteKey, Source};
use vet_app::fetch::FetchMark;
use vet_app::platform::{Platform, PlatformError, PlatformKind, PullRequestCache, Standing, commits_checks};
use vet_app::ports::{ChangesetStore, LiveStore, ReviewWrites};
use vet_app::pull_request::{self, FetchError, PullRequestList, hosted_repositories, hosts_to_ask_about, on_github};
use vet_app::session::RowReview;
use vet_infra::clock::unix_now;
use vet_infra::store::SqliteStore;
use vet_mapping as mapping;
use vet_protocol::{ChangesetKey, Event, ProjectId, Response};

use crate::accounts::Held;
use crate::error::{Error, Result};
use crate::hosting::{Hosting, Platforms};
use crate::open::Ready;
use crate::sync::sync_in_background;
use crate::{Live, Open, Orchestrator, start_servers_in_background};

impl Orchestrator {
    pub(crate) async fn list_changesets(&self, project: ProjectId) -> Result<Response> {
        let list = self.asking(project, async |session| session.changesets().await).await?;
        Ok(mapping::changesets(&list))
    }

    pub(crate) async fn list_pull_requests(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        let remotes = open.open.fetch.remotes().await?;
        let candidates = hosted_repositories(&remotes, PlatformKind::GitHub);
        let mut hosts_with_account = Vec::new();
        for host in hosts_to_ask_about(&candidates) {
            if self.accounts.has_account(&host, unix_now()).await {
                hosts_with_account.push(host);
            }
        }
        let on_github = on_github(candidates.clone(), &hosts_with_account);
        for skipped in candidates.iter().filter(|c| !on_github.contains(c)) {
            tracing::warn!(
                remote = %skipped.remote,
                host = %skipped.key.host,
                "not listing pull requests: the host is not github.com and has no account, so it is not known to be GitHub"
            );
        }
        let github: Vec<_> = on_github.into_iter().map(|h| h.key).collect();
        let azure_devops: Vec<_> = hosted_repositories(&remotes, PlatformKind::AzureDevOps)
            .into_iter()
            .map(|h| h.key)
            .collect();
        if github.is_empty() && azure_devops.is_empty() {
            let names: Vec<_> = remotes.iter().map(|r| r.name.as_str()).collect();
            tracing::warn!(
                ?names,
                "not listing pull requests: no remote of the repository is a hosted repository"
            );
        }
        let on_view = match open.session.source().await {
            Source::PullRequest { remote, number } => Some((remote, number)),
            _ => None,
        };
        let on_view = on_view.as_ref().map(|(remote, number)| (remote, *number));
        let since = open.open.fetch.mark();
        let list = pull_request::list_pull_requests(&self.platforms.hosting(PlatformKind::GitHub), &github, on_view)
            .await
            .and(
                pull_request::list_pull_requests(
                    &self.platforms.hosting(PlatformKind::AzureDevOps),
                    &azure_devops,
                    on_view,
                )
                .await,
            );
        if let Err(error) = prune_pull_request_refs(&open, &list, since).await {
            tracing::warn!(%error, "deleting the refs of pull requests no longer listed failed");
        }
        fetch_heads_in_background(open.open.clone(), list.clone());
        prefetch_details(
            self.platforms.clone(),
            self.store.clone(),
            self.platforms.known(),
            list.clone(),
            self.config.prefetch_pace,
        );
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

    pub(crate) async fn changeset_lines(&self, project: ProjectId, changeset: ChangesetKey) -> Result<Response> {
        let source = mapping::core_changeset_key(changeset).map_err(Error::Invalid)?;
        let lines = self
            .asking(project, async |session| session.lines_of(&source).await)
            .await?;
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
        let hosting = self.platforms.hosting(remote.kind());
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
        let hosting = self.platforms.hosting(remote.kind());
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

/// How many of the newest listed pull requests have their details fetched ahead of the queue:
/// enough that the ones the viewer is likely to open are ready, few enough to stay gentle on the
/// platform.
const PREFETCH_LIMIT: usize = 20;

/// Fetches the details of the newest listed pull requests whose `updatedAt` the stored copy does
/// not have, in the background, so a revisit shows them at once. An unchanged pull request is not
/// asked for. The platform is asked `pace` between two; a list whose account is limited or failing
/// stops there. Spawned, so it never delays the list the viewer asked for.
fn prefetch_details(
    platforms: Arc<Platforms>,
    store: SqliteStore,
    known: Arc<PullRequestCache>,
    list: PullRequestList,
    pace: Duration,
) {
    tokio::spawn(async move {
        let mut stopped: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut fetched = 0usize;
        for listed in &list.rows {
            if fetched >= PREFETCH_LIMIT {
                break;
            }
            let account = listed.remote.account();
            if stopped.contains(&account) {
                continue;
            }
            if matches!(platforms.standing(&account), Some(Standing::Failing(_))) {
                stopped.insert(account);
                continue;
            }
            let stored = store.stored_discussion(listed.remote.clone(), listed.row.number).await;
            if let Ok(Some(stored)) = &stored
                && stored.updated_at == listed.row.updated_at
            {
                // Nothing changed since the copy that is kept: no call is made.
                continue;
            }
            fetched += 1;
            let hosting = platforms.hosting(listed.remote.kind());
            match hosting.discussion(&listed.remote, listed.row.number).await {
                Ok(discussion) => {
                    let at = unix_now();
                    known.record_discussion(&listed.remote, listed.row.number, at, discussion.clone());
                    if let Err(error) = store
                        .writes()
                        .set_discussion(
                            listed.remote.clone(),
                            listed.row.number,
                            listed.row.head.clone(),
                            listed.row.updated_at,
                            at,
                            &discussion,
                        )
                        .await
                    {
                        tracing::warn!(error = %error.0, "storing a prefetched pull request's details failed");
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        repository = %listed.remote,
                        number = listed.row.number,
                        %error,
                        "prefetching a pull request's details failed"
                    );
                    stopped.insert(account);
                }
            }
            tokio::time::sleep(pace).await;
        }
    });
}

/// Brings in the pull requests of `list` that are not in the repository, so the queue can count
/// the lines of every row, and tells the subscribers when something came.
fn fetch_heads_in_background(open: Arc<Open>, list: PullRequestList) {
    tokio::spawn(async move {
        match pull_request::fetch_listed_heads(&open.fetch, &Held(&open.accounts), &list).await {
            Ok(true) => open
                .subscribers
                .broadcast(Event::ChangesetsChanged { project: open.id }),
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "fetching the listed pull requests failed"),
        }
    });
}

/// Deletes the refs of the pull requests fetched before `since` that `list` no longer has, unless
/// one is on view or a review of it is open: those may need their commits before the platform can
/// be asked again (a closed pull request's branch may be gone).
async fn prune_pull_request_refs(open: &Ready, list: &PullRequestList, since: FetchMark) -> Result<()> {
    let unlisted = pull_request::unlisted_fetched(&open.open.fetch, list).await?;
    if unlisted.is_empty() {
        return Ok(());
    }
    let sources: Vec<Source> = unlisted
        .iter()
        .map(|(remote, number)| Source::PullRequest {
            remote: remote.clone(),
            number: *number,
        })
        .collect();
    let reviews = open.session.reviews_of(&sources).await?;
    let on_view = open.session.source().await;
    let doomed: Vec<_> = unlisted
        .into_iter()
        .zip(sources.iter().zip(reviews))
        .filter(|(_, (source, review))| **source != on_view && !matches!(review, RowReview::Open { .. }))
        .map(|(pull_request, _)| pull_request)
        .collect();
    let deleted = pull_request::delete_pull_request_refs(&open.open.fetch, &doomed, since).await?;
    if !deleted.is_empty() {
        tracing::info!(
            refs = deleted.len(),
            "deleted the refs of pull requests no longer listed"
        );
    }
    Ok(())
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
    let brought =
        pull_request::fetch_pull_request(&open.hosting(remote.kind()), &open.fetch, &held, remote, number).await?;
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
