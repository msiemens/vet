//! The checks of commits: statuses by iteration, policy evaluations, builds and their logs.

use serde_json::{Value, json};

use super::*;

fn status_of(id: i32, iteration: Option<i32>, state: &str, name: &str) -> Value {
    let mut status = json!({
        "id": id, "state": state, "context": { "genre": "ci", "name": name },
        "targetUrl": format!("https://ci.contoso.com/{id}"),
        "creationDate": "2024-05-01T12:00:00Z", "updatedDate": "2024-05-01T12:05:00Z",
    });
    if let Some(iteration) = iteration {
        status["iterationId"] = json!(iteration);
    }
    status
}

fn evaluation(id: &str, status: &str, name: &str, blocking: bool, build: Option<i64>) -> Value {
    json!({
        "evaluationId": id, "status": status,
        "startedDate": "2024-05-01T12:00:00Z", "completedDate": "2024-05-01T12:10:00Z",
        "context": build.map(|b| json!({ "buildId": b })),
        "configuration": {
            "id": 1, "isBlocking": blocking,
            "type": { "id": "t", "displayName": name, "url": "u" },
            "settings": { "displayName": if build.is_some() { json!(name) } else { Value::Null } },
        },
    })
}

fn iterations() -> Value {
    let iteration = |id: i32, source: &str| {
        json!({ "id": id, "reason": "push", "createdDate": "2024-04-01T10:00:00Z",
            "sourceRefCommit": { "commitId": source }, "commonRefCommit": { "commitId": "base" } })
    };
    json!({ "count": 2, "value": [iteration(1, "a1"), iteration(2, "a2")] })
}

fn pull_request() -> Value {
    let mut doc = Doc::new(7).json();
    doc["lastMergeSourceCommit"] = json!({ "commitId": "a2" });
    doc["repository"]["project"]["id"] = json!("proj-1");
    doc
}

/// A server for pull request 7, with the statuses and policy evaluations it is given. A commit
/// that no pull request has is `zzz`.
fn building(statuses: Vec<Value>, evaluations: Vec<Value>) -> Fake {
    Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap().to_ascii_lowercase();
        let list = |items: &[Value]| Canned::json(&json!({ "count": items.len(), "value": items }).to_string());
        if path.ends_with("/_apis/connectiondata") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path.ends_with("/pullrequestquery") {
            let asked: Value = serde_json::from_str(&request.body).unwrap();
            let commit = asked["queries"][0]["items"][0].as_str().unwrap().to_owned();
            let found = if commit == "zzz" {
                json!([])
            } else {
                json!([{ "pullRequestId": 7, "status": "active" }])
            };
            Canned::json(&json!({ "queries": [], "results": [{ commit: found }] }).to_string())
        } else if path.ends_with("/pullrequests/7") {
            Canned::json(&pull_request().to_string())
        } else if path.ends_with("/iterations") {
            Canned::json(&iterations().to_string())
        } else if path.ends_with("/statuses") {
            list(&statuses)
        } else if path == "/contoso/shop/_apis/policy/evaluations" {
            assert!(
                request
                    .path
                    .contains("artifactId=vstfs%3A%2F%2F%2FCodeReview%2FCodeReviewId%2Fproj-1%2F7"),
                "{}",
                request.path
            );
            list(&evaluations)
        } else {
            panic!("unexpected request {} {}", request.method, request.path)
        }
    })
}

fn statuses() -> Vec<Value> {
    vec![
        status_of(1, Some(1), "failed", "build"),
        status_of(2, Some(1), "succeeded", "build"),
        status_of(3, Some(2), "pending", "build"),
        status_of(4, Some(2), "succeeded", "lint"),
        status_of(5, None, "succeeded", "sonar"),
    ]
}

fn evaluations() -> Vec<Value> {
    vec![
        evaluation("e1", "rejected", "Build validation", true, Some(55)),
        evaluation("e2", "approved", "Minimum number of reviewers", true, None),
        evaluation("e3", "rejected", "Docs", false, Some(56)),
        evaluation("e4", "notApplicable", "Work item linking", true, None),
    ]
}

fn summary(checks: &vet_app::platform::CommitChecks) -> Vec<(&str, &str, CiState, bool)> {
    checks
        .checks
        .iter()
        .map(|c| (c.id.as_str(), c.name.as_str(), c.state, c.log))
        .collect()
}

