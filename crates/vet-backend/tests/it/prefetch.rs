//! The details of pull requests are fetched ahead of the queue and kept, against a local server
//! that answers as GitHub does.
//!
//! The project's remote is an ssh URL that names `github.com/acme/app`, but a wrapper set as
//! `core.sshCommand` serves the fetch from a bare repository on disk: no network is touched, and
//! the platform's HTTP API is the fake server the tests point the backend at
//! (`Orchestrator::with_platform_endpoints`).

#![allow(clippy::disallowed_methods)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use vet_app::platform::RemoteKey;
use vet_app::ports::ChangesetStore;
use vet_backend::{Config, Orchestrator, open_store};
use vet_infra::platform::fake::{Canned, Fake, Seen};
use vet_protocol::{ChangesetKey, ProjectId, PullRequestInfo, Request, Response, Secret};

#[cfg(debug_assertions)]
use crate::common::background_ended;
use crate::common::{Client, git, init, write};

const NOW: &str = "2024-05-01T12:00:00Z";

/// One pull request the fake lists.
pub(crate) struct Pr {
    pub(crate) number: u64,
    /// The platform's `updatedAt`, which a later change moves.
    pub(crate) updated_at: String,
    /// What the discussion's `body` says.
    pub(crate) description: String,
    /// The commit `refs/pull/<number>/head` serves.
    pub(crate) head: String,
}

/// What the fake answers, and how it goes.
#[derive(Default)]
pub(crate) struct Mock {
    pub(crate) prs: Vec<Pr>,
    /// The base commit every pull request is compared with.
    pub(crate) base: String,
    /// How many discussions were answered.
    pub(crate) discussions: usize,
    /// How many discussion requests arrived, answered or not; counted before the delay, so a test
    /// can close the project while one is in flight.
    pub(crate) started: usize,
    /// After this many discussions every further one is refused as a rate limit.
    pub(crate) limit_after: Option<usize>,
    /// The status a refusal uses; 0 means a rate limit (429). A plain failure (500) is not gated
    /// by the client, so a test can tell the prefetch's own stop from the client's.
    pub(crate) refusal: u16,
    /// A pull request the platform does not have, which answers 404.
    pub(crate) missing: Option<u64>,
    /// How long answering a discussion takes, so a viewer's read can be seen to not wait for it.
    pub(crate) delay: Duration,
    /// The JSON answered for a commit's check runs once `checks_pending_calls` is spent; empty
    /// means a commit without runs.
    pub(crate) check_runs: String,
    /// The JSON answered while `checks_pending_calls` remains: a run still going.
    pub(crate) check_runs_pending: String,
    /// How many more check-run reads answer with `check_runs_pending` before `check_runs`.
    pub(crate) checks_pending_calls: usize,
    /// From this read on (1-based), every check-run read answers with `check_runs_pending`: the
    /// platform shows the new run only late.
    pub(crate) checks_pending_from: Option<usize>,
    /// How many check-run reads are refused with a rate limit (429, `Retry-After: 1`) first.
    pub(crate) checks_refusals: usize,
    /// How many check-run reads were answered, refusals included.
    pub(crate) checks_calls: usize,
}

fn user_json() -> String {
    let url = |what: &str| format!("https://api.github.com/users/ada/{what}");
    format!(
        r#"{{"login":"ada","id":1,"node_id":"U_1","avatar_url":"https://avatars.example/u/1","gravatar_id":"",
"url":"{}","html_url":"https://github.com/ada","followers_url":"{}","following_url":"{}","gists_url":"{}",
"starred_url":"{}","subscriptions_url":"{}","organizations_url":"{}","repos_url":"{}","events_url":"{}",
"received_events_url":"{}","type":"User","site_admin":false,"name":"Ada"}}"#,
        url(""),
        url("followers"),
        url("following"),
        url("gists"),
        url("starred"),
        url("subscriptions"),
        url("organizations"),
        url("repos"),
        url("events"),
        url("received_events"),
    )
}

