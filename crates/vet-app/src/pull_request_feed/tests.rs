//! The cycle over the ports, with a platform that lists and discusses, git remotes that record
//! what they were asked, and a session that records what the cycle did to it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;

use super::*;
use crate::fetch::Credentials;
use crate::platform::{
    CheckLog, CommitChecks, Discussion, FakeHosting, Health, PlatformError, PlatformTraits, PullRequest,
    PullRequestDetail, PullRequestListing, PullRequestQuery, QueueTab, Viewer, Watched, pr, rows, viewer,
};
use crate::ports::{FetchAuth, GitError, RefTip, RefUpdate, RemoteInfo};
use crate::progress::Tasks;
use crate::pull_request::tests::{Anonymous, Remotes, fetching, fetching_into, github, info, names};
use crate::pull_request::{ListedPullRequest, PullRequestRefs, RepositoryStatus};
use crate::session::tests::MemStore;

/// What the cycle did, in order, as each double records it.
type Log = Arc<Mutex<Vec<String>>>;

fn recorded(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// The platform side of the ports, over a platform of the test's and git remotes of the test's.
struct Side<P, G = Remotes> {
    platform: P,
    health: Arc<Health>,
    known: Arc<PullRequestCache>,
    fetch: Arc<Fetch<G>>,
    /// The hosts the viewer has an account for; the tests' remotes are all known by name.
    accounts: Vec<String>,
}

impl<P: Clone, G> Clone for Side<P, G> {
    fn clone(&self) -> Self {
        Self {
            platform: self.platform.clone(),
            health: self.health.clone(),
            known: self.known.clone(),
            fetch: self.fetch.clone(),
            accounts: self.accounts.clone(),
        }
    }
}

impl<P, G> Hostings for Side<P, G>
where
    P: Platform + Clone + Send + Sync + 'static,
    G: Fetcher + Send + Sync + 'static,
{
    type Fetcher = G;

    fn hosting(&self, kind: PlatformKind) -> impl Platform + Send {
        Watched::new(self.platform.clone(), kind, self.health.clone(), self.known.clone())
    }

    async fn has_account(&self, host: &str) -> bool {
        self.accounts.iter().any(|h| h == host)
    }

    fn standing(&self, account: &str) -> Option<Standing> {
        self.health.standing(account)
    }

    fn known(&self) -> Arc<PullRequestCache> {
        self.known.clone()
    }

    fn fetch(&self) -> &Fetch<G> {
        &self.fetch
    }

    fn credentials(&self) -> impl Credentials + Send + Sync + '_ {
        Anonymous
    }
}

/// The pull requests a test's platform lists for each repository.
type Listed = Arc<Mutex<Vec<(RemoteKey, Vec<PullRequest>)>>>;

/// A platform the cycle lists and discusses, recording each of those in the log. What it lists and
/// which repositories it fails can change between cycles, and a listing waits for the next gate a
/// test left, so a cycle can be held in the middle of listing.
#[derive(Clone)]
struct Fake {
    listed: Listed,
    failing: Arc<Mutex<Vec<RemoteKey>>>,
    traits: PlatformTraits,
    log: Log,
    gates: Arc<Mutex<Vec<Arc<Notify>>>>,
}

impl Fake {
    /// Lists `numbers` newest-numbered first for `remote` from now on.
    fn lists(&self, listed: Vec<(RemoteKey, Vec<u64>)>) {
        *self.listed.lock().unwrap() = listed
            .into_iter()
            .map(|(remote, numbers)| (remote, numbers.into_iter().map(pr).collect()))
            .collect();
    }

    /// Fails every call about `remote` from now on.
    fn fails(&self, remote: &RemoteKey) {
        self.failing.lock().unwrap().push(remote.clone());
    }

    /// Holds the next listing until the gate is opened, once.
    fn gate(&self, gate: Arc<Notify>) {
        self.gates.lock().unwrap().push(gate);
    }
}

impl Platform for Fake {
    fn traits(&self) -> PlatformTraits {
        self.traits.clone()
    }