#[tokio::test]
async fn the_head_has_the_statuses_of_its_iteration_and_of_the_pull_request_and_the_policies() {
    set_now(NOW);
    let fake = building(statuses(), evaluations());

    let checks = client(&fake).checks(&remote(), "a2").await.unwrap();

    assert_eq!(
        summary(&checks),
        [
            ("status:3", "ci/build", CiState::Pending, false),
            ("status:4", "ci/lint", CiState::Success, false),
            ("status:5", "ci/sonar", CiState::Success, false),
            ("build:55", "Build validation", CiState::Failure, true),
            ("policy:e2", "Minimum number of reviewers", CiState::Success, false),
            ("build:56", "Docs", CiState::Neutral, true),
        ],
        "the newest status of each context; a policy that is not blocking only advises; one that does not apply is left out"
    );
    assert_eq!(checks.state(), Some(CiState::Pending));
    assert_eq!(
        checks.url.as_deref(),
        Some("https://dev.azure.com/contoso/Shop/_git/api/pullrequest/7")
    );
    let build = &checks.checks[3];
    assert_eq!(
        build.url.as_deref(),
        Some("https://dev.azure.com/contoso/Shop/_build/results?buildId=55")
    );
    assert_eq!(
        (build.started, build.finished),
        (Some(1_714_564_800), Some(1_714_565_400))
    );
    let lint = &checks.checks[1];
    assert_eq!(lint.url.as_deref(), Some("https://ci.contoso.com/4"));
}

/// A server that lists pull requests 1 to 3 of a project with an id, and the policy evaluations
/// of each of them as `evaluations` has them, by number.
fn listing_with_policies(evaluations: Vec<(i32, Vec<Value>)>) -> Fake {
    Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap().to_ascii_lowercase();
        let list = |items: &[Value]| Canned::json(&json!({ "count": items.len(), "value": items }).to_string());
        if path.ends_with("/_apis/connectiondata") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path == PULLS.to_ascii_lowercase() {
            let docs: Vec<Value> = (1..=3)
                .map(|n| {
                    let mut doc = Doc::new(n).json();
                    doc["repository"]["project"]["id"] = json!("proj-1");
                    doc
                })
                .collect();
            list(&docs)
        } else if path.ends_with("/statuses") || path.ends_with("/threads") {
            list(&[])
        } else if path == "/contoso/shop/_apis/policy/evaluations" {
            let number: i32 = request.path.rsplit("%2F").next().unwrap().parse().unwrap();
            let found = evaluations.iter().find(|(n, _)| *n == number);
            list(found.map_or(&[][..], |(_, e)| &e[..]))
        } else {
            panic!("unexpected request {} {}", request.method, request.path)
        }
    })
}

#[tokio::test]
async fn a_pull_request_is_blocked_by_a_blocking_policy_that_failed_and_by_nothing_else() {
    set_now(NOW);
    let fake = listing_with_policies(vec![
        (
            1,
            vec![
                evaluation("a", "approved", "Minimum number of reviewers", true, None),
                evaluation("b", "rejected", "Build validation", true, Some(55)),
            ],
        ),
        (
            2,
            vec![
                evaluation("c", "rejected", "Docs", false, Some(56)),
                evaluation("d", "running", "Minimum number of reviewers", true, None),
                evaluation("e", "queued", "Build validation", true, Some(57)),
                evaluation("f", "notApplicable", "Work item linking", true, None),
            ],
        ),
        (3, vec![evaluation("g", "broken", "Required reviewers", true, None)]),
    ]);

    let found = client(&fake)
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap()
        .pull_requests;

    let blocked = |n: u64| found.iter().find(|p| p.number == n).unwrap().blocked;
    assert_eq!(
        (blocked(1), blocked(2), blocked(3)),
        (true, false, true),
        "a failed advisory policy, and ones still running, hold nothing back"
    );
}

#[tokio::test]
async fn the_policies_of_a_pull_request_are_asked_for_with_its_checks_and_threads() {
    set_now(NOW);
    let fake = listing_with_policies(vec![]);
    let client = client(&fake);

    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();

    assert_eq!(
        requests_to(&fake, "/policy/evaluations"),
        3,
        "once per pull request, then kept"
    );
}

