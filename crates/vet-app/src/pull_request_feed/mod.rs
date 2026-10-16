//! The pull request feed's cycle: listing a project's repositories' pull requests, pruning the
//! refs of the ones a listing no longer has, fetching the listed heads and prefetching the
//! details. It works over two ports: the platform side, which reaches the hosting platforms and
//! the project's refs, and the project side, which is the session on view and the window it tells.
//! The cycles are the runs of a [`Trigger`] on the project's supervisor, and the head fetch and the
//! details read ahead are child runs of it, so a client waiting for quiet waits for them and
//! closing the project ends them.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures::FutureExt;
use tokio::sync::oneshot;

use crate::fetch::{Credentials, Fetch, FetchMark};
use crate::mutex::lock;
use crate::platform::{
    Added, Platform, PlatformError, PlatformKind, Platforms, PullRequest, PullRequestCache, RemoteKey, Standing,
};
use crate::ports::{Fetcher, LiveStore, ReviewWrites};
use crate::pull_request::{
    ListedPullRequest, PullRequestList, RepositoryStatus, delete_pull_request_refs, fetch_listed_heads,
    hosted_repositories, hosts_to_ask_about, list_pull_requests, row_of, unlisted_fetched, unlisted_row,
};
use crate::session::{SessionError, UnixClock};
use crate::supervise::{Supervisor, Ticket, Trigger};

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

/// The kinds of run the feed counts on the supervisor. One cycle, listing through the head fetch,
/// is `LISTING`; its head fetch is `HEADS` and the details read ahead are `PREFETCH`, which are also
/// the slots of the two child runs.
pub const LISTING: &str = "pull requests";
pub const HEADS: &str = "pull request heads";
pub const PREFETCH: &str = "prefetch";

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
    /// The feed published a different list, which the window reads again.
    ListChanged,
}

/// The pull request on view, as the platform last described it.
#[derive(Clone, Debug)]
pub struct OnView {
    pub remote: RemoteKey,
    pub pull_request: PullRequest,
}

/// Why a cycle runs, merged over the requests it serves.
#[derive(Clone, Copy)]
struct Reasons {
    /// Whether the cycle asks every account, as the first listing and a refresh the viewer asked for
    /// do, rather than leaving the rate limited ones alone as the timer and an account change do.
    asks: bool,
}

impl Reasons {
    fn merge(&mut self, more: Reasons) {
        self.asks |= more.asks;
    }
}

/// The platform side of the feed: the clients of the hosting platforms, the viewer's accounts,
/// how the platforms are doing, and the project's refs.
pub trait Hostings {
    /// The project's git remotes and refs.
    type Fetcher: Fetcher + Send + Sync;