    async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
        Ok(viewer())
    }

    async fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        _: &PullRequestQuery,
    ) -> Result<PullRequestListing, PlatformError> {
        self.log.lock().unwrap().push("list".to_owned());
        let gate = {
            let mut gates = self.gates.lock().unwrap();
            (!gates.is_empty()).then(|| gates.remove(0))
        };
        if let Some(gate) = gate {
            gate.notified().await;
        }
        if self.failing.lock().unwrap().contains(remote) {
            return Err(PlatformError::Rejected("refused".to_owned()));
        }
        Ok(PullRequestListing {
            pull_requests: self
                .listed
                .lock()
                .unwrap()
                .iter()
                .find(|(r, _)| r == remote)
                .map(|(_, prs)| prs.clone())
                .unwrap_or_default(),
            truncated: false,
        })
    }

    async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
        if self.failing.lock().unwrap().contains(remote) {
            return Err(PlatformError::SignedOut(remote.host.clone()));
        }
        let mut pull_request = pr(number);
        pull_request.target_branch = "release".to_owned();
        Ok(PullRequestDetail {
            pull_request,
            base: "b0".to_owned(),
            pushes: Vec::new(),
        })
    }

    async fn discussion(&self, _: &RemoteKey, number: u64) -> Result<Discussion, PlatformError> {
        self.log.lock().unwrap().push(format!("discussion {number}"));
        let mut discussion = FakeHosting::default().0.lock().unwrap().discussion.clone();
        discussion.title = format!("PR {number}");
        Ok(discussion)
    }

    async fn checks(&self, _: &RemoteKey, _: &str) -> Result<CommitChecks, PlatformError> {
        unreachable!("the cycle lists, fetches heads and prefetches")
    }

    async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
        unreachable!("the cycle lists, fetches heads and prefetches")
    }
}

/// The project's git remotes, which a test can make unreadable or refuse the refs: a cycle that
/// cannot read the remotes fails before it lists, and one the remote refuses the refs of lists but
/// fetches no head, as being offline does.
#[derive(Clone)]
struct Readable {
    remotes: Arc<Mutex<Vec<RemoteInfo>>>,
    refs: Arc<Mutex<Vec<RefTip>>>,
    unreadable: Arc<AtomicBool>,
    refused: Arc<AtomicBool>,
}

impl Readable {
    fn of(remotes: Vec<RemoteInfo>) -> Self {
        Self {
            remotes: Arc::new(Mutex::new(remotes)),
            refs: Arc::default(),
            unreadable: Arc::default(),
            refused: Arc::default(),
        }
    }

    fn break_remotes(&self) {
        self.unreadable.store(true, Ordering::SeqCst);
    }

    fn mend_remotes(&self) {
        self.unreadable.store(false, Ordering::SeqCst);
    }

    fn refuse_refs(&self) {
        self.refused.store(true, Ordering::SeqCst);
    }

    fn allow_refs(&self) {
        self.refused.store(false, Ordering::SeqCst);
    }
}

impl Fetcher for Readable {
    async fn remotes(&self) -> Result<Vec<RemoteInfo>, GitError> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(GitError::Remote("the remotes cannot be read".to_owned()));
        }
        Ok(self.remotes.lock().unwrap().clone())
    }

    async fn fetch(&self, _: &str, _: Option<FetchAuth>) -> Result<usize, GitError> {
        unreachable!("the cycle fetches refs, not whole remotes")
    }

    async fn fetch_refs(&self, _: &str, updates: Vec<RefUpdate>, _: Option<FetchAuth>) -> Result<(), GitError> {
        if self.refused.load(Ordering::SeqCst) {
            return Err(GitError::Remote("the refs were refused".to_owned()));
        }
        let mut refs = self.refs.lock().unwrap();
        for update in &updates {
            refs.retain(|r| r.name != update.destination);
            refs.push(RefTip {
                name: update.destination.clone(),
                commit: "tip".to_owned(),
            });
        }
        Ok(())
    }

    async fn refs_below(&self, prefix: &str) -> Result<Vec<RefTip>, GitError> {
        Ok(self
            .refs
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.name.starts_with(prefix))
            .cloned()
            .collect())
    }

    async fn descends_from(&self, descendant: &str, ancestor: &str) -> Result<bool, GitError> {
        Ok(descendant == ancestor)
    }

    async fn delete_refs(&self, names: Vec<String>) -> Result<(), GitError> {
        self.refs.lock().unwrap().retain(|r| !names.contains(&r.name));
        Ok(())
    }
}

/// The project side of the ports, recording the prune, the head fetch and the announcements.
#[derive(Clone, Default)]
struct ProjectFake {
    log: Log,
    on_view: Option<(RemoteKey, u64)>,
}

