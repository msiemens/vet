//! The cycle over the ports, with a platform that lists and discusses, git remotes that record
//! what they were asked, a session that records what the cycle did to it, and a background that
//! notes when the two phases that continue after the list are started.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::fetch::Credentials;
use crate::platform::{
    CheckLog, CommitChecks, Discussion, FakeHosting, Health, PlatformError, PlatformTraits, PullRequest,
    PullRequestDetail, PullRequestListing, PullRequestQuery, QueueTab, Viewer, Watched, pr, rows, viewer,
};
use crate::ports::RefUpdate;
use crate::pull_request::tests::{Anonymous, Remotes, fetching, fetching_into, github, info, names};
use crate::pull_request::{ListedPullRequest, PullRequestRefs, RepositoryStatus};
use crate::session::tests::MemStore;

/// What the cycle did, in order, as each double records it.
type Log = Arc<Mutex<Vec<String>>>;

fn recorded(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// The platform side of the ports, over a platform of the test's and the recording git remotes.
#[derive(Clone)]
struct Side<P> {
    platform: P,
    health: Arc<Health>,
    known: Arc<PullRequestCache>,
    fetch: Arc<Fetch<Remotes>>,
    /// The hosts the viewer has an account for; the tests' remotes are all known by name.
    accounts: Vec<String>,
}

impl<P: Platform + Clone + Send + Sync + 'static> Hostings for Side<P> {
    type Fetcher = Remotes;

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

    fn fetch(&self) -> &Fetch<Remotes> {
        &self.fetch
    }

    fn credentials(&self) -> impl Credentials + Send + Sync + '_ {
        Anonymous
    }
}

/// A platform the cycle lists and discusses, recording each of those in the log.
#[derive(Clone)]
struct Fake {
    listed: Vec<(RemoteKey, Vec<PullRequest>)>,
    failing: Vec<RemoteKey>,
    traits: PlatformTraits,
    log: Log,
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
        if self.failing.contains(remote) {
            return Err(PlatformError::Rejected("refused".to_owned()));
        }
        Ok(PullRequestListing {
            pull_requests: self
                .listed
                .iter()
                .find(|(r, _)| r == remote)
                .map(|(_, prs)| prs.clone())
                .unwrap_or_default(),
            truncated: false,
        })
    }

    async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
        if self.failing.contains(remote) {
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
        self.log.lock().unwrap().push("discussion".to_owned());
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

/// The project side of the ports, recording the prune, the head fetch and the announcements.
#[derive(Clone, Default)]
struct ProjectFake {
    log: Log,
    on_view: Option<(RemoteKey, u64)>,
}

impl Project for ProjectFake {
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
        };
        self.log.lock().unwrap().push(name.to_owned());
    }
}

/// The background of the ports: it records when each phase is started and drops its work, so the
/// order of the starts is what the log shows.
#[derive(Clone, Default)]
struct BackgroundFake {
    log: Log,
}

impl Background for BackgroundFake {
    fn fetch_heads(&self, work: impl Future<Output = ()> + Send + 'static) {
        self.log.lock().unwrap().push("heads".to_owned());
        drop(work);
    }

    fn prefetch(&self, work: impl Future<Output = ()> + Send + 'static) {
        self.log.lock().unwrap().push("prefetch".to_owned());
        drop(work);
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
            listed: vec![(app, vec![pr(7)])],
            failing: vec![],
            traits: PlatformTraits::github(),
            log: log.clone(),
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
    let background = BackgroundFake { log: log.clone() };

    refresh(
        &side,
        MemStore::default(),
        &project,
        &background,
        || 1_000,
        Prefetch {
            pace: Duration::ZERO,
            announce: Duration::ZERO,
        },
    )
    .await
    .unwrap();

    assert_eq!(recorded(&log), ["list", "prune", "heads", "prefetch"]);
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
            listed: vec![],
            failing: vec![],
            traits: PlatformTraits::github(),
            log: Log::default(),
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
