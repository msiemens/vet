//! The sync of the pull request on view with its platform, whoever asks for it: at most one runs and
//! one waits, a request is answered by a run that began after it, and it ends with the project.
//! Against a local server that answers as GitHub does; a debug build only, which counts the runs.

#![cfg(debug_assertions)]
#![allow(clippy::disallowed_methods)] // The fake GitHub's requests are answered from a thread of its own.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vet_backend::{Config, Orchestrator};
use vet_infra::platform::fake::Fake;
use vet_protocol::{ChangesetKey, Event, ProjectId, Request, Secret, ServerMessage, TaskKind, TaskState, Verdict};

use crate::common::{Client, background_live, background_started};
use crate::prefetch::{Mock, backend_with, config, fixture, remote};

/// A platform that takes a while to answer a discussion, so a sync is seen to be in flight.
const SLOW: Duration = Duration::from_millis(400);

/// A backend with pull request 7 on view and nothing left to do, whose platform answers slowly.
struct Rig {
    client: Client,
    project: ProjectId,
    mock: Arc<Mutex<Mock>>,
    fake: Fake,
    _orchestrator: Arc<Orchestrator>,
    _dirs: [tempfile::TempDir; 3],
}

impl Rig {
    async fn start(config: Config) -> Self {
        let (mock, fake, repo, scratch) = fixture();
        let data = tempfile::tempdir().unwrap();
        let (orchestrator, client) = backend_with(data.path(), &fake.base(), config).await;
        let project = client.open(repo.path()).await;
        let rig = Self {
            client,
            project,
            mock,
            fake,
            _orchestrator: orchestrator,
            _dirs: [repo, scratch, data],
        };
        rig.open_pull_request().await;
        rig.rest().await;
        rig.mock.lock().unwrap().delay = SLOW;
        rig
    }

    async fn open_pull_request(&self) {
        self.client
            .call(Request::OpenChangeset {
                project: self.project,
                changeset: ChangesetKey::PullRequest {
                    remote: remote().to_string(),
                    number: 7,
                },
            })
            .await
            .unwrap();
    }