impl Project for ProjectFake {
    type Activity = ();

    fn activity(&self, _: &'static str) {}

    async fn on_view(&self) -> Option<(RemoteKey, u64)> {
        self.on_view.clone()
    }

    async fn prunable(&self, unlisted: Vec<(RemoteKey, u64)>) -> Result<Vec<(RemoteKey, u64)>, SessionError> {
        self.log.lock().unwrap().push("prune".to_owned());
        Ok(unlisted)
    }

    async fn heads_fetched(&self, _: &PullRequestList) {
        self.log.lock().unwrap().push("heads_fetched".to_owned());
    }

    fn announce(&self, event: FeedEvent) {
        let name = match event {
            FeedEvent::ChangesetsChanged => "changesets_changed",
            FeedEvent::PullRequestsChanged(_) => "pull_requests_changed",
            FeedEvent::ListChanged => "list_changed",
        };
        self.log.lock().unwrap().push(name.to_owned());
    }
}

/// The platform side of the ports over `platform` and empty remotes.
fn hosting_side(platform: FakeHosting) -> Side<FakeHosting> {
    Side {
        platform,
        health: Arc::new(Health::new(|_| {})),
        known: Arc::default(),
        fetch: Arc::new(fetching(vec![]).0),
        accounts: vec![],
    }
}

/// `numbers` listed for `remote`, newest first by their numbers.
fn listed(numbers: &[u64], remote: &RemoteKey) -> PullRequestList {
    let query = PullRequestQuery::new(QueueTab::AllOpen);
    PullRequestList {
        rows: numbers
            .iter()
            .map(|number| {
                let mut pull_request = pr(*number);
                pull_request.updated_at = *number as i64;
                ListedPullRequest {
                    remote: remote.clone(),
                    row: rows(vec![pull_request], &viewer(), &query).remove(0),
                }
            })
            .collect(),
        sources: vec![RepositoryStatus {
            remote: remote.clone(),
            error: None,
            truncated: false,
        }],
    }
}

#[tokio::test]
async fn the_cycle_lists_prunes_fetches_heads_and_prefetches_in_order() {
    let log = Log::default();
    let app = github("acme/app");
    let stale = PullRequestRefs::of(&app, 9);
    let (fetch, _, _) = fetching_into(
        vec![info("origin", "https://github.com/acme/app")],
        &[&stale.head, &stale.target],
    );
    let side = Side {
        platform: Fake {
            listed: Arc::new(Mutex::new(vec![(app, vec![pr(7)])])),
            failing: Arc::default(),
            traits: PlatformTraits::github(),
            log: log.clone(),
            gates: Arc::default(),
        },
        health: Arc::new(Health::new(|_| {})),
        known: Arc::default(),
        fetch: Arc::new(fetch),
        accounts: vec![],
    };
    let project = ProjectFake {
        log: log.clone(),
        on_view: None,
    };
    let feed = PullRequestFeed::new(
        side,
        MemStore::default(),
        project,
        || 1_000,
        Prefetch {
            pace: Duration::ZERO,
            announce: Duration::ZERO,
        },
    );
    let _run = tokio::spawn(feed.clone().run());

    feed.manual().await;
    // The prefetch runs after the cycle the manual request waited for; wait for it to reach a row.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while discussions(&log) == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let phases: Vec<String> = recorded(&log)
        .into_iter()
        .filter(|entry| matches!(entry.as_str(), "list" | "prune" | "heads_fetched") || entry.starts_with("discussion"))
        .collect();
    assert_eq!(phases, ["list", "prune", "heads_fetched", "discussion 7"]);
}

#[tokio::test]
async fn the_refs_of_a_pull_request_fetched_during_the_listing_are_not_pruned() {
    let app = github("acme/app");
    let of = |number| PullRequestRefs::of(&app, number);
    let before = [of(4).head, of(4).target];
    let before: Vec<&str> = before.iter().map(String::as_str).collect();
    let (fetch, _, held) = fetching_into(vec![info("origin", "https://github.com/acme/app")], &before);
    let side = Side {
        platform: Fake {
            listed: Arc::default(),
            failing: Arc::default(),
            traits: PlatformTraits::github(),
            log: Log::default(),
            gates: Arc::default(),
        },
        health: Arc::new(Health::new(|_| {})),
        known: Arc::default(),
        fetch: Arc::new(fetch),
        accounts: vec![],
    };

    let since = side.fetch.mark();
    // #5 is brought in while the list is being asked for: its refs are written after the mark.
    side.fetch
        .refs(
            &Anonymous,
            &info("origin", "https://github.com/acme/app"),
            vec![RefUpdate {
                source: "refs/pull/5/head".to_owned(),
                destination: of(5).head.clone(),
            }],
            "Fetching #5".to_owned(),
        )
        .await
        .unwrap();

    let list = PullRequestList {
        rows: vec![],
        sources: vec![RepositoryStatus {
            remote: app.clone(),
            error: None,
            truncated: false,
        }],
    };
    prune(&side, &ProjectFake::default(), &list, since).await.unwrap();

    let names = names(&held);
    assert!(names.contains(&of(5).head), "{names:?}");
    assert!(
        !names.contains(&of(4).head),
        "the one fetched before the mark is pruned"
    );
}

#[tokio::test]
async fn prefetching_stops_for_a_failing_account_but_not_for_a_pull_request_that_is_gone() {
    let remote = github("acme/app");
    let list = listed(&[7, 8, 9], &remote);

    // An account the platform refuses stops the rest of the run.
    let refused = FakeHosting::default();
    refused.fail("discussion", PlatformError::Rejected("refused".to_owned()));
    prefetch(
        &hosting_side(refused.clone()),
        &MemStore::default(),
        &ProjectFake::default(),
        list.clone(),
        || 0,
        Prefetch {
            pace: Duration::ZERO,
            announce: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(refused.calls(), ["discussion"], "the first refusal stops the rest");

    // A pull request that is merely gone says nothing of the account.
    let gone = FakeHosting::default();
    gone.fail("discussion", PlatformError::NotFound("gone".to_owned()));
    prefetch(
        &hosting_side(gone.clone()),
        &MemStore::default(),
        &ProjectFake::default(),
        list,
        || 0,
        Prefetch {
            pace: Duration::ZERO,
            announce: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(gone.calls().len(), 3, "a gone pull request does not stop the others");
}

#[tokio::test]
async fn the_prefetch_bound_covers_a_list_that_leaves_details_apart_and_not_one_that_carries_them() {
    let remote = github("acme/app");
    let numbers: Vec<u64> = (1..=25).collect();
    let list = listed(&numbers, &remote);

    // GitHub's list leaves the details apart: every listed pull request is fetched.
    let apart = FakeHosting::default();
    prefetch(
        &hosting_side(apart.clone()),
        &MemStore::default(),
        &ProjectFake::default(),
        list.clone(),
        || 0,
        Prefetch {
            pace: Duration::ZERO,
            announce: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(apart.calls().len(), 25);

    // A platform whose list already carries the details stops at the limit.
    let carrying = FakeHosting::per_comment();
    prefetch(
        &hosting_side(carrying.clone()),
        &MemStore::default(),
        &ProjectFake::default(),
        list,
        || 0,
        Prefetch {
            pace: Duration::ZERO,
            announce: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(carrying.calls().len(), 20);
}

/// How many entries of `log` are exactly `what`.
fn counting(log: &Log, what: &str) -> usize {
    recorded(log).iter().filter(|entry| entry.as_str() == what).count()
}

/// How many pull requests' details the log says were read.
fn discussions(log: &Log) -> usize {
    recorded(log)
        .iter()
        .filter(|entry| entry.starts_with("discussion"))
        .count()
}

/// A prefetch that paces and announces nothing, for the tests that do not hold it running.
fn zero() -> Prefetch {
    Prefetch {
        pace: Duration::ZERO,
        announce: Duration::ZERO,
    }
}

/// A platform that lists `numbers` for each remote and records its calls in `log`.
fn listing(log: &Log, listed: Vec<(RemoteKey, Vec<u64>)>) -> Fake {
    let fake = Fake {
        listed: Arc::default(),
        failing: Arc::default(),
        traits: PlatformTraits::github(),
        log: log.clone(),
        gates: Arc::default(),
    };
    fake.lists(listed);
    fake
}

/// A feed over `platform` and the git `remotes`, with a mem store and the clock `1_000`.
fn feed_of(
    platform: Fake,
    project: ProjectFake,
    remotes: Vec<RemoteInfo>,
    timing: Prefetch,
) -> (PullRequestFeed<Side<Fake, Readable>, MemStore, ProjectFake>, Readable) {
    let readable = Readable::of(remotes);
    let side = Side {
        platform,
        health: Arc::new(Health::new(|_| {})),
        known: Arc::default(),
        fetch: Arc::new(Fetch::new(readable.clone(), Tasks::default())),
        accounts: vec![],
    };
    (
        PullRequestFeed::new(side, MemStore::default(), project, || 1_000, timing),
        readable,
    )
}

/// A project that records what the feed announces and does to it.
fn project(log: &Log) -> ProjectFake {
    ProjectFake {
        log: log.clone(),
        on_view: None,
    }
}

fn numbers(list: &PullRequestList) -> Vec<u64> {
    list.rows.iter().map(|r| r.row.number).collect()
}

#[tokio::test]
async fn ten_timer_requests_during_a_run_give_one_cycle() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7])]);
    let gate = Arc::new(Notify::new());
    platform.gate(gate.clone());
    let (feed, _) = feed_of(
        platform,
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    feed.timer();
    // The one cycle starts and waits inside its listing while the other requests arrive.
    tokio::task::yield_now().await;
    for _ in 0..10 {
        feed.timer();
    }
    gate.notify_one();

    assert_eq!(numbers(&feed.list().await.unwrap()), [7]);
    assert_eq!(counting(&log, "list"), 1, "the requests joined the one cycle");
}

#[tokio::test]
async fn a_manual_request_during_a_run_gives_one_trailing_cycle_and_its_caller_waits() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7])]);
    let first = Arc::new(Notify::new());
    let second = Arc::new(Notify::new());
    platform.gate(first.clone());
    platform.gate(second.clone());
    let (feed, _) = feed_of(
        platform,
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    feed.timer();
    tokio::task::yield_now().await; // the first cycle waits inside its listing

    let manual = feed.clone();
    let (done, mut told) = tokio::sync::oneshot::channel();
    let waiting = tokio::spawn(async move {
        manual.manual().await;
        done.send(()).ok();
    });
    tokio::task::yield_now().await; // the manual request is made during the first cycle

    first.notify_one();
    tokio::task::yield_now().await; // the trailing cycle waits inside its listing

    assert!(
        told.try_recv().is_err(),
        "the caller waits for a cycle started after its request"
    );

    second.notify_one();
    waiting.await.unwrap();
    assert_eq!(counting(&log, "list"), 2, "exactly one trailing cycle");
}

#[tokio::test]
async fn reading_an_empty_feed_waits_for_the_first_listing() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7])]);
    let (feed, _) = feed_of(
        platform,
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    assert_eq!(numbers(&feed.list().await.unwrap()), [7]);

    // With no hosted repository the first read still waits for the empty list the feed publishes.
    let log = Log::default();
    let platform = listing(&log, vec![]);
    let (feed, _) = feed_of(
        platform,
        project(&log),
        vec![info("lab", "https://gitlab.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    let listed = feed.list().await.unwrap();
    assert!(listed.rows.is_empty() && listed.sources.is_empty());
}

#[tokio::test]
async fn a_failed_cycle_keeps_the_last_list() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7])]);
    let (feed, refs) = feed_of(
        platform.clone(),
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    feed.manual().await;
    assert_eq!(numbers(&feed.list().await.unwrap()), [7]);

    // The remotes cannot be read: the next cycle fails and the last list stands.
    refs.break_remotes();
    platform.lists(vec![(app.clone(), vec![8])]);
    feed.manual().await;

    assert_eq!(
        numbers(&feed.list().await.unwrap()),
        [7],
        "the failed listing is not shown"
    );
    assert_eq!(counting(&log, "list"), 1, "the platforms were not asked again");
}

