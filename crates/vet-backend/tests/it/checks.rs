//! The checks of commits are those of a pull request on GitHub, and of no other changeset. The
//! backend refreshes the checks of the changeset on view itself and pushes them, against a local
//! server that answers as GitHub does.

#![allow(clippy::disallowed_methods)] // The fake GitHub's requests are answered from a thread of its own.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use vet_backend::Config;
use vet_protocol::{
    ChangesetKey, CiState, CommitChecks, Event, PipelineAction, ProjectId, Request, Response, ServerMessage,
};

#[cfg(debug_assertions)]
use crate::common::background_ended;
use crate::common::{Client, handler, repo_with};
#[cfg(debug_assertions)]
use crate::common::{background_live, background_started};
use crate::prefetch::{Mock, backend, backend_with, config, fixture};

/// A successful check run of the head commit, as GitHub's REST API answers it.
const CHECKS_SUCCESS: &str = r#"{"total_count":1,"check_runs":[{"id":11,"name":"test (ubuntu)","status":"completed","conclusion":"success","started_at":"2024-05-01T12:00:00Z","completed_at":"2024-05-01T12:00:05Z","html_url":null,"app":{"slug":"github-actions"}}]}"#;

/// The same run while it is going.
const CHECKS_PENDING: &str = r#"{"total_count":1,"check_runs":[{"id":11,"name":"test (ubuntu)","status":"in_progress","conclusion":null,"started_at":"2024-05-01T12:00:00Z","completed_at":null,"html_url":null,"app":{"slug":"github-actions"}}]}"#;

fn pull_request(number: u32) -> ChangesetKey {
    ChangesetKey::PullRequest {
        remote: "github.com/acme/app".to_owned(),
        number,
    }
}

/// Opens the project, shows the review screen and opens pull request 7 on view, so the checks of
/// the changeset on view are the backend's.
async fn open_review(client: &Client, repo: &Path) -> ProjectId {
    let project = client.open(repo).await;
    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();
    client
        .call(Request::OpenChangeset {
            project,
            changeset: pull_request(7),
        })
        .await
        .unwrap();
    project
}

/// The next `ChecksChanged` event of `project`, waiting up to `within`.
async fn next_checks(
    inbox: &mut Receiver<ServerMessage>,
    project: ProjectId,
    within: Duration,
) -> Option<(Vec<CommitChecks>, Option<String>, Option<u32>)> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        let Ok(message) = tokio::time::timeout(left, inbox.recv()).await else {
            return None;
        };
        if let Some(ServerMessage::Event {
            event:
                Event::ChecksChanged {
                    project: at,
                    checks,
                    error,
                    limited_until,
                },
        }) = message
            && at == project
        {
            return Some((checks, error, limited_until));
        }
    }
}

fn checks_calls(mock: &Mutex<Mock>) -> usize {
    mock.lock().unwrap().checks_calls
}

fn head(mock: &Mutex<Mock>) -> String {
    mock.lock().unwrap().prs[0].head.clone()
}