    /// Waits until the project has nothing running and the platform has been asked nothing new for
    /// a while.
    async fn rest(&self) {
        let mut last = usize::MAX;
        for _ in 0..100 {
            self.quiet().await;
            let now = self.fake.requests().len();
            if now == last {
                return;
            }
            last = now;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        panic!("the platform was still being asked");
    }

    async fn quiet(&self) {
        self.client
            .call(Request::WaitForQuiet { project: self.project })
            .await
            .unwrap();
    }

    async fn sync_threads(&self) {
        self.client
            .call(Request::SyncThreads { project: self.project })
            .await
            .unwrap();
    }

    /// How often the project's syncs started.
    fn syncs(&self) -> usize {
        background_started(&self.client, self.project, "sync")
    }

    /// How many discussion requests the platform received.
    fn asked(&self) -> usize {
        self.mock.lock().unwrap().started
    }

    /// Waits until the platform has received more than `before` discussion requests.
    async fn until_asked(&self, before: usize) {
        let waiting = async {
            while self.asked() <= before {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), waiting)
            .await
            .expect("the platform was never asked");
    }

    /// The events the client has heard and not yet read.
    fn heard(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(message) = self.client.inbox.try_recv() {
            if let ServerMessage::Event { event } = message {
                events.push(event);
            }
        }
        events
    }
}

#[tokio::test]
async fn five_quick_triggers_give_at_most_two_syncs() {
    let rig = Rig::start(config()).await;
    let before = rig.syncs();

    tokio::join!(
        rig.sync_threads(),
        rig.sync_threads(),
        rig.sync_threads(),
        rig.sync_threads(),
        rig.sync_threads(),
    );

    let runs = rig.syncs() - before;
    assert!((1..=2).contains(&runs), "{runs} syncs ran for five requests");
}

#[tokio::test]
async fn choosing_the_pull_request_on_view_again_syncs_it_once_more() {
    let rig = Rig::start(config()).await;
    let before = rig.syncs();

    rig.open_pull_request().await;
    rig.quiet().await;

    assert_eq!(rig.syncs() - before, 1);
}

#[tokio::test]
async fn an_account_change_syncs_the_pull_request_of_that_host_once() {
    let rig = Rig::start(config()).await;
    let before = rig.syncs();
    let sign_in = async |host: &str, token: &str| {
        rig.client
            .call(Request::SetCredentials {
                host: host.into(),
                token: Secret(token.into()),
                validate: false,
            })
            .await
            .unwrap();
        rig.quiet().await;
    };

    sign_in("dev.azure.com/contoso", "pat_other").await;
    assert_eq!(rig.syncs(), before, "a host the pull request is not on");

    sign_in("github.com", "gho_secret_2").await;
    assert_eq!(rig.syncs() - before, 1);
}

#[tokio::test]
async fn waiting_for_quiet_waits_for_a_sync_in_flight() {
    let rig = Rig::start(config()).await;
    let before = rig.asked();

    // Opening the pull request answers before its sync is done.
    rig.open_pull_request().await;
    rig.until_asked(before).await;
    assert_eq!(rig.client.handler.background_counts(rig.project, "sync").1, 1);

    rig.quiet().await;

    assert_eq!(rig.client.handler.background_counts(rig.project, "sync").1, 0);
}

#[tokio::test]
async fn a_post_is_followed_by_a_sync_that_began_after_it() {
    let rig = Rig::start(config()).await;
    let before = rig.fake.requests().len();

    rig.client
        .call(Request::SubmitReview {
            project: rig.project,
            verdict: Verdict::Approve,
            body: None,
        })
        .await
        .unwrap();

    let asked = rig.fake.requests().split_off(before);
    let posted = asked
        .iter()
        .position(|r| r.body.contains("submitPullRequestReview("))
        .expect("the review was posted");
    assert!(
        asked[posted..].iter().any(|r| r.body.contains("body isDraft state")),
        "the post was not followed by a read of the discussion: {asked:#?}"
    );
}

#[tokio::test]
async fn a_sync_for_a_source_no_longer_on_view_announces_nothing() {
    let mut rig = Rig::start(config()).await;
    rig.heard();
    let before = rig.asked();

    tokio::join!(rig.sync_threads(), async {
        rig.until_asked(before).await;
        rig.client
            .call(Request::OpenChangeset {
                project: rig.project,
                changeset: ChangesetKey::WorkingTree,
            })
            .await
            .unwrap();
    });
    rig.quiet().await;

    let told: Vec<_> = rig
        .heard()
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                Event::PlatformStatus { .. }
                    | Event::ThreadsChanged { .. }
                    | Event::PullRequestsChanged { .. }
                    | Event::Notification { .. }
            )
        })
        .collect();
    assert!(told.is_empty(), "{told:#?}");
}

/// The sync tasks the client heard of, as `(state)`.
fn sync_tasks(events: &[Event]) -> Vec<TaskState> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Task(t) if t.kind == TaskKind::Sync => Some(t.state.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_failing_sync_shows_a_failed_task_and_a_later_success_clears_it() {
    let mut rig = Rig::start(config()).await;
    rig.mock.lock().unwrap().delay = Duration::ZERO;
    {
        let mut mock = rig.mock.lock().unwrap();
        mock.refusal = 500;
        mock.limit_after = Some(0);
    }
    rig.heard();

    rig.sync_threads().await;
    let failed = sync_tasks(&rig.heard());
    assert!(
        matches!(failed.last(), Some(TaskState::Failed { .. })),
        "a failed sync is shown: {failed:?}"
    );

    rig.mock.lock().unwrap().limit_after = None;
    rig.sync_threads().await;
    assert_eq!(sync_tasks(&rig.heard()).last(), Some(&TaskState::Finished));
}

#[tokio::test]
async fn closing_the_project_ends_a_slow_sync_within_the_bound() {
    let rig = Rig::start(Config {
        shutdown_bound: Duration::from_millis(100),
        ..config()
    })
    .await;
    rig.mock.lock().unwrap().delay = Duration::from_secs(2);
    let before = rig.asked();

    let (_, closed_in) = tokio::join!(rig.sync_threads(), async {
        rig.until_asked(before).await;
        let closing = Instant::now();
        rig.client
            .call(Request::CloseProject { project: rig.project })
            .await
            .unwrap();
        closing.elapsed()
    });

    assert!(closed_in < Duration::from_secs(1), "closing took {closed_in:?}");
    background_live(&rig.client, rig.project, "sync", 0).await;
}