#[tokio::test]
async fn a_read_of_a_feed_with_only_a_failure_lists_again() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7])]);
    let (feed, refs) = feed_of(
        platform,
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    refs.break_remotes();
    assert!(feed.list().await.is_err(), "the first listing fails");

    // The cause goes away: the next read asks again and answers with that cycle.
    refs.mend_remotes();
    assert_eq!(numbers(&feed.list().await.unwrap()), [7], "the next read lists again");
}

#[tokio::test]
async fn a_repository_whose_listing_fails_keeps_its_rows_and_shows_the_error() {
    let log = Log::default();
    let app = github("acme/app");
    let lib = github("acme/lib");
    let remotes = vec![
        info("origin", "https://github.com/acme/app"),
        info("lib", "https://github.com/acme/lib"),
    ];
    let platform = listing(&log, vec![(app.clone(), vec![1, 2]), (lib.clone(), vec![3])]);
    let (feed, _) = feed_of(platform.clone(), project(&log), remotes, zero());
    let _run = tokio::spawn(feed.clone().run());

    feed.manual().await;
    assert_eq!(numbers(&feed.list().await.unwrap()), [3, 2, 1]);

    // One repository cannot be listed: it keeps its rows beside its error, the other still lists.
    platform.fails(&app);
    feed.manual().await;
    let listed = feed.list().await.unwrap();

    assert_eq!(
        listed
            .rows
            .iter()
            .map(|r| (&r.remote, r.row.number))
            .collect::<Vec<_>>(),
        [(&lib, 3), (&app, 2), (&app, 1)],
        "the failed repository's rows are kept, newest first"
    );
    let app_status = listed.sources.iter().find(|s| s.remote == app).unwrap();
    assert!(
        matches!(app_status.error, Some(PlatformError::Rejected(_))),
        "{app_status:?}"
    );
    let lib_status = listed.sources.iter().find(|s| s.remote == lib).unwrap();
    assert_eq!(lib_status.error, None);
}

