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
use vet_protocol::{
    ChangesetKey, Event, ProjectId, PullRequestInfo, PullRequestRow, QueueStatus, Request, Response, Secret,
    ServerMessage,
};

#[cfg(debug_assertions)]
use crate::common::background_ended;
use crate::common::{Client, git, init, write};

const NOW: &str = "2024-05-01T12:00:00Z";

/// One pull request the fake lists.
#[derive(Clone)]
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
    /// How many unresolved review threads every pull request has; the search does not say.
    pub(crate) unresolved_threads: usize,
    /// Who asked for changes on every pull request, which holds the review decision at changes
    /// requested; the search does not say.
    pub(crate) asking_changes: Option<String>,
    /// The pull request whose merge the platform holds back; the search does not say.
    pub(crate) blocked: Option<u64>,
    /// Whether the search was asked for what is expensive to work out for every pull request:
    /// the review threads, the reviewers' verdicts, the review decision or the merge state.
    pub(crate) search_asked_expensive_fields: bool,
}

/// What the Azure DevOps fake answers, and how it goes.
#[derive(Default)]
struct AzureMock {
    /// The pull requests the list endpoint holds.
    prs: Vec<Pr>,
    /// How many single-pull-request reads arrived: one per prefetched discussion.
    discussions: usize,
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
        "author": { "login": "ada", "name": "Ada" },
        "labels": { "nodes": [] },
        "assignees": { "nodes": [] },
        "reviewRequests": { "nodes": [] },
        "viewerLatestReview": null,
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
    pull_request["reviewDecision"] = json!(mock.asking_changes.as_ref().map(|_| "CHANGES_REQUESTED"));
    pull_request["mergeStateStatus"] = json!((mock.blocked == Some(number)).then_some("BLOCKED"));
    pull_request["latestOpinionatedReviews"] = json!({ "nodes": mock.asking_changes.iter()
        .map(|name| json!({ "state": "CHANGES_REQUESTED", "author": { "login": name, "name": name } }))
        .collect::<Vec<_>>() });
    pull_request["reviewThreads"] = json!({ "nodes": (0..mock.unresolved_threads)
        .map(|_| json!({ "isResolved": false }))
        .collect::<Vec<_>>() });
    pull_request["baseRefOid"] = json!(mock.base);
    pull_request["timelineItems"] = json!({ "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } });
    Some(json!({ "data": { "repository": { "pullRequest": pull_request } } }).to_string())
}

fn thread_json(id: usize) -> Value {
    json!({
        "id": format!("T{id}"),
        "isResolved": false,
        "isOutdated": false,
        "path": "a.txt",
        "diffSide": "RIGHT",
        "line": 1,
        "startLine": null,
        "originalLine": 1,
        "originalStartLine": null,
        "subjectType": "LINE",
        "resolvedBy": null,
        "comments": { "nodes": [{
            "id": format!("C{id}"),
            "body": "why?",
            "createdAt": NOW,
            "lastEditedAt": null,
            "state": "SUBMITTED",
            "viewerDidAuthor": false,
            "author": { "login": "ada", "name": "Ada" },
            "originalCommit": null
        }], "pageInfo": { "hasNextPage": false, "endCursor": null } }
    })
}

fn discussion_json(mock: &Mock, number: u64, description: &str) -> String {
    json!({ "data": { "repository": { "pullRequest": {
        "title": format!("PR {number}"),
        "url": format!("https://github.com/acme/app/pull/{number}"),
        "body": description,
        "isDraft": false,
        "state": "OPEN",
        "headRefName": "feature",
        "baseRefName": "main",
        "reviewDecision": mock.asking_changes.as_ref().map(|_| "CHANGES_REQUESTED"),
        "mergeStateStatus": (mock.blocked == Some(number)).then_some("BLOCKED"),
        "author": { "login": "ada", "name": "Ada" },
        "labels": { "nodes": [] },
        "reviewRequests": { "nodes": [] },
        "latestReviews": { "nodes": mock.asking_changes.iter()
            .map(|name| json!({ "state": "CHANGES_REQUESTED", "author": { "login": name, "name": name } }))
            .collect::<Vec<_>>() },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] },
        "comments": { "nodes": [], "pageInfo": { "hasPreviousPage": false, "startCursor": null } },
        "reviews": { "nodes": [], "pageInfo": { "hasPreviousPage": false, "startCursor": null } },
        "reviewThreads": {
            "nodes": (0..mock.unresolved_threads).map(thread_json).collect::<Vec<_>>(),
            "pageInfo": { "hasNextPage": false, "endCursor": null }
        }
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
            let mut mock = mock.lock().unwrap();
            mock.search_asked_expensive_fields |= [
                "reviewThreads",
                "latestOpinionatedReviews",
                "reviewDecision",
                "mergeStateStatus",
            ]
            .iter()
            .any(|expensive| query.contains(expensive));
            return Canned::json(&search_json(&mock));
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
            let mut held = mock.lock().unwrap();
            let refused = held.limit_after.is_some_and(|n| held.discussions >= n);
            held.discussions += 1;
            drop(held);
            if missing == Some(number) {
                return Canned::status(404, r#"{"message":"not found"}"#);
            }
            return if refused {
                let status = if refusal == 0 { 429 } else { refusal };
                Canned::status(status, r#"{"message":"refused"}"#).with("Retry-After", "1")
            } else {
                Canned::json(&discussion_json(&mock.lock().unwrap(), number, &description))
            };
        }
        Canned::status(500, r#"{"message":"unexpected query"}"#)
    })
}

/// The list endpoint of the fake Azure DevOps repository.
const AZURE_PULLS: &str = "/contoso/shop/_apis/git/repositories/api/pullrequests";

/// The signed-in user of the fake, as `access.rs` reads it.
fn azure_profile() -> String {
    json!({ "authenticatedUser": {
        "id": "me-id",
        "providerDisplayName": "Ada Lovelace",
        "properties": { "Account": { "$type": "System.String", "$value": "ada@contoso.com" } }
    } })
    .to_string()
}

/// One pull request, as the Azure DevOps list and single-pull-request endpoints answer it. The
/// project carries no id, so no policy evaluation is asked for.
fn azure_pr(pr: &Pr) -> Value {
    json!({
        "pullRequestId": pr.number,
        "title": format!("PR {}", pr.number),
        "description": pr.description,
        "createdBy": { "id": "grace-id", "displayName": "Grace", "uniqueName": "grace@contoso.com", "isContainer": false },
        "creationDate": "2024-05-01T12:00:00Z",
        "isDraft": false,
        "labels": [],
        "sourceRefName": "refs/heads/feature",
        "targetRefName": "refs/heads/main",
        "status": "active",
        "url": format!("https://dev.azure.com/contoso/_apis/git/repositories/api/pullRequests/{}", pr.number),
        "repository": {
            "id": "r",
            "name": "api",
            "url": "https://dev.azure.com/contoso/_apis/git/repositories/api",
            "project": { "name": "Shop", "visibility": "private" }
        },
        "lastMergeSourceCommit": { "commitId": pr.head },
        "reviewers": []
    })
}

/// A server that answers like Azure DevOps: the viewer, the list of pull requests, and one pull
/// request's threads, iterations and statuses, all without any.
fn azure(mock: Arc<Mutex<AzureMock>>) -> Fake {
    Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap().to_lowercase();
        let path = path.as_str();
        if path.ends_with("/_apis/connectiondata") {
            return Canned::json(&azure_profile());
        }
        if path.ends_with("/_apis/identities") {
            return Canned::json(r#"{"count":0,"value":[]}"#);
        }
        if path == AZURE_PULLS {
            let mock = mock.lock().unwrap();
            let value: Vec<Value> = mock.prs.iter().map(azure_pr).collect();
            return Canned::json(&json!({ "count": value.len(), "value": value }).to_string());
        }
        if let Some(rest) = path.strip_prefix(&format!("{AZURE_PULLS}/")) {
            let mut mock = mock.lock().unwrap();
            let (number, tail) = match rest.split_once('/') {
                Some((number, tail)) => (number, Some(tail)),
                None => (rest, None),
            };
            let number: u64 = number.parse().unwrap_or(0);
            let Some(pr) = mock.prs.iter().find(|p| p.number == number).cloned() else {
                return Canned::status(404, r#"{"message":"not found"}"#);
            };
            return match tail {
                None => {
                    // A discussion reads the pull request itself once before its threads.
                    mock.discussions += 1;
                    Canned::json(&azure_pr(&pr).to_string())
                }
                Some(_) => Canned::json(r#"{"count":0,"value":[]}"#),
            };
        }
        Canned::status(404, r#"{"message":"unexpected query"}"#)
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

/// A project whose `origin` is `dev.azure.com/contoso/Shop/api`, fetched through `wrapper`.
fn azure_project(wrapper: &Path) -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    write(repo.path(), "a.txt", "base\n");
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-qm", "base"]);
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "ssh://git@ssh.dev.azure.com/v3/contoso/Shop/api",
        ],
    );
    git(repo.path(), &["config", "core.sshCommand", wrapper.to_str().unwrap()]);
    git(repo.path(), &["config", "ssh.variant", "ssh"]);
    repo
}

/// `count` pull requests the fake Azure DevOps lists, all new, and the bare repository their heads
/// are fetched from.
fn azure_fixture(count: u64) -> (Arc<Mutex<AzureMock>>, Fake, tempfile::TempDir, tempfile::TempDir) {
    let scratch = tempfile::tempdir().unwrap();
    let (_base, head, wrapper) = served(scratch.path());
    let mock = Arc::new(Mutex::new(AzureMock {
        prs: (0..count)
            .map(|i| Pr {
                number: 7 + i,
                updated_at: format!("2024-06-{:02}T12:00:00Z", i + 1),
                description: format!("pr {}", i + 1),
                head: head.clone(),
            })
            .collect(),
        ..AzureMock::default()
    }));
    let fake = azure(mock.clone());
    let repo = azure_project(&wrapper);
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

/// A backend whose Azure DevOps answers at `base`, with the credential the fake accepts.
async fn azure_backend(data: &Path, base: &str) -> (Arc<Orchestrator>, Client) {
    let orchestrator = Arc::new(
        Orchestrator::new(open_store(data).await.unwrap(), config()).with_platform_endpoints(None, Some(base)),
    );
    let client = Client::new(&orchestrator);
    client
        .call(Request::SetCredentials {
            host: "dev.azure.com/contoso".into(),
            token: Secret("pat_secret".into()),
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

/// The rows the list answers with, by pull request number.
async fn rows(client: &Client, project: ProjectId) -> Vec<(u64, PullRequestRow)> {
    match client.call(Request::ListPullRequests { project }).await.unwrap() {
        Response::PullRequests { rows, .. } => rows
            .into_iter()
            .map(|row| match &row.key {
                ChangesetKey::PullRequest { number, .. } => (u64::from(*number), row),
                other => panic!("{other:?}"),
            })
            .collect(),
        other => panic!("{other:?}"),
    }
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
async fn a_row_is_listed_without_its_threads_and_has_them_once_its_details_are_fetched() {
    let (mock, fake, repo, _scratch) = fixture();
    {
        let mut mock = mock.lock().unwrap();
        mock.unresolved_threads = 2;
        mock.asking_changes = Some("Linus".to_owned());
        mock.blocked = Some(7);
    }
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    let before = rows(&client, project).await;

    assert!(
        !mock.lock().unwrap().search_asked_expensive_fields,
        "the search asks for what is expensive to work out for every pull request"
    );
    for (number, row) in &before {
        assert_eq!(
            (row.status, row.unresolved_threads, row.changes_requested.len()),
            (None, 0, 0),
            "#{number}: the search does not say"
        );
    }

    // The prefetch reads the details, and says that the list shows more of them now.
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(ServerMessage::Event {
                event: Event::PullRequestsChanged { .. },
            }) = client.inbox.recv().await
            {
                break;
            }
        }
    })
    .await
    .expect("the list was not announced to have changed");

    for (number, row) in rows(&client, project).await {
        assert_eq!(row.unresolved_threads, 2, "#{number}");
        if number == 7 {
            assert_eq!(row.status, Some(QueueStatus::Blocked));
        } else {
            let asked: Vec<_> = row.changes_requested.iter().map(|c| c.reviewer.as_str()).collect();
            assert_eq!(
                (row.status, asked),
                (Some(QueueStatus::ChangesRequested), vec!["Linus"])
            );
        }
    }
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

/// Replaces the mock's list with `count` pull requests from 7 on, so a listing longer than the
/// prefetch limit can be made.
fn many_prs(mock: &Mutex<Mock>, count: u64) {
    let mut mock = mock.lock().unwrap();
    let head = mock.prs[0].head.clone();
    mock.prs = (0..count)
        .map(|i| Pr {
            number: 7 + i,
            updated_at: format!("2024-06-{:02}T12:00:00Z", i + 1),
            description: format!("pr {}", i + 1),
            head: head.clone(),
        })
        .collect();
}

/// A platform whose list leaves the expensive fields out has no prefetch limit: without the
/// details the rows cannot show their state at all.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_platform_that_leaves_details_apart_has_every_listed_pull_request_prefetched() {
    let (mock, fake, repo, _scratch) = fixture();
    many_prs(&mock, 25);
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    background_ended(&client, project, "prefetch").await;

    assert_eq!(
        answered(&mock),
        25,
        "GitHub leaves the details out of its list, so every listed pull request is fetched"
    );
}

/// A platform whose list already carries the details keeps the limit: more would only cost calls
/// for what the list says anyway.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_platform_whose_list_carries_the_details_still_stops_at_the_limit() {
    let (mock, fake, repo, _scratch) = azure_fixture(25);
    let data = tempfile::tempdir().unwrap();
    let (_handler, client) = azure_backend(data.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    background_ended(&client, project, "prefetch").await;

    assert_eq!(
        mock.lock().unwrap().discussions,
        20,
        "the limit still bounds a platform whose list carries the details"
    );
}

/// A run longer than the coalescing window tells the window before it is through, so the chips
/// the details add appear while the queue is looked at rather than minutes later.
#[cfg(debug_assertions)]
#[tokio::test]
async fn the_changed_list_is_announced_while_a_long_prefetch_still_runs() {
    let (mock, fake, repo, _scratch) = fixture();
    many_prs(&mock, 25);
    // A detail the list leaves out, so reading each discussion changes what the queue shows.
    mock.lock().unwrap().unresolved_threads = 1;
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        prefetch_pace: Duration::from_millis(20),
        prefetch_announce: Duration::from_millis(60),
        ..config()
    };
    let (_handler, mut client) = backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;

    // Every change since the last announcement is told at most once per window, so a run that
    // takes many windows tells the window more than once: only the last may wait for the run.
    let mut announced = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        announced += drain_announcements(&mut client.inbox);
        let (started, live) = client.handler.background_counts(project, "prefetch");
        if started > 0 && live == 0 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "the prefetch did not end");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    announced += drain_announcements(&mut client.inbox);

    assert!(
        announced >= 2,
        "the changed list was announced only once, when the run was through"
    );
}

/// How many announcements that the listed pull requests changed are waiting for the connection.
#[cfg(debug_assertions)]
fn drain_announcements(inbox: &mut tokio::sync::mpsc::Receiver<ServerMessage>) -> usize {
    std::iter::from_fn(|| inbox.try_recv().ok())
        .filter(|m| {
            matches!(
                m,
                ServerMessage::Event {
                    event: Event::PullRequestsChanged { .. }
                }
            )
        })
        .count()
}