/// One `PullRequestNode` of a search: every field the adapter reads.
fn pr_node(pr: &Pr) -> Value {
    json!({
        "number": pr.number,
        "title": format!("PR {}", pr.number),
        "url": format!("https://github.com/acme/app/pull/{}", pr.number),
        "isDraft": false,
        "updatedAt": pr.updated_at,
        "headRefName": "feature",
        "baseRefName": "main",
        "headRefOid": pr.head,
        "additions": 1,
        "deletions": 0,
        "totalCommentsCount": 0,
        "isCrossRepository": false,
        "headRepository": null,
        "reviewDecision": null,
        "mergeStateStatus": null,
        "author": { "login": "ada", "name": "Ada" },
        "labels": { "nodes": [] },
        "assignees": { "nodes": [] },
        "reviewRequests": { "nodes": [] },
        "viewerLatestReview": null,
        "latestOpinionatedReviews": { "nodes": [] },
        "reviewThreads": { "nodes": [] },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
    })
}

fn search_json(mock: &Mock) -> String {
    json!({ "data": { "search": {
        "nodes": mock.prs.iter().map(pr_node).collect::<Vec<_>>(),
        "pageInfo": { "hasNextPage": false, "endCursor": null }
    } } })
    .to_string()
}

fn detail_json(mock: &Mock, number: u64) -> Option<String> {
    let pr = mock.prs.iter().find(|p| p.number == number)?;
    let mut pull_request = pr_node(pr);
    pull_request["baseRefOid"] = json!(mock.base);
    pull_request["timelineItems"] = json!({ "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } });
    Some(json!({ "data": { "repository": { "pullRequest": pull_request } } }).to_string())
}

fn discussion_json(number: u64, description: &str) -> String {
    json!({ "data": { "repository": { "pullRequest": {
        "title": format!("PR {number}"),
        "url": format!("https://github.com/acme/app/pull/{number}"),
        "body": description,
        "isDraft": false,
        "state": "OPEN",
        "headRefName": "feature",
        "baseRefName": "main",
        "author": { "login": "ada", "name": "Ada" },
        "labels": { "nodes": [] },
        "reviewRequests": { "nodes": [] },
        "latestReviews": { "nodes": [] },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] },
        "comments": { "nodes": [], "pageInfo": { "hasPreviousPage": false, "startCursor": null } },
        "reviews": { "nodes": [], "pageInfo": { "hasPreviousPage": false, "startCursor": null } },
        "reviewThreads": { "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } }
    } } } })
    .to_string()
}

fn query_of(request: &Seen) -> String {
    serde_json::from_str::<Value>(&request.body)
        .ok()
        .and_then(|v| v["query"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn number_of(request: &Seen) -> u64 {
    serde_json::from_str::<Value>(&request.body)
        .ok()
        .and_then(|v| v["variables"]["number"].as_u64())
        .unwrap_or(0)
}

/// A server that answers the viewer, the list, one pull request's detail and its discussion.
fn github(mock: Arc<Mutex<Mock>>) -> Fake {
    Fake::start(move |request, _| {
        if request.path == "/user" {
            return Canned::json(&user_json()).with("Cache-Control", "no-store");
        }
        if request.path.starts_with("/user/teams") {
            return Canned::json("[]").with("Cache-Control", "no-store");
        }
        if request.path.contains("/check-runs") {
            let mut mock = mock.lock().unwrap();
            mock.checks_calls += 1;
            if mock.checks_refusals > 0 {
                mock.checks_refusals -= 1;
                return Canned::status(429, r#"{"message":"slow down"}"#).with("Retry-After", "1");
            }
            let pending =
                mock.checks_pending_calls > 0 || mock.checks_pending_from.is_some_and(|from| mock.checks_calls >= from);
            if mock.checks_pending_calls > 0 {
                mock.checks_pending_calls -= 1;
            }
            let body = if pending {
                mock.check_runs_pending.clone()
            } else {
                mock.check_runs.clone()
            };
            let body = if body.is_empty() {
                r#"{"total_count":0,"check_runs":[]}"#.to_owned()
            } else {
                body
            };
            return Canned::json(&body);
        }
        if request.path.contains("/commits/") && request.path.contains("/status") {
            return Canned::json(r#"{"state":"success","statuses":[]}"#);
        }
        if request.path.contains("/actions/runs") {
            return Canned::json(r#"{"total_count":0,"workflow_runs":[]}"#);
        }
        if request.path.starts_with("/repos/acme/app/pulls") {
            let (number, updated) = {
                let mock = mock.lock().unwrap();
                mock.prs
                    .iter()
                    .map(|p| (p.number, p.updated_at.clone()))
                    .max_by(|a, b| a.1.cmp(&b.1))
                    .unwrap_or((0, String::new()))
            };
            return Canned::json(&format!(r#"[{{"number":{number},"updated_at":"{updated}"}}]"#))
                .with("Cache-Control", "no-store");
        }
        let query = query_of(request);
        if query.contains("search(query:") {
            return Canned::json(&search_json(&mock.lock().unwrap()));
        }
        if query.contains("baseRefOid") {
            let found = detail_json(&mock.lock().unwrap(), number_of(request));
            return match found {
                Some(body) => Canned::json(&body),
                None => Canned::status(404, r#"{"message":"not found"}"#),
            };
        }
        if query.contains("body isDraft state") {
            let number = number_of(request);
            let (delay, description, refusal, missing) = {
                let mock = mock.lock().unwrap();
                (
                    mock.delay,
                    mock.prs
                        .iter()
                        .find(|p| p.number == number)
                        .map(|p| p.description.clone())
                        .unwrap_or_default(),
                    mock.refusal,
                    mock.missing,
                )
            };
            mock.lock().unwrap().started += 1;
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let mut mock = mock.lock().unwrap();
            let refused = mock.limit_after.is_some_and(|n| mock.discussions >= n);
            mock.discussions += 1;
            drop(mock);
            if missing == Some(number) {
                return Canned::status(404, r#"{"message":"not found"}"#);
            }
            return if refused {
                let status = if refusal == 0 { 429 } else { refusal };
                Canned::status(status, r#"{"message":"refused"}"#).with("Retry-After", "1")
            } else {
                Canned::json(&discussion_json(number, &description))
            };
        }
        Canned::status(500, r#"{"message":"unexpected query"}"#)
    })
}

/// A bare repository the wrapper serves: `main` at the base commit and `refs/pull/7/head` at the
/// head. Answers the two commits and the wrapper's path.
fn served(dir: &Path) -> (String, String, PathBuf) {
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    init(&work);
    write(&work, "a.txt", "base\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "base"]);
    let base = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["checkout", "-q", "-b", "feature"]);
    write(&work, "a.txt", "base\nfeature\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "feature"]);
    let head = git(&work, &["rev-parse", "HEAD"]);
    let bare = dir.join("served.git");
    git(dir, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    git(
        &work,
        &[
            "push",
            "-q",
            bare.to_str().unwrap(),
            "main",
            "feature:refs/pull/7/head",
            "feature:refs/pull/8/head",
        ],
    );
    let wrapper = dir.join("ssh-fake.sh");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec git-upload-pack '{}'\n", bare.display()),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    (base, head, wrapper)
}

/// A project whose `origin` is `github.com/acme/app`, fetched through `wrapper`.
fn project(wrapper: &Path) -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    write(repo.path(), "a.txt", "base\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-qm", "base"]);
    git(
        repo.path(),
        &["remote", "add", "origin", "ssh://git@github.com/acme/app.git"],
    );
    git(repo.path(), &["config", "core.sshCommand", wrapper.to_str().unwrap()]);
    // The `ssh` variant is invoked with the options gix uses; without it gix probes the command
    // with a feature check, which would run the wrapper's `git-upload-pack` and wait on its stdin.
    git(repo.path(), &["config", "ssh.variant", "ssh"]);
    repo
}

/// Two pull requests, 7 updated before 8, and the bare repository they are fetched from.
pub(crate) fn fixture() -> (Arc<Mutex<Mock>>, Fake, tempfile::TempDir, tempfile::TempDir) {
    let scratch = tempfile::tempdir().unwrap();
    let (base, head, wrapper) = served(scratch.path());
    let mock = Arc::new(Mutex::new(Mock {
        base,
        prs: vec![
            Pr {
                number: 7,
                updated_at: NOW.to_owned(),
                description: "first".to_owned(),
                head: head.clone(),
            },
            Pr {
                number: 8,
                updated_at: "2024-05-02T12:00:00Z".to_owned(),
                description: "second".to_owned(),
                head: head.clone(),
            },
        ],
        ..Mock::default()
    }));
    let fake = github(mock.clone());
    let repo = project(&wrapper);
    (mock, fake, repo, scratch)
}

pub(crate) fn config() -> Config {
    Config {
        debounce: Duration::from_millis(150),
        poll: Duration::from_millis(300),
        orphan_grace: Duration::from_secs(60),
        prefetch_pace: Duration::ZERO,
        checks_pace: Duration::from_millis(30),
        checks_settle: Duration::from_millis(40),
        ..Config::default()
    }
}

/// A backend whose platforms answer at `base`, with the credential the fake accepts.
pub(crate) async fn backend(data: &Path, base: &str) -> (Arc<Orchestrator>, Client) {
    backend_with(data, base, config()).await
}

/// A backend on `config`, whose platforms answer at `base`, with the credential the fake accepts.
pub(crate) async fn backend_with(data: &Path, base: &str, config: Config) -> (Arc<Orchestrator>, Client) {
    let orchestrator =
        Arc::new(Orchestrator::new(open_store(data).await.unwrap(), config).with_platform_endpoints(Some(base), None));
    let client = Client::new(&orchestrator);
    client
        .call(Request::SetCredentials {
            host: "github.com".into(),
            token: Secret("gho_secret".into()),
            validate: false,
        })
        .await
        .unwrap();
    (orchestrator, client)
}

async fn list(client: &Client, project: ProjectId) {
    let answer = client.call(Request::ListPullRequests { project }).await.unwrap();
    assert!(matches!(answer, Response::PullRequests { .. }), "{answer:?}");
}

/// The details of the pull request on view, as the Comments panel reads them.
async fn info(client: &Client, project: ProjectId) -> PullRequestInfo {
    match client.call(Request::PullRequestInfo { project }).await.unwrap() {
        Response::PullRequestInfo(info) => *info,
        other => panic!("{other:?}"),
    }
}

/// The descriptions the store keeps for `number`, polled until it has one or the wait runs out.
async fn stored_description(data: &Path, number: u64) -> Option<String> {
    let store = open_store(data).await.unwrap();
    for _ in 0..200 {
        if let Ok(Some(stored)) = store.stored_discussion(remote(), number).await {
            return Some(stored.discussion.description);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

/// The descriptions the store keeps for `number`, read once.
async fn stored_now(data: &Path, number: u64) -> Option<String> {
    let store = open_store(data).await.unwrap();
    store
        .stored_discussion(remote(), number)
        .await
        .ok()
        .flatten()
        .map(|stored| stored.discussion.description)
}

pub(crate) fn remote() -> RemoteKey {
    RemoteKey {
        host: "github.com".to_owned(),
        path: "acme/app".to_owned(),
    }
}

/// How many discussions the fake answered.
fn answered(mock: &Mutex<Mock>) -> usize {
    mock.lock().unwrap().discussions
}

/// How many discussion requests the fake received, those in flight included.
fn started(mock: &Mutex<Mock>) -> usize {
    mock.lock().unwrap().started
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn the_details_of_a_listed_pull_request_are_fetched_ahead_and_a_changed_one_again_but_not_an_unchanged_one() {
    let (mock, fake, repo, _scratch) = fixture();
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    assert_eq!(stored_description(repo.path(), 7).await.as_deref(), Some("first"));
    assert_eq!(stored_description(repo.path(), 8).await.as_deref(), Some("second"));
    assert_eq!(answered(&mock), 2, "both are new");

    // Nothing about #7 or #8 changed: neither is asked for again. The prefetch this listing
    // started has ended, and made no call.
    list(&client, project).await;
    background_ended(&client, project, "prefetch").await;
    assert_eq!(answered(&mock), 2, "an unchanged pull request costs no call");

    // #7 changed: it alone is fetched again.
    {
        let mut mock = mock.lock().unwrap();
        mock.prs[0].updated_at = "2024-05-03T12:00:00Z".to_owned();
        mock.prs[0].description = "third".to_owned();
    }
    list(&client, project).await;
    assert_eq!(stored_description(repo.path(), 7).await.as_deref(), Some("third"));
    assert_eq!(answered(&mock), 3, "a changed pull request is fetched");
}

#[tokio::test]
async fn a_pull_request_that_is_gone_does_not_stop_the_others() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        // #8 is gone; #9 and #7 are still fetched.
        mock.missing = Some(8);
        let head = mock.prs[0].head.clone();
        mock.prs.push(Pr {
            number: 9,
            updated_at: "2024-05-04T12:00:00Z".to_owned(),
            description: "third".to_owned(),
            head,
        });
    }
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;

    assert_eq!(stored_description(repo.path(), 9).await.as_deref(), Some("third"));
    assert_eq!(stored_description(repo.path(), 7).await.as_deref(), Some("first"));
    assert!(stored_now(repo.path(), 8).await.is_none());
    assert_eq!(
        answered(&mock),
        3,
        "the gone pull request is passed over, the account is not"
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn prefetching_stops_when_the_project_closes() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        let head = mock.prs[0].head.clone();
        mock.prs.push(Pr {
            number: 9,
            updated_at: "2024-05-04T12:00:00Z".to_owned(),
            description: "third".to_owned(),
            head,
        });
        // Slow platforms, so the prefetch is still running when the project closes.
        mock.delay = Duration::from_millis(200);
    }
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    // Close while the first call is in flight: wait until the platform has the request.
    for _ in 0..200 {
        if started(&mock) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    client.call(Request::CloseProject { project }).await.unwrap();
    // The task ends with the project: the call in flight when it closed is the last.
    background_ended(&client, project, "prefetch").await;
    for _ in 0..200 {
        if answered(&mock) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(
        answered(&mock),
        1,
        "the one in flight when the project closed, and no more"
    );
    assert!(stored_now(repo.path(), 9).await.is_none());
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn prefetching_stops_while_an_account_is_rate_limited() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.limit_after = Some(0);
        let head = mock.prs[0].head.clone();
        mock.prs.push(Pr {
            number: 9,
            updated_at: "2024-05-04T12:00:00Z".to_owned(),
            description: "third".to_owned(),
            head,
        });
    }
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    // The refusal stops the run, so the task ends; one that did not stop would still be running.
    background_ended(&client, project, "prefetch").await;

    assert_eq!(answered(&mock), 1, "the first refusal stops the rest");
    assert!(stored_now(repo.path(), 7).await.is_none());
    assert!(stored_now(repo.path(), 8).await.is_none());
    assert!(stored_now(repo.path(), 9).await.is_none());
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn prefetching_stops_when_the_account_fails() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.limit_after = Some(0);
        // A plain refusal: the client neither gates nor retries it, so only the prefetch's own
        // stop can keep the other pull requests from being asked for.
        mock.refusal = 400;
        let head = mock.prs[0].head.clone();
        mock.prs.push(Pr {
            number: 9,
            updated_at: "2024-05-04T12:00:00Z".to_owned(),
            description: "third".to_owned(),
            head,
        });
    }
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    // The failure stops the run, so the task ends; one that did not stop would still be running.
    background_ended(&client, project, "prefetch").await;

    assert_eq!(answered(&mock), 1, "the first failure stops the rest");
    assert!(stored_now(repo.path(), 7).await.is_none());
}

#[tokio::test]
async fn the_details_are_shown_from_the_store_after_a_restart_and_refreshed_after() {
    let (mock, fake, repo, _scratch) = fixture();
    let data = tempfile::tempdir().unwrap();
    let (_first, one) = backend(data.path(), &fake.base()).await;
    let project = one.open(repo.path()).await;
    list(&one, project).await;
    assert_eq!(stored_description(data.path(), 7).await.as_deref(), Some("first"));

    // The platform now says something else, and is slow about saying it.
    {
        let mut mock = mock.lock().unwrap();
        mock.prs[0].description = "second".to_owned();
        mock.delay = Duration::from_millis(600);
        mock.discussions = 0;
    }

    let (_second, two) = backend(data.path(), &fake.base()).await;
    let project = two.open(repo.path()).await;
    two.call(Request::OpenChangeset {
        project,
        changeset: ChangesetKey::PullRequest {
            remote: "github.com/acme/app".to_owned(),
            number: 7,
        },
    })
    .await
    .unwrap();

    // At once, before the platform's slow answer, the stored details are shown.
    let shown = info(&two, project).await;
    assert_eq!(shown.description, "first", "the stored copy, not the platform's");
    assert_eq!(answered(&mock), 0, "the platform had not answered a discussion yet");

    // And once the platform's answer is in, the shown details are the platform's.
    let mut refreshed = shown.description.clone();
    for _ in 0..200 {
        refreshed = info(&two, project).await.description;
        if refreshed == "second" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(refreshed, "second", "the platform's answer replaces the stored copy");
}