#[tokio::test]
async fn an_unchanged_list_announces_nothing_and_keeps_a_running_prefetch_a_changed_one_restarts_it() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7, 8])]);
    let (feed, _) = feed_of(
        platform.clone(),
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        Prefetch {
            pace: Duration::from_secs(3_600),
            announce: Duration::ZERO,
        },
    );
    let _run = tokio::spawn(feed.clone().run());

    feed.manual().await; // lists 7 and 8 and leaves the details fetched ahead running
    assert_eq!(counting(&log, "list"), 1);
    assert_eq!(discussions(&log), 1, "the prefetch runs on the first row");

    // The list is the same: nothing is announced and the running prefetch is left alone.
    let announced = counting(&log, "list_changed");
    feed.manual().await;
    assert_eq!(counting(&log, "list"), 2);
    assert_eq!(
        counting(&log, "list_changed"),
        announced,
        "an unchanged list is not announced"
    );
    assert_eq!(discussions(&log), 1, "the running prefetch was not started over");

    // The list changed: the details fetched ahead are started again.
    platform.lists(vec![(app.clone(), vec![7, 8, 9])]);
    feed.manual().await;
    assert_eq!(
        counting(&log, "list_changed"),
        announced + 1,
        "the changed list is announced"
    );
    assert_eq!(
        discussions(&log),
        2,
        "the changed list starts the details fetched ahead again"
    );
}