#[tokio::test]
async fn a_changeset_that_is_not_a_pull_request_has_no_checks_and_no_logs() {
    let repo = repo_with(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;

    let answer = client
        .call(Request::Checks {
            project,
            commits: vec!["abc123".to_owned()],
        })
        .await
        .unwrap();
    assert_eq!(
        answer,
        Response::Checks {
            checks: Vec::new(),
            error: None,
            limited_until: None,
        }
    );

    let refused = client
        .call(Request::CheckLog {
            project,
            run: "11".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(refused.message.contains("not a pull request"), "{refused}");
    assert_eq!(refused.kind, vet_protocol::ErrorKind::Invalid);

    let steps = client
        .call(Request::PipelineSteps {
            project,
            check: "11".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(steps.message.contains("not a pull request"), "{steps}");
    let changed = client
        .call(Request::ChangePipeline {
            project,
            action: PipelineAction::Cancel { run: "5".to_owned() },
        })
        .await
        .unwrap_err();
    assert!(changed.message.contains("not a pull request"), "{changed}");
}

#[tokio::test]
async fn the_checks_of_the_changeset_on_view_are_pushed_without_being_asked_for() {
    let (mock, fake, repo, _scratch) = fixture();
    mock.lock().unwrap().check_runs = CHECKS_SUCCESS.to_owned();
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = open_review(&client, repo.path()).await;

    let (checks, error, limited) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the backend pushes the checks with no request");
    assert_eq!(error, None);
    assert_eq!(limited, None);
    assert_eq!(checks.len(), 1, "{checks:?}");
    assert_eq!(checks[0].commit, head(&mock));
    assert_eq!(checks[0].state, Some(CiState::Success));
    assert_eq!(checks[0].runs.len(), 1);
    assert_eq!(checks_calls(&mock), 1);
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn the_checks_are_read_again_while_a_run_still_runs_and_stop_once_it_is_done() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.check_runs_pending = CHECKS_PENDING.to_owned();
        mock.check_runs = CHECKS_SUCCESS.to_owned();
        mock.checks_pending_calls = 1;
    }
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = open_review(&client, repo.path()).await;

    let (first, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the first round");
    assert_eq!(first[0].state, Some(CiState::Pending));
    let (second, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("a second round while it runs");
    assert_eq!(second[0].state, Some(CiState::Success));
    assert_eq!(checks_calls(&mock), 2, "asked again while a run went on");

    // Nothing is running any more, so the task ends; its end is the marker that it asked for
    // nothing more.
    background_ended(&client, project, "checks").await;
    assert_eq!(checks_calls(&mock), 2, "nothing more once it is done");
}

#[tokio::test]
async fn the_checks_are_kept_fresh_while_the_queue_shows_the_changeset() {
    let (mock, fake, repo, _scratch) = fixture();
    mock.lock().unwrap().check_runs = CHECKS_SUCCESS.to_owned();
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
    // The queue shows the selected pull request's checks; the review screen is not shown.
    client
        .call(Request::OpenChangeset {
            project,
            changeset: pull_request(7),
        })
        .await
        .unwrap();

    let (checks, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the queue's checks are pushed too");
    assert_eq!(checks[0].commit, head(&mock));
    assert_eq!(checks[0].state, Some(CiState::Success));
    assert_eq!(checks_calls(&mock), 1);
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn showing_the_review_screen_again_replaces_the_run_that_was_going() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.check_runs_pending = CHECKS_PENDING.to_owned();
        mock.checks_pending_calls = usize::MAX;
    }
    let slow = Config {
        checks_pace: Duration::from_millis(300),
        ..config()
    };
    let (_handler, mut client) = backend_with(repo.path(), &fake.base(), slow).await;
    let project = open_review(&client, repo.path()).await;
    let _ = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("a round");
    assert_eq!(checks_calls(&mock), 1);

    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();

    let (again, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the new run pushes at once");
    assert_eq!(again[0].state, Some(CiState::Pending));
    assert_eq!(checks_calls(&mock), 2, "the new run's first round");

    // The run it replaced is gone: only one is left. The new run's own next round (300 ms apart)
    // then reaches the platform exactly once.
    background_live(&client, project, "checks", 1).await;
    let (next, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the new run reads again");
    assert_eq!(next[0].state, Some(CiState::Pending));
    assert_eq!(checks_calls(&mock), 3, "one run, not two");
}

#[tokio::test]
async fn the_checks_wait_for_a_rate_limit_the_account_knows_and_resume_after_it() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.check_runs = CHECKS_SUCCESS.to_owned();
        mock.checks_refusals = 1;
    }
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = open_review(&client, repo.path()).await;

    let (none, error, until) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the limit is told");
    assert!(none.is_empty(), "{none:?}");
    assert!(error.is_some());
    assert!(until.is_some(), "the limit's end is told");
    let (later, error, limited_until) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("it resumes after the limit");
    assert_eq!(error, None);
    assert_eq!(limited_until, None);
    assert_eq!(later[0].state, Some(CiState::Success));
    assert_eq!(checks_calls(&mock), 2, "one refusal, then the answer");
}

#[tokio::test]
async fn changing_a_run_makes_the_checks_be_read_again() {
    let (mock, fake, repo, _scratch) = fixture();
    mock.lock().unwrap().check_runs = CHECKS_SUCCESS.to_owned();
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = open_review(&client, repo.path()).await;
    let _ = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("a round");
    assert_eq!(checks_calls(&mock), 1);

    client
        .call(Request::ChangePipeline {
            project,
            action: PipelineAction::RerunAll { run: "5".to_owned() },
        })
        .await
        .unwrap();

    let (again, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the checks are read again after the action");
    assert_eq!(again[0].state, Some(CiState::Success));
    assert!(checks_calls(&mock) >= 2, "the action made the checks be read again");
}

#[tokio::test]
async fn the_checks_are_read_again_shortly_after_an_action_that_the_platform_shows_late() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.check_runs = CHECKS_SUCCESS.to_owned();
        mock.check_runs_pending = CHECKS_PENDING.to_owned();
        // The old, finished run shows on the read right after the action; the new run only on the
        // read after that.
        mock.checks_pending_from = Some(3);
    }
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = open_review(&client, repo.path()).await;
    let _ = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the finished run first");
    assert_eq!(checks_calls(&mock), 1);

    client
        .call(Request::ChangePipeline {
            project,
            action: PipelineAction::RerunAll { run: "5".to_owned() },
        })
        .await
        .unwrap();

    // The read right after the action still sees the finished run; the settle read sees the new
    // one. Without the settle read the run would stop after the first and the new run would never
    // show until the next push.
    let (stale, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the read right after the action");
    assert_eq!(stale[0].state, Some(CiState::Success), "the platform had not caught up");
    let (fresh, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the settle read shows the new run");
    assert_eq!(fresh[0].state, Some(CiState::Pending));
    assert!(checks_calls(&mock) >= 3, "the settle read reached the new run");
}

#[tokio::test]
async fn a_revision_move_on_a_pull_request_reads_the_checks_again() {
    let (mock, fake, repo, _scratch) = fixture();
    mock.lock().unwrap().check_runs = CHECKS_SUCCESS.to_owned();
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = open_review(&client, repo.path()).await;
    let _ = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("a round");
    assert_eq!(checks_calls(&mock), 1);

    client
        .call(Request::SetProjectFilter {
            project,
            filter: vet_protocol::ReviewFilter {
                roots: vec!["a.txt".into()],
                ..Default::default()
            },
        })
        .await
        .unwrap();

    let (again, ..) = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("the checks are read again for the new revision");
    assert_eq!(again[0].state, Some(CiState::Success));
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_working_tree_starts_no_checks_run() {
    let repo = repo_with(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();
    client
        .call(Request::SetProjectFilter {
            project,
            filter: vet_protocol::ReviewFilter {
                roots: vec!["a.txt".into()],
                ..Default::default()
            },
        })
        .await
        .unwrap();
    client.call(Request::WaitForQuiet { project }).await.unwrap();

    assert_eq!(background_started(&client, project, "checks"), 0);
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn closing_the_project_stops_the_checks() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.check_runs_pending = CHECKS_PENDING.to_owned();
        mock.checks_pending_calls = usize::MAX;
    }
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = open_review(&client, repo.path()).await;
    let _ = next_checks(&mut client.inbox, project, Duration::from_secs(5))
        .await
        .expect("a round");

    client.call(Request::CloseProject { project }).await.unwrap();
    // The task ends with the project; its end is the marker that it reads no more.
    background_ended(&client, project, "checks").await;
    assert_eq!(checks_calls(&mock), 1, "nothing runs after the project closes");
}