    /// The client of the platform `kind`.
    fn hosting(&self, kind: PlatformKind) -> impl Platform + Send + Sync;
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

/// Asks the platforms for the project's repositories' open pull requests. Answers with the list, the
/// pull request on view the moment before the listing, and the mark the refs were at before it,
/// which [`prune`] keeps what is fetched after it. A repository the platform cannot answer for is
/// reported in `sources`, the others still list; a failure before any listing, such as unreadable
/// remotes, fails the whole call. Unless the cycle `asks`, an account that is rate limited until
/// after `now` is not asked: its repositories report the limit and list no rows.
async fn listed<H: Hostings, P: Project>(
    platforms: &H,
    project: &P,
    asks: bool,
    now: i64,
) -> Result<(PullRequestList, Option<(RemoteKey, u64)>, FetchMark), SessionError> {
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
    let since = fetch.mark();
    let list = list_unless_limited(platforms, PlatformKind::GitHub, &github, asks, now)
        .await
        .and(list_unless_limited(platforms, PlatformKind::AzureDevOps, &azure_devops, asks, now).await);
    Ok((list, on_view, since))
}

/// The open pull requests of `repositories` on the platform `kind`, leaving out the ones of an
/// account that is limited until after `now` when the cycle does not `ask`. The sources keep the
/// order of `repositories`.
async fn list_unless_limited<H: Hostings>(
    platforms: &H,
    kind: PlatformKind,
    repositories: &[RemoteKey],
    asks: bool,
    now: i64,
) -> PullRequestList {
    let limit_of = |remote: &RemoteKey| match platforms.standing(&remote.account()) {
        Some(Standing::Failing(error @ PlatformError::RateLimited { retry_at: Some(at), .. })) if at > now => {
            Some(error)
        }
        _ => None,
    };
    let (skipped, asked): (Vec<_>, Vec<_>) = if asks {
        (Vec::new(), repositories.iter().map(|r| (r, None)).collect())
    } else {
        repositories
            .iter()
            .map(|r| (r, limit_of(r)))
            .partition(|(_, limit)| limit.is_some())
    };
    let asked: Vec<RemoteKey> = asked.into_iter().map(|(remote, _)| remote.clone()).collect();
    let mut list = list_pull_requests(&platforms.hosting(kind), &asked).await;
    list.sources
        .extend(skipped.into_iter().map(|(remote, error)| RepositoryStatus {
            remote: remote.clone(),
            error,
            truncated: false,
        }));
    list.sources
        .sort_by_key(|source| repositories.iter().position(|r| *r == source.remote));
    list
}

/// The feed of a project: the last listing, the requests to list again, and the cycles they run.
/// A cloneable handle; it answers the readers from the list it keeps, and its requests are the
/// triggers of a [`Trigger`], which merges the ones that come during a cycle into one more cycle.
pub struct PullRequestFeed<H, S, P> {
    inner: Arc<Feed<H, S, P>>,
}

/// What the handles of a [`PullRequestFeed`] share.
struct Feed<H, S, P> {
    platforms: H,
    store: S,
    project: P,
    now: UnixClock,
    timing: Prefetch,
    /// Runs the cycles, and the child runs of a cycle.
    supervisor: Supervisor,
    refresh: Trigger<Reasons, ()>,
    /// The last listing and what the cycles made of it.
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The last listing, as the platforms gave it.
    listed: Option<PullRequestList>,
    /// The row of the pull request on view when the platforms' listing leaves it out, which the
    /// list shows beside the listed rows.
    extra: Option<ListedPullRequest>,
    /// Counts the changes of the pull request on view, so a cycle that listed before one does not
    /// publish the row it made of the earlier.
    on_view_changes: u64,
    /// Why the last cycle failed, kept while nothing is listed.
    error: Option<SessionError>,
}

impl State {
    /// The list the readers see: the listed rows and the extra one.
    fn shown(&self) -> Option<PullRequestList> {
        self.listed.clone().map(|list| list.with_row(self.extra.as_ref()))
    }
}

/// What the feed has to answer [`PullRequestFeed::list`] with.
enum Current {
    Listed(PullRequestList),
    Failed(SessionError),
    Empty,
}

/// What one cycle made of the listing.
enum Cycle {
    /// The list is what it was: nothing was announced or started again.
    Unchanged,
    /// The list changed and was published; its details are the prefetch's work.
    Changed(PullRequestList),
    /// The cycle failed; the last list, if any, is kept.
    Failed(SessionError),
}

impl<H, S, P> Clone for PullRequestFeed<H, S, P> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<H, S, P> PullRequestFeed<H, S, P>
where
    H: Hostings + Clone + Send + Sync + 'static,
    S: LiveStore + Clone + Send + Sync + 'static,
    P: Project + Clone + Send + Sync + 'static,
{
    /// A feed over the ports, whose cycles and child runs go on `supervisor`. Idle until asked.
    pub fn new(platforms: H, store: S, project: P, now: UnixClock, timing: Prefetch, supervisor: Supervisor) -> Self {
        let inner = Arc::new_cyclic(|feed: &Weak<Feed<H, S, P>>| {
            let feed = feed.clone();
            Feed {
                platforms,
                store,
                project,
                now,
                timing,
                supervisor: supervisor.clone(),
                refresh: Trigger::new(supervisor, LISTING, Reasons::merge, move |reasons| {
                    let feed = feed.clone();
                    async move {
                        if let Some(inner) = feed.upgrade() {
                            PullRequestFeed { inner }.refresh(reasons).await;
                        }
                    }
                    .boxed()
                }),
                state: Mutex::new(State::default()),
            }
        });
        Self { inner }
    }

    /// The feed's list. When there is none, the read waits for the cycle under way or asks for one,
    /// so a failure that left no list is asked for again and a transient one recovers on the next
    /// read. Readers arriving together share the cycle.
    pub async fn list(&self) -> Result<PullRequestList, SessionError> {
        if let Current::Listed(list) = self.current() {
            return Ok(list);
        }
        self.inner.refresh.join(Reasons { asks: true }).wait().await;
        match self.current() {
            Current::Listed(list) => Ok(list),
            Current::Failed(error) => Err(error),
            Current::Empty => Err(SessionError::Invalid("the project is closed".to_owned())),
        }
    }

    /// Requests a refresh that asks every account and waits for the cycle serving it, which begins
    /// after the request: the next one when the feed is idle, one more when a cycle is running.
    pub async fn manual(&self) {
        self.request(true).wait().await;
    }

    /// Requests the periodic refresh, which a cycle under way answers, and which leaves the rate
    /// limited accounts alone.
    pub fn timer(&self) {
        self.inner.refresh.join(Reasons { asks: false });
    }

    /// Requests a refresh because the viewer's accounts changed, which leaves the rate limited
    /// accounts alone.
    pub fn accounts(&self) {
        self.request(false);
    }

    fn request(&self, asks: bool) -> Ticket<()> {
        self.inner.refresh.request(Reasons { asks })
    }

    /// One refresh, for the requests merged into it: a cycle, then the details read ahead for the
    /// list it published, as a child run that outlives it. A cycle that finds the list unchanged
    /// keeps the details already under way, so a stream of requests does not start them over.
    async fn refresh(&self, reasons: Reasons) {
        match self.cycle(reasons.asks).await {
            Cycle::Changed(list) => self.start_prefetch(list),
            Cycle::Unchanged => {}
            Cycle::Failed(error) => self.failed(error),
        }
    }

    fn current(&self) -> Current {
        let state = lock(&self.inner.state);
        match state.shown() {
            Some(list) => Current::Listed(list),
            None => match &state.error {
                Some(error) => Current::Failed(error.clone()),
                None => Current::Empty,
            },
        }
    }

    /// One cycle: list, publish when the list differs from the last, prune, fetch the heads and
    /// hand the details back for the prefetch. The listing of a repository that fails keeps the rows
    /// of the last list for it, with its error in its source. The heads are fetched on every
    /// successful cycle, changed or not: a head fetch that failed is retried by the next one. A
    /// cycle that does not `ask` leaves the accounts that are rate limited alone.
    async fn cycle(&self, asks: bool) -> Cycle {
        let on_view_changes = lock(&self.inner.state).on_view_changes;
        let now = (self.inner.now)();
        let (fresh, on_view, since) = match listed(&self.inner.platforms, &self.inner.project, asks, now).await {
            Ok(got) => got,
            Err(error) => return Cycle::Failed(error),
        };
        let merged = self.keep_failed_repositories(fresh);
        let found = self.on_view_row(&merged, on_view.as_ref()).await;
        let (shown, changed, announce) = {
            let mut state = lock(&self.inner.state);
            let before = state.shown();
            // A pull request put on view while the cycle listed has the row it was put on view
            // with, which is newer than the one the cycle made of the earlier.
            let extra = if state.on_view_changes == on_view_changes {
                found
            } else {
                state.extra.take()
            };
            state.extra = extra.filter(|row| !merged.has(&row.remote, row.row.number));
            let shown = merged.clone().with_row(state.extra.as_ref());
            state.listed = Some(merged);
            let changed = before.as_ref() != Some(&shown);
            // A read waits for the first listing, so only a reader that was told of a failure or
            // holds an older list has anything to read again.
            let announce = changed && (before.is_some() || state.error.is_some());
            if changed {
                state.error = None;
            }
            (shown, changed, announce)
        };
        if announce {
            self.inner.project.announce(FeedEvent::ListChanged);
        }
        if changed {
            // The details read ahead for the list before are not the ones this list needs.
            self.inner.supervisor.stop(PREFETCH);
            if let Err(error) = prune(&self.inner.platforms, &self.inner.project, &shown, since).await {
                tracing::warn!(%error, "deleting the refs of pull requests no longer listed failed");
            }
        }
        self.fetch_heads_as_child(&shown).await;
        if changed {
            Cycle::Changed(shown)
        } else {
            Cycle::Unchanged
        }
    }

    /// The row of the pull request on view, when `listed` leaves it out. A repository the platform
    /// cannot answer for keeps the row the pull request had.
    async fn on_view_row(
        &self,
        listed: &PullRequestList,
        on_view: Option<&(RemoteKey, u64)>,
    ) -> Option<ListedPullRequest> {
        let (remote, number) = on_view?;
        let kind = platform_of(remote, &self.inner.platforms).await?;
        let found = unlisted_row(&self.inner.platforms.hosting(kind), listed, remote, *number).await;
        let failed = listed
            .sources
            .iter()
            .any(|source| source.remote == *remote && source.error.is_some());
        if found.is_some() || !failed {
            return found;
        }
        lock(&self.inner.state)
            .extra
            .clone()
            .filter(|row| row.remote == *remote && row.row.number == *number)
    }

    /// Tells the feed which pull request is on view, with the platform's description of it, so the
    /// list shows its row beside the listed ones without asking the platform for it. None when the
    /// changeset on view is no pull request. Announces the list when it differs from what readers
    /// last saw.
    pub async fn on_view_changed(&self, on_view: Option<OnView>) {
        let row = match on_view {
            Some(OnView { remote, pull_request }) => match platform_of(&remote, &self.inner.platforms).await {
                Some(kind) => row_of(&self.inner.platforms.hosting(kind), &remote, pull_request).await,
                None => None,
            },
            None => None,
        };
        let announce = {
            let mut state = lock(&self.inner.state);
            state.on_view_changes += 1;
            let before = state.shown();
            let extra = row.filter(|row| {
                state
                    .listed
                    .as_ref()
                    .is_none_or(|list| !list.has(&row.remote, row.row.number))
            });
            state.extra = extra;
            let after = state.shown();
            after != before && before.is_some()
        };
        if announce {
            self.inner.project.announce(FeedEvent::ListChanged);
        }
    }

    /// The rows of the repositories whose listing failed, from the last list, so a repository the
    /// platform cannot answer for shows its error beside the rows it had.
    fn keep_failed_repositories(&self, fresh: PullRequestList) -> PullRequestList {
        let failed: Vec<RemoteKey> = fresh
            .sources
            .iter()
            .filter(|source| source.error.is_some())
            .map(|source| source.remote.clone())
            .collect();
        if failed.is_empty() {
            return fresh;
        }
        let kept = {
            let state = lock(&self.inner.state);
            let Some(old) = &state.listed else {
                return fresh;
            };
            old.rows
                .iter()
                .filter(|listed| failed.contains(&listed.remote))
                .cloned()
                .collect()
        };
        fresh.and(PullRequestList {
            rows: kept,
            sources: Vec::new(),
        })
    }

    /// Keeps the failure while nothing is listed, so a read answers with it instead of waiting.
    fn failed(&self, error: SessionError) {
        let mut state = lock(&self.inner.state);
        if state.listed.is_none() {
            state.error = Some(error);
        }
    }

    /// Fetches the heads of `list` as a child run and waits for it. The cycle goes on without
    /// waiting further when the project closes and the run is aborted.
    async fn fetch_heads_as_child(&self, list: &PullRequestList) {
        let (platforms, project, list) = (self.inner.platforms.clone(), self.inner.project.clone(), list.clone());
        let (done, ended) = oneshot::channel();
        let supervisor = &self.inner.supervisor;
        supervisor.replace(
            HEADS,
            supervisor.track(HEADS, async move {
                fetch_heads(&platforms, &project, &list).await;
                let _ = done.send(());
            }),
        );
        let _ = ended.await;
    }

    /// Reads the details of `list` ahead as a child run that replaces the one before and ends
    /// with the project.
    fn start_prefetch(&self, list: PullRequestList) {
        let platforms = self.inner.platforms.clone();
        let store = self.inner.store.clone();
        let project = self.inner.project.clone();
        let (now, timing) = (self.inner.now, self.inner.timing);
        let supervisor = &self.inner.supervisor;
        supervisor.replace(
            PREFETCH,
            supervisor.track(PREFETCH, async move {
                prefetch(&platforms, &store, &project, list, now, timing).await
            }),
        );
    }
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