#[tokio::test]
async fn the_first_list_is_announced_only_to_a_reader_that_was_told_of_a_failure() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7])]);
    let (feed, refs) = feed_of(
        platform,
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    refs.break_remotes();
    assert!(feed.list().await.is_err());
    refs.mend_remotes();
    feed.manual().await;
    assert_eq!(
        counting(&log, "list_changed"),
        1,
        "the reader that got the failure is told of the list"
    );

    let log = Log::default();
    let platform = listing(&log, vec![(app, vec![7])]);
    let (feed, _) = feed_of(
        platform,
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());
    feed.list().await.unwrap();
    assert_eq!(
        counting(&log, "list_changed"),
        0,
        "a reader that waited for the first list reads it from the answer"
    );
}

#[tokio::test]
async fn an_unchanged_cycle_fetches_a_head_a_failed_fetch_left_missing() {
    let log = Log::default();
    let app = github("acme/app");
    let platform = listing(&log, vec![(app.clone(), vec![7])]);
    let (feed, refs) = feed_of(
        platform,
        project(&log),
        vec![info("origin", "https://github.com/acme/app")],
        zero(),
    );
    let _run = tokio::spawn(feed.clone().run());

    refs.refuse_refs();
    feed.manual().await; // lists 7, and the heads' fetch is refused
    assert_eq!(counting(&log, "heads_fetched"), 0, "the heads' fetch failed");

    refs.allow_refs();
    feed.manual().await; // the list is the same, and the missing head is fetched now
    assert_eq!(
        counting(&log, "heads_fetched"),
        1,
        "the unchanged cycle fetched the missing head"
    );
}