#[tokio::test]
async fn an_earlier_commit_has_only_the_statuses_of_the_iteration_it_was_the_source_of() {
    set_now(NOW);
    let fake = building(statuses(), evaluations());
    let client = client(&fake);

    let checks = client.checks(&remote(), "a1").await.unwrap();

    assert_eq!(summary(&checks), [("status:2", "ci/build", CiState::Success, false)]);
    assert!(checks.is_final());
    let asked = fake.requests().len();
    client.checks(&remote(), "a1").await.unwrap();
    assert_eq!(fake.requests().len(), asked, "checks that are all done are kept");
}

#[tokio::test]
async fn a_commit_that_no_pull_request_has_has_no_checks_and_the_pull_request_of_a_commit_is_asked_for_once() {
    set_now(NOW);
    let fake = building(statuses(), evaluations());
    let client = client(&fake);

    let none = client.checks(&remote(), "zzz").await.unwrap();
    client.checks(&remote(), "a2").await.unwrap();
    client.checks(&remote(), "a2").await.unwrap();

    assert!(none.checks.is_empty() && none.url.is_none());
    let queries = fake
        .requests()
        .iter()
        .filter(|r| r.path.to_ascii_lowercase().contains("pullrequestquery"))
        .count();
    assert_eq!(queries, 2, "once for each commit");
}

#[tokio::test]
async fn a_token_that_may_not_read_policies_still_has_the_statuses() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap().to_ascii_lowercase();
        if path.ends_with("/_apis/connectiondata") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path.ends_with("/pullrequestquery") {
            Canned::json(&json!({ "results": [{ "a2": [{ "pullRequestId": 7, "status": "active" }] }] }).to_string())
        } else if path.ends_with("/pullrequests/7") {
            Canned::json(&pull_request().to_string())
        } else if path.ends_with("/iterations") {
            Canned::json(&iterations().to_string())
        } else if path.ends_with("/statuses") {
            Canned::json(&json!({ "value": [status_of(4, Some(2), "succeeded", "lint")] }).to_string())
        } else {
            Canned::status(
                403,
                r#"{"message":"TF400813: The user is not authorized to access this resource."}"#,
            )
        }
    });

    let checks = client(&fake).checks(&remote(), "a2").await.unwrap();

    assert_eq!(summary(&checks), [("status:4", "ci/lint", CiState::Success, false)]);
}

const TIMELINE: &str = "/contoso/shop/_apis/build/builds/55/timeline/";

fn building_with_log(log: Canned) -> Fake {
    let log = std::sync::Mutex::new(Some(log));
    Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap().to_ascii_lowercase();
        if path.ends_with("/_apis/connectiondata") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path == TIMELINE {
            Canned::json(
                &json!({ "records": [
                    { "id": "r1", "name": "Job", "type": "Job", "result": "failed", "order": 1, "log": { "id": 3, "type": "Container", "url": "u" } },
                    { "id": "r3", "name": "Build", "type": "Task", "result": "succeeded", "order": 2, "log": { "id": 7, "type": "Container", "url": "u" } },
                    { "id": "r2", "name": "Run tests", "type": "Task", "result": "failed", "order": 3, "log": { "id": 9, "type": "Container", "url": "u" } },
                ]})
                .to_string(),
            )
        } else if path == "/contoso/shop/_apis/build/builds/55/logs/9" {
            log.lock().unwrap().take().expect("asked once")
        } else {
            panic!("unexpected request {} {}", request.method, request.path)
        }
    })
}

const LOG: &str =
    "starting\n2024-05-01T12:01:00Z running 3 tests\n2024-05-01T12:01:02Z ##[error]test two failed\ndone\n";

#[tokio::test]
async fn the_log_of_a_build_is_that_of_the_step_that_failed_with_the_line_of_its_first_error() {
    set_now(NOW);
    for reply in [Canned::json(&serde_json::to_string(LOG).unwrap()), Canned::json(LOG)] {
        let fake = building_with_log(reply);

        let log = client(&fake).check_log(&remote(), "build:55").await.unwrap();

        assert_eq!(log.text, LOG);
        let failure = log.failure.expect("where it failed");
        assert_eq!((failure.line, failure.step.as_deref()), (2, Some("Run tests")));
    }
}

#[tokio::test]
async fn only_a_build_has_a_log() {
    set_now(NOW);
    let fake = building(vec![], vec![]);

    let error = client(&fake).check_log(&remote(), "status:3").await.unwrap_err();

    assert!(matches!(error, PlatformError::NotFound(_)), "{error:?}");
    assert!(fake.requests().is_empty());
}
