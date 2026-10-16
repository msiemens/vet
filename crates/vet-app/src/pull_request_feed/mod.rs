//! The pull request feed's cycle: listing a project's repositories' pull requests, pruning the
//! refs of the ones a listing no longer has, fetching the listed heads and prefetching the
//! details. It works over two ports: the platform side, which reaches the hosting platforms and
//! the project's refs, and the project side, which is the session on view and the window it tells.
//! The phases that must not delay the list are handed to [`Background`].

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::fetch::{Credentials, Fetch, FetchMark};
use crate::platform::{Added, Platform, PlatformKind, Platforms, PullRequestCache, RemoteKey, Standing};
use crate::ports::{Fetcher, LiveStore, ReviewWrites};
use crate::pull_request::{
    PullRequestList, delete_pull_request_refs, fetch_listed_heads, hosted_repositories, hosts_to_ask_about,
    list_pull_requests, unlisted_fetched,
};
use crate::session::{SessionError, UnixClock};

/// How many of the newest listed pull requests of a platform whose list already carries their
/// details have their discussion fetched ahead of the queue: enough that the ones the viewer is
/// likely to open are ready, few enough to stay gentle on the platform. A platform whose list
/// leaves the details apart (`PlatformTraits::details_apart`) has no such cap: without them the
/// rows cannot show their state at all.
const PREFETCH_LIMIT: usize = 20;

/// How a prefetch run is paced, and how often it tells the window.
#[derive(Clone, Copy, Debug)]
pub struct Prefetch {
    /// How long between two pull requests asked.
    pub pace: Duration,
    /// How long between two announcements that the details changed the list.
    pub announce: Duration,
}

/// A pull request whose details, read again, changed what its row of the queue shows.
#[derive(Clone, Debug)]
pub struct ChangedPullRequest {
    pub remote: RemoteKey,
    pub number: u64,
    /// Where it stands, its unresolved threads and who asked for changes, as the details say.
    pub added: Added,
}

/// What the cycle tells the project's window.
#[derive(Clone, Debug)]
pub enum FeedEvent {
    /// The details read ahead changed what the listed rows show.
    PullRequestsChanged(Vec<ChangedPullRequest>),
    /// The listed heads brought a new revision or branch.
    ChangesetsChanged,
}

/// The platform side of the feed: the clients of the hosting platforms, the viewer's accounts,
/// how the platforms are doing, and the project's refs.
pub trait Hostings {
    /// The project's git remotes and refs.
    type Fetcher: Fetcher + Send + Sync;

    /// The client of the platform `kind`.
    fn hosting(&self, kind: PlatformKind) -> impl Platform + Send;
    /// Whether the viewer has an account for `host`, which decides whether a host Vet does not
    /// know by name is GitHub.
    fn has_account(&self, host: &str) -> impl Future<Output = bool> + Send;
    /// How the last call to the platform of `account` went, if there was one.
    fn standing(&self, account: &str) -> Option<Standing>;
    /// What the platforms last said of pull requests.
    fn known(&self) -> Arc<PullRequestCache>;
    /// The refs of the project, which the cycle reads and deletes.
    fn fetch(&self) -> &Fetch<Self::Fetcher>;
    /// The credentials a fetch of the project's remotes presents.
    fn credentials(&self) -> impl Credentials + Send + Sync + '_;
}

/// The project side of the feed: the review session on view and the window it tells.
pub trait Project {
    /// The pull request on view, if the changeset on view is one.
    fn on_view(&self) -> impl Future<Output = Option<(RemoteKey, u64)>> + Send;
    /// Of `unlisted` (the refs of pull requests a listing no longer has), those whose refs may be
    /// dropped: the one on view and those whose review is open keep theirs.
    fn prunable(
        &self,
        unlisted: Vec<(RemoteKey, u64)>,
    ) -> impl Future<Output = Result<Vec<(RemoteKey, u64)>, SessionError>> + Send;
    /// Takes in a push the listed heads brought: looks at the pull request on view again, and moves
    /// the revision and starts the servers when it did, as a fetch does.
    fn heads_fetched(&self, list: &PullRequestList) -> impl Future<Output = ()> + Send;
    /// Tells the project's window of a change the cycle made.
    fn announce(&self, event: FeedEvent);
}

/// Where the cycle starts the work that must not delay the list it answers: the heads of the
/// listed pull requests, and the details fetched ahead.
pub trait Background {
    /// Runs `work` for the listed heads, which nothing waits for. An earlier run is not stopped.
    fn fetch_heads(&self, work: impl Future<Output = ()> + Send + 'static);
    /// Runs `work` for the details fetched ahead, replacing an earlier run.
    fn prefetch(&self, work: impl Future<Output = ()> + Send + 'static);
}

/// One pass of the feed's cycle over the ports, in order: list the repositories' pull requests,
/// prune the refs of the ones no longer listed, fetch the listed heads and prefetch the details.
/// The two phases that must not delay the list are handed to `background`. Answers with the list.
pub async fn refresh<H, S, P, B>(
    platforms: &H,
    store: S,
    project: &P,
    background: &B,
    now: UnixClock,
    timing: Prefetch,
) -> Result<PullRequestList, SessionError>
where
    H: Hostings + Clone + Send + Sync + 'static,
    S: LiveStore + Clone + Send + Sync + 'static,
    P: Project + Clone + Send + Sync + 'static,
    B: Background + Sync,
{
    let fetch = platforms.fetch();
    let remotes = fetch.remotes().await?;
    // The platforms registry: the hosts Vet knows by name, and the hosts the viewer has an account
    // for (a GitHub Enterprise installation). A host that is neither is not listed.
    let mut known_hosts = Platforms::new();
    for host in hosts_to_ask_about(&remotes) {
        if platforms.has_account(&host).await {
            known_hosts = known_hosts.with_account(&host);
        } else {
            tracing::warn!(
                host = %host,
                "not listing pull requests: the host is not one Vet knows and has no account, so it is not known to be a platform"
            );
        }
    }
    let github: Vec<_> = hosted_repositories(&remotes, &known_hosts, PlatformKind::GitHub)
        .into_iter()
        .map(|h| h.key)
        .collect();
    let azure_devops: Vec<_> = hosted_repositories(&remotes, &known_hosts, PlatformKind::AzureDevOps)
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
    let on_view = project.on_view().await;
    let listing_on_view = on_view.as_ref().map(|(remote, number)| (remote, *number));
    let since = fetch.mark();
    let list = list_pull_requests(&platforms.hosting(PlatformKind::GitHub), &github, listing_on_view)
        .await
        .and(
            list_pull_requests(
                &platforms.hosting(PlatformKind::AzureDevOps),
                &azure_devops,
                listing_on_view,
            )
            .await,
        );
    if let Err(error) = prune(platforms, project, &list, since).await {
        tracing::warn!(%error, "deleting the refs of pull requests no longer listed failed");
    }
    background.fetch_heads({
        let platforms = platforms.clone();
        let project = project.clone();
        let list = list.clone();
        async move { fetch_heads(&platforms, &project, &list).await }
    });
    background.prefetch({
        let platforms = platforms.clone();
        let project = project.clone();
        let list = list.clone();
        async move { prefetch(&platforms, &store, &project, list, now, timing).await }
    });
    Ok(list)
}

/// Deletes the refs of the pull requests fetched before `since` that `list` no longer has, unless
/// one is on view or a review of it is open: those may need their commits before the platform can
/// be asked again (a closed pull request's branch may be gone).
async fn prune<H: Hostings, P: Project>(
    platforms: &H,
    project: &P,
    list: &PullRequestList,
    since: FetchMark,
) -> Result<(), SessionError> {
    let unlisted = unlisted_fetched(platforms.fetch(), list).await?;
    if unlisted.is_empty() {
        return Ok(());
    }
    let doomed = project.prunable(unlisted).await?;
    let deleted = delete_pull_request_refs(platforms.fetch(), &doomed, since).await?;
    if !deleted.is_empty() {
        tracing::info!(
            refs = deleted.len(),
            "deleted the refs of pull requests no longer listed"
        );
    }
    Ok(())
}

/// Brings in the pull requests of `list` that are not in the repository, so the queue can count
/// the lines of every row, and tells the window when something came.
async fn fetch_heads<H: Hostings, P: Project>(platforms: &H, project: &P, list: &PullRequestList) {
    let credentials = platforms.credentials();
    match fetch_listed_heads(platforms.fetch(), &credentials, list).await {
        Ok(true) => {
            project.announce(FeedEvent::ChangesetsChanged);
            project.heads_fetched(list).await;
        }
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "fetching the listed pull requests failed"),
    }
}

/// Fetches the details of the listed pull requests whose `updatedAt` the stored copy does not
/// have, so a revisit shows them at once. An unchanged pull request is not asked for. The run is
/// paced by [`Prefetch`]; an account that is limited or failing is left alone for the rest of the
/// run, a pull request that is merely gone is passed over. A platform whose list leaves the
/// details apart (`PlatformTraits::details_apart`) is asked for every listed pull request; another
/// for at most [`PREFETCH_LIMIT`] of them, whose list already carries them. What the list shows of
/// a pull request that its details change (`PullRequestCache::record_discussion`) is announced
/// while the run goes, coalesced to one every `Prefetch::announce`, and once more at the end when
/// anything changed since the last.
async fn prefetch<H: Hostings, S: LiveStore, P: Project>(
    platforms: &H,
    store: &S,
    project: &P,
    list: PullRequestList,
    now: UnixClock,
    timing: Prefetch,
) {
    let known = platforms.known();
    let mut stopped: HashSet<String> = HashSet::new();
    let mut capped = 0usize;
    let mut changed: Vec<ChangedPullRequest> = Vec::new();
    let mut announced_at: Option<Instant> = None;
    for listed in &list.rows {
        // What changed since the last announcement reaches the window without waiting for the run
        // to end, but no faster than `timing.announce`.
        if !changed.is_empty() && announced_at.is_none_or(|at| at.elapsed() >= timing.announce) {
            project.announce(FeedEvent::PullRequestsChanged(std::mem::take(&mut changed)));
            announced_at = Some(Instant::now());
        }
        let account = listed.remote.account();
        if stopped.contains(&account) {
            continue;
        }
        if matches!(platforms.standing(&account), Some(Standing::Failing(_))) {
            stopped.insert(account);
            continue;
        }
        let Some(kind) = platform_of(&listed.remote, platforms).await else {
            tracing::warn!(
                repository = %listed.remote,
                "prefetching a pull request of an unknown host"
            );
            continue;
        };
        let hosting = platforms.hosting(kind);
        let apart = hosting.traits().details_apart;
        if !apart && capped >= PREFETCH_LIMIT {
            continue;
        }
        let stored = store.stored_discussion(listed.remote.clone(), listed.row.number).await;
        if let Ok(Some(stored)) = &stored
            && stored.updated_at == listed.row.updated_at
        {
            // Nothing changed since the copy that is kept: no call is made, and the list shows
            // what it says.
            if known.discussion(&listed.remote, listed.row.number).is_none()
                && known.record_discussion(
                    &listed.remote,
                    listed.row.number,
                    stored.synced_at,
                    stored.discussion.clone(),
                )
            {
                record(&known, &listed.remote, listed.row.number, &mut changed);
            }
            continue;
        }
        if !apart {
            capped += 1;
        }
        match hosting.discussion(&listed.remote, listed.row.number).await {
            Ok(discussion) => {
                let at = now();
                if known.record_discussion(&listed.remote, listed.row.number, at, discussion.clone()) {
                    record(&known, &listed.remote, listed.row.number, &mut changed);
                }
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
                // A pull request that is gone says nothing of the account; only an account the
                // platform left unwell (limited, unreachable, refused) stops the rest.
                if matches!(platforms.standing(&account), Some(Standing::Failing(_))) {
                    stopped.insert(account);
                }
            }
        }
        tokio::time::sleep(timing.pace).await;
    }
    if !changed.is_empty() {
        project.announce(FeedEvent::PullRequestsChanged(changed));
    }
}

/// The platform of `remote`: the host Vet knows by name, or one the viewer has an account for (a
/// GitHub Enterprise installation).
async fn platform_of<H: Hostings>(remote: &RemoteKey, platforms: &H) -> Option<PlatformKind> {
    match remote.kind() {
        Some(kind) => Some(kind),
        None if platforms.has_account(&remote.host).await => Some(PlatformKind::GitHub),
        None => None,
    }
}

/// Adds what the details last read of `remote`'s pull request `number` add to its row of the
/// queue to `changed`, for the announcement.
fn record(known: &PullRequestCache, remote: &RemoteKey, number: u64, changed: &mut Vec<ChangedPullRequest>) {
    if let Some(added) = known.added(remote, number) {
        changed.push(ChangedPullRequest {
            remote: remote.clone(),
            number,
            added,
        });
    }
}

#[cfg(test)]
mod tests;
