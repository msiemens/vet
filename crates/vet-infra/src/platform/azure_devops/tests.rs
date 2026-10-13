//! The client against a local server that answers like Azure DevOps.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vet_app::platform::{
    ChangesAsked, ChangesRequest, Credential, CredentialSource, ReviewDecision, ReviewRequest, ReviewState,
    ReviewVerdict, Token, ViewerReview, list_rows, parse_remote,
};

use super::*;
use crate::http::client_builder;
use crate::platform::fake::{Canned, Fake, Seen, Signed};

mod checks;
mod discussion;
mod limits;

const NOW: i64 = 1_700_000_000;

thread_local! {
    static CLOCK: Cell<i64> = const { Cell::new(NOW) };
}

fn now() -> i64 {
    CLOCK.with(Cell::get)
}

fn set_now(at: i64) {
    CLOCK.with(|c| c.set(at));
}

fn client(fake: &Fake) -> AzureDevOpsClient<Signed> {
    client_with(fake, Signed(Some("secret"), CredentialSource::Pat))
}

fn client_with<C: CredentialProvider + Sync>(fake: &Fake, credentials: C) -> AzureDevOpsClient<C> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = client_builder(TIMEOUT).no_proxy().build().unwrap();
    AzureDevOpsClient {
        credentials,
        http: OnceLock::from(Ok(http)),
        endpoints: Some(Endpoints {
            api: fake.base(),
            identities: fake.base(),
        }),
        page_size: PAGE_SIZE,
        now,
        state: Mutex::default(),
        limits: Arc::default(),
    }
}

fn remote() -> RemoteKey {
    RemoteKey {
        host: "dev.azure.com".to_owned(),
        path: "contoso/Shop/api".to_owned(),
    }
}

fn all_open() -> PullRequestQuery {
    PullRequestQuery::new(QueueTab::AllOpen)
}

const PULLS: &str = "/contoso/Shop/_apis/git/repositories/api/pullrequests";

/// An `IdentityRef`: the address is the unique name, and the display name is the part before the `@`,
/// capitalized (`ada@contoso.com` is "Ada").
fn identity(id: &str, unique_name: &str, container: bool) -> Value {
    let local = unique_name.split('@').next().unwrap();
    let display: String = local
        .chars()
        .enumerate()
        .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
        .collect();
    json!({ "id": id, "displayName": display, "uniqueName": unique_name, "isContainer": container })
}

fn reviewer(who: Value, vote: i64) -> Value {
    let mut reviewer = who;
    reviewer["vote"] = json!(vote);
    reviewer
}

/// The user the tests sign in as.
fn ada() -> Value {
    identity("me-id", "ada@contoso.com", false)
}

fn team() -> Value {
    json!({ "id": "g1", "displayName": "[Shop]\\Reviewers", "uniqueName": "vstfs:///Classification/TeamProject/1", "isContainer": true })
}

struct Doc {
    number: i32,
    author: Value,
    head: String,
    reviewers: Vec<Value>,
}

impl Doc {
    fn new(number: i32) -> Self {
        Self {
            number,
            author: identity("grace-id", "grace@contoso.com", false),
            head: format!("head{number}"),
            reviewers: Vec::new(),
        }
    }

    fn json(&self) -> Value {
        json!({
            "pullRequestId": self.number,
            "title": format!("PR {}", self.number),
            "description": format!("Why PR {} exists.\n\n- one\n- two", self.number),
            "createdBy": self.author,
            "creationDate": format!("2024-05-01T12:00:{:02}Z", self.number % 60),
            "isDraft": self.number % 2 == 0,
            "labels": [{ "name": "performance" }],
            "sourceRefName": "refs/heads/feature",
            "targetRefName": "refs/heads/main",
            "status": "active",
            "url": "https://dev.azure.com/contoso/_apis/git/repositories/r/pullRequests/1",
            "repository": {
                "id": "r", "name": "api", "url": "https://dev.azure.com/contoso/_apis/git/repositories/r",
                "project": { "name": "Shop", "visibility": "private" }
            },
            "lastMergeSourceCommit": { "commitId": self.head },
            "reviewers": self.reviewers,
        })
    }
}

fn list_of(docs: &[Doc]) -> String {
    json!({ "count": docs.len(), "value": docs.iter().map(Doc::json).collect::<Vec<_>>() }).to_string()
}

fn query_has(request: &Seen, pair: &str) -> bool {
    request
        .path
        .split_once('?')
        .is_some_and(|(_, q)| q.split('&').any(|p| p == pair))
}

fn no_value() -> String {
    r#"{"count":0,"value":[]}"#.to_owned()
}

/// What an organization's `connectionData` says of the signed-in user.
fn profile() -> String {
    json!({ "authenticatedUser": {
        "id": "me-id",
        "providerDisplayName": "Ada Lovelace",
        "properties": { "Account": { "$type": "System.String", "$value": "ada@contoso.com" } }
    } })
    .to_string()
}

fn status(id: i32, state: &str, name: &str) -> Value {
    json!({ "id": id, "state": state, "context": { "genre": "ci", "name": name }, "iterationId": 1 })
}

fn thread(status: &str, comment_type: &str) -> Value {
    json!({ "status": status, "comments": [{ "commentType": comment_type, "content": "x" }] })
}

/// A server that holds `docs` as the repository's active pull requests, with `statuses`
/// and `threads` for each of them.
fn repository(docs: Vec<Doc>, statuses: Vec<Value>, threads: Vec<Value>) -> Fake {
    Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            if query_has(request, "identityIds=me-id") {
                Canned::json(r#"{"count":1,"value":[{"id":"me-id","memberOf":["vssgp.A","vssgp.B"]}]}"#)
            } else {
                Canned::json(
                    &json!({ "count": 2, "value": [
                        { "id": "g1", "providerDisplayName": "[Shop]\\Reviewers", "isContainer": true },
                        { "id": "g2", "providerDisplayName": "[Shop]\\Other", "isContainer": true },
                    ]})
                    .to_string(),
                )
            }
        } else if path == PULLS {
            let number = |name: &str, default: usize| {
                request
                    .path
                    .split(['?', '&'])
                    .find_map(|p| {
                        p.strip_prefix(name)
                            .or_else(|| p.strip_prefix(&name.replacen('$', "%24", 1)))
                    })
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default)
            };
            let page: Vec<Doc> = docs
                .iter()
                .skip(number("$skip=", 0))
                .take(number("$top=", 100))
                .map(|d| Doc {
                    number: d.number,
                    author: d.author.clone(),
                    head: d.head.clone(),
                    reviewers: d.reviewers.clone(),
                })
                .collect();
            Canned::json(&list_of(&page))
        } else if path.ends_with("/statuses") {
            Canned::json(&json!({ "count": statuses.len(), "value": statuses }).to_string())
        } else if path.ends_with("/threads") {
            Canned::json(&json!({ "count": threads.len(), "value": threads }).to_string())
        } else {
            panic!("unexpected request {}", request.path)
        }
    })
}

fn small_pages(client: &mut AzureDevOpsClient<Signed>) {
    client.page_size = 2;
}

fn requests_to(fake: &Fake, what: &str) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.split('?').next().unwrap().ends_with(what))
        .count()
}

#[tokio::test]
async fn a_list_follows_the_pages_and_reads_what_the_queue_shows() {
    set_now(NOW);
    let mut first = Doc::new(1);
    first.reviewers = vec![
        reviewer(ada(), 0),
        reviewer(team(), 0),
        reviewer(identity("x", "linus@contoso.com", false), 10),
    ];
    let fake = repository(
        vec![first, Doc::new(2), Doc::new(3)],
        vec![
            status(1, "succeeded", "build"),
            status(2, "failed", "build"),
            status(3, "pending", "lint"),
        ],
        vec![
            thread("active", "text"),
            thread("pending", "text"),
            thread("active", "system"),
            thread("fixed", "text"),
            thread("closed", "text"),
        ],
    );
    let mut client = client(&fake);
    small_pages(&mut client);
    let listing = client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(!listing.truncated);
    let found = listing.pull_requests;

    assert_eq!(
        found.iter().map(|p| p.number).collect::<Vec<_>>(),
        [3, 2, 1],
        "newest first"
    );
    let first = found.iter().find(|p| p.number == 1).unwrap();
    assert_eq!(
        (
            first.title.as_str(),
            first.author.as_str(),
            first.source_branch.as_str(),
            first.target_branch.as_str(),
            first.head.as_str(),
            first.draft,
            first.ci,
            first.unresolved_threads,
            first.url.as_str(),
        ),
        (
            "PR 1",
            "grace@contoso.com",
            "feature",
            "main",
            "head1",
            false,
            // The newest status of a context counts; one that failed fails the pull request.
            Some(CiState::Failure),
            2,
            "https://dev.azure.com/contoso/Shop/_git/api/pullrequest/1",
        )
    );
    assert_eq!(
        first.author_name, "Grace",
        "the author is shown by display name, and compared by address"
    );
    assert_eq!(first.labels, ["performance"]);
    assert_eq!(first.comments, 4, "resolved threads count, the system's own do not");
    assert_eq!(
        first.review_requests,
        [
            ReviewRequest::User("ada@contoso.com".to_owned()),
            ReviewRequest::Team("[Shop]\\Reviewers".to_owned())
        ],
        "a reviewer who has voted is no request"
    );
    assert!(found.iter().find(|p| p.number == 2).unwrap().draft);
    assert_eq!(
        (first.decision, found.iter().find(|p| p.number == 3).unwrap().decision),
        (ReviewDecision::Approved, ReviewDecision::Undecided),
        "the reviewers' votes add up"
    );
}

#[tokio::test]
async fn a_list_that_stops_at_the_most_it_holds_says_it_was_cut_short_whatever_it_narrows_to() {
    set_now(NOW);
    let docs: Vec<Doc> = (1..=MAX_LISTED as i32 + 1).map(Doc::new).collect();
    let fake = repository(docs, vec![], vec![]);
    let many = client(&fake);

    let listing = many.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(listing.truncated);
    assert_eq!(listing.pull_requests.len(), MAX_LISTED);

    // The pull request waiting for the viewer may be on a page that was never asked for.
    let waiting = many
        .list_pull_requests(&remote(), &PullRequestQuery::new(QueueTab::WaitingForMe))
        .await
        .unwrap();
    assert!(waiting.truncated);

    let small = repository(vec![Doc::new(1), Doc::new(2)], vec![], vec![]);
    let listing = client(&small).list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(!listing.truncated);
}

#[tokio::test]
async fn what_is_known_of_the_pull_requests_is_not_asked_again_until_it_is_old() {
    set_now(NOW);
    let fake = repository(vec![Doc::new(1), Doc::new(2), Doc::new(3)], vec![], vec![]);
    let mut client = client(&fake);
    small_pages(&mut client);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(requests_to(&fake, "/statuses"), 3, "one per pull request");
    assert_eq!(requests_to(&fake, "/threads"), 3, "one per pull request");
    let before = fake.requests().len();

    // The same list again: the list is asked for, what is known of the pull requests is not.
    set_now(NOW + 60);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(fake.requests().len() - before, 2);

    set_now(NOW + EXTRAS_MAX_AGE + 1);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(requests_to(&fake, "/statuses"), 6, "asked again after a while");
}

#[tokio::test]
async fn a_pull_request_was_updated_by_its_newest_thread_or_status_and_the_list_is_ordered_by_that() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap();
        let of = |n: u32| path.contains(&format!("/pullRequests/{n}/"));
        let list = |value: Value| Canned::json(&json!({ "value": value }).to_string());
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path == PULLS {
            Canned::json(&list_of(&[Doc::new(1), Doc::new(2), Doc::new(3)]))
        } else if path.ends_with("/threads") && of(1) {
            list(json!([
                // What the system writes when someone votes.
                { "id": 1, "lastUpdatedDate": "2024-06-01T00:00:00Z",
                  "comments": [{ "commentType": "system", "content": "Ada voted 10" }] },
                { "id": 2, "lastUpdatedDate": "2024-05-02T00:00:00Z", "status": "active",
                  "comments": [{ "commentType": "text", "content": "x" }] },
                { "id": 3, "lastUpdatedDate": "2024-07-01T00:00:00Z", "isDeleted": true, "comments": [] },
            ]))
        } else if path.ends_with("/statuses") && of(2) {
            let mut posted = status(1, "succeeded", "build");
            posted["creationDate"] = json!("2024-05-10T00:00:00Z");
            posted["updatedDate"] = json!("2024-05-15T00:00:00Z");
            list(json!([posted]))
        } else {
            Canned::json(&no_value())
        }
    });
    let found = client(&fake)
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap()
        .pull_requests;
    let updated: Vec<_> = found.iter().map(|p| (p.number, p.updated_at)).collect();
    assert_eq!(
        updated,
        [
            (1, 1_717_200_000), // the vote, not the deleted thread after it
            (2, 1_715_731_200), // the status
            (3, 1_714_564_803), // nothing since it was created
        ]
    );
}

#[tokio::test]
async fn what_is_known_of_a_pull_request_is_asked_again_when_its_source_branch_moved() {
    set_now(NOW);
    let head = Arc::new(Mutex::new("a".to_owned()));
    let served = head.clone();
    let fake = Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path == PULLS {
            let mut doc = Doc::new(1);
            doc.head = served.lock().unwrap().clone();
            Canned::json(&list_of(&[doc]))
        } else {
            Canned::json(&no_value())
        }
    });
    let client = client(&fake);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    *head.lock().unwrap() = "b".to_owned();
    set_now(NOW + 10);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(requests_to(&fake, "/statuses"), 2);
}

#[tokio::test]
async fn the_tabs_and_the_rule_decide_who_is_waiting_for_the_viewer() {
    set_now(NOW);
    let mut asked = Doc::new(1);
    asked.reviewers = vec![reviewer(ada(), 0)];
    let mut by_team = Doc::new(2);
    by_team.reviewers = vec![reviewer(team(), 0)];
    let mut approved = Doc::new(3);
    approved.reviewers = vec![reviewer(ada(), 10), reviewer(team(), 0)];
    let mut mine = Doc::new(4);
    mine.author = ada();
    let unrelated = Doc::new(5);
    let fake = repository(vec![asked, by_team, approved, mine, unrelated], vec![], vec![]);
    let client = client(&fake);

    let numbers = |rows: Vec<vet_app::platform::PullRequestRow>| rows.iter().map(|r| r.number).collect::<Vec<_>>();
    let waiting = list_rows(&client, &remote(), &PullRequestQuery::new(QueueTab::WaitingForMe))
        .await
        .unwrap()
        .rows;
    assert_eq!(
        numbers(waiting),
        [2, 1],
        "asked directly, and asked of a team the viewer is in"
    );

    let mine = list_rows(&client, &remote(), &PullRequestQuery::new(QueueTab::Mine))
        .await
        .unwrap()
        .rows;
    assert_eq!(numbers(mine), [4]);
    let search = fake
        .requests()
        .into_iter()
        .rev()
        .find(|r| r.path.split('?').next() == Some(PULLS))
        .unwrap();
    assert!(query_has(&search, "searchCriteria.creatorId=me-id"), "{}", search.path);
    assert!(query_has(&search, "searchCriteria.status=active"), "{}", search.path);

    let assigned = list_rows(&client, &remote(), &PullRequestQuery::new(QueueTab::Assigned))
        .await
        .unwrap()
        .rows;
    assert!(assigned.is_empty(), "Azure DevOps has no assignees");
}

#[tokio::test]
async fn a_vote_is_a_review_of_the_head() {
    set_now(NOW);
    let mut approved = Doc::new(1);
    approved.reviewers = vec![reviewer(ada(), 5)];
    let mut rejected = Doc::new(2);
    rejected.reviewers = vec![reviewer(ada(), -5)];
    let fake = repository(vec![approved, rejected], vec![], vec![]);
    let client = client(&fake);
    let found = client
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap()
        .pull_requests;
    let review = |n: u64| found.iter().find(|p| p.number == n).unwrap().viewer_review.clone();
    assert_eq!(
        review(1),
        Some(ViewerReview {
            verdict: ReviewVerdict::Approved,
            commit: Some("head1".to_owned())
        })
    );
    assert_eq!(review(2).map(|r| r.verdict), Some(ReviewVerdict::ChangesRequested));
    let rows = list_rows(&client, &remote(), &all_open()).await.unwrap().rows;
    assert!(rows.iter().all(|r| matches!(r.review, ReviewState::Reviewed(_))));
}

#[tokio::test]
async fn a_rejection_and_a_wait_for_the_author_are_told_apart_by_reviewer() {
    set_now(NOW);
    let mut held = Doc::new(1);
    let mut declined = identity("x", "x@contoso.com", false);
    declined["hasDeclined"] = json!(true);
    held.reviewers = vec![
        reviewer(ada(), 10),
        reviewer(
            json!({ "id": "l", "displayName": "Linus T", "uniqueName": "linus@contoso.com", "isContainer": false }),
            -10,
        ),
        reviewer(team(), -5),
        reviewer(declined, -10),
    ];
    let fake = repository(vec![held, Doc::new(3)], vec![], vec![]);
    let found = client(&fake)
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap()
        .pull_requests;
    let of = |n: u64| found.iter().find(|p| p.number == n).unwrap().changes_requested.clone();

    assert_eq!(
        of(1),
        [
            ChangesRequest {
                reviewer: "Linus T".to_owned(),
                how: ChangesAsked::Rejected
            },
            ChangesRequest {
                reviewer: "[Shop]\\Reviewers".to_owned(),
                how: ChangesAsked::WaitingForAuthor
            },
        ],
        "named as people see them; who declined to review holds nothing back"
    );
    assert_eq!(of(3), []);
}

#[tokio::test]
async fn a_token_that_may_not_read_threads_or_groups_still_lists() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == PULLS {
            Canned::json(&list_of(&[Doc::new(1)]))
        } else if path.ends_with("/statuses") {
            Canned::json(&json!({ "value": [status(1, "succeeded", "build")] }).to_string())
        } else {
            Canned::status(
                403,
                r#"{"message":"TF400813: The user is not authorized to access this resource."}"#,
            )
        }
    });
    let found = client(&fake)
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap()
        .pull_requests;
    assert_eq!((found[0].ci, found[0].unresolved_threads), (Some(CiState::Success), 0));
}

#[tokio::test]
async fn a_pull_request_comes_with_its_iterations_as_pushes_and_the_merge_base() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" || path.ends_with("/statuses") || path.ends_with("/threads") {
            Canned::json(&no_value())
        } else if path.ends_with("/iterations") {
            let iteration = |id: i32, source: &str, reason: &str, date: &str| {
                json!({ "id": id, "reason": reason, "createdDate": date,
                    "sourceRefCommit": { "commitId": source },
                    "commonRefCommit": { "commitId": format!("base{id}") } })
            };
            Canned::json(
                &json!({ "count": 4, "value": [
                    iteration(1, "a1", "create", "2024-04-01T10:00:00Z"),
                    iteration(2, "a2", "push", "2024-04-02T10:00:00Z"),
                    iteration(3, "a3", "forcePush", "2024-04-03T10:00:00Z"),
                    // The target moved: the same source commit again.
                    iteration(4, "a3", "push", "2024-04-04T10:00:00Z"),
                ]})
                .to_string(),
            )
        } else if path == format!("{PULLS}/7") {
            let mut doc = Doc::new(7).json();
            doc["head"] = json!("a3");
            doc["lastMergeSourceCommit"] = json!({ "commitId": "a3" });
            doc["forkSource"] = json!({ "name": "refs/heads/feature", "objectId": "a3", "repository": {
                "id": "f", "name": "api", "url": "https://dev.azure.com/x", "project": { "name": "Shop", "visibility": "private" } } });
            Canned::json(&doc.to_string())
        } else {
            Canned::status(
                404,
                r#"{"message":"TF401180: The requested pull request was not found."}"#,
            )
        }
    });
    let client = client(&fake);
    let detail = client.pull_request(&remote(), 7).await.unwrap();
    assert_eq!(detail.base, "base4");
    assert_eq!(detail.pull_request.fork.as_deref(), Some("api"));
    assert_eq!(detail.pull_request.head, "a3");
    let pushes: Vec<_> = detail.pushes.iter().map(|p| (p.head.as_str(), p.forced)).collect();
    assert_eq!(pushes, [("a1", false), ("a2", false), ("a3", true)]);
    assert_eq!(detail.pushes[1].at, Some(1_712_052_000));

    let error = client.pull_request(&remote(), 8).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::NotFound(
            "contoso/Shop/api#8 was not found on dev.azure.com/contoso, or the token cannot see it".to_owned()
        )
    );
}

#[tokio::test]
async fn a_personal_token_is_the_password_of_a_user_with_no_name_and_a_cli_token_a_bearer() {
    set_now(NOW);
    let fake = repository(vec![], vec![], vec![]);
    client(&fake).list_pull_requests(&remote(), &all_open()).await.unwrap();
    // base64 of ":secret".
    assert!(
        fake.requests()
            .iter()
            .all(|r| r.header("authorization") == Some("Basic OnNlY3JldA=="))
    );

    let fake = repository(vec![], vec![], vec![]);
    client_with(&fake, Signed::cli("secret"))
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap();
    assert!(
        fake.requests()
            .iter()
            .all(|r| r.header("authorization") == Some("Bearer secret"))
    );
}

#[tokio::test]
async fn the_viewer_is_the_profile_and_the_groups_they_are_in() {
    set_now(NOW);
    let fake = repository(vec![], vec![], vec![]);
    let client = client(&fake);
    let viewer = client.viewer(&remote()).await.unwrap();
    assert_eq!(
        viewer,
        Viewer {
            login: "ada@contoso.com".to_owned(),
            teams: vec!["[Shop]\\Reviewers".to_owned(), "[Shop]\\Other".to_owned()]
        }
    );
    let sent = fake.requests().len();
    set_now(NOW + 60);
    assert_eq!(client.viewer(&remote()).await.unwrap(), viewer);
    assert_eq!(fake.requests().len(), sent, "remembered for a while");
}

#[tokio::test]
async fn a_rejected_token_a_forbidden_repository_a_rate_limit_and_a_down_server_have_readable_errors() {
    set_now(NOW);
    let answer = Arc::new(Mutex::new((401, "")));
    let served = answer.clone();
    let fake = Fake::start(move |request, _| {
        if request.path.ends_with("/_apis/connectionData") {
            return Canned::json(&profile());
        }
        if request.path.starts_with("/contoso/_apis/identities") {
            return Canned::json(&no_value());
        }
        let (code, body) = *served.lock().unwrap();
        let canned = Canned::status(code, body);
        if code == 429 {
            canned.with("Retry-After", "120")
        } else {
            canned
        }
    });
    let client = client(&fake);

    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert!(matches!(error, PlatformError::Rejected(_)), "{error:?}");

    *answer.lock().unwrap() = (
        403,
        r#"{"message":"TF401019: The Git repository does not exist or you do not have permission."}"#,
    );
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert!(matches!(error, PlatformError::Forbidden(_)), "{error:?}");
    assert!(error.to_string().contains("TF401019"), "{error}");

    *answer.lock().unwrap() = (429, "");
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::RateLimited {
            message: "dev.azure.com/contoso asks to slow down; try again in 2 minutes".to_owned(),
            retry_at: Some(NOW + 120)
        }
    );

    set_now(NOW + 120);
    *answer.lock().unwrap() = (500, "");
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::Unreachable("dev.azure.com/contoso answered 500".to_owned())
    );
}

#[tokio::test]
async fn a_failed_request_is_logged_without_its_token_and_what_is_not_found_is_nothing() {
    set_now(NOW);
    let capture = vet_log::capture::Capture::default();
    let _logging = capture.install(vet_log::filter(None).0);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == PULLS {
            Canned::json(&list_of(&[Doc::new(1)]))
        } else if path.ends_with("/statuses") {
            Canned::status(404, r#"{"message":"TF401180: Not found."}"#)
        } else if path.ends_with("/threads") {
            Canned::json(&no_value())
        } else {
            Canned::status(404, &format!(r#"{{"message":"Gone: {}"}}"#, "x".repeat(400)))
        }
    });
    let found = client(&fake).list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(found.pull_requests.len(), 1, "what is not found is nothing");

    let log = capture.text();
    assert!(log.contains("TF401180"), "the failed request's answer is logged: {log}");
    assert!(!log.contains("secret") && !log.contains("OnNlY3JldA"), "{log}");
}

#[tokio::test]
async fn many_groups_are_named_over_several_requests_so_no_query_gets_too_long() {
    set_now(NOW);
    let descriptors: Vec<String> = (0..52)
        .map(|i| format!("Microsoft.TeamFoundation.Identity;S-1-9-1551374245-2249841213-4234504771-2956991958-2555110769-1-1452809802-541141059-2234181250-{i:09}"))
        .collect();
    let member_of = descriptors.clone();
    let fake = Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            if query_has(request, "identityIds=me-id") {
                return Canned::json(
                    &json!({ "count": 1, "value": [{ "id": "me-id", "memberOf": member_of }] }).to_string(),
                );
            }
            let value = request
                .path
                .split(['?', '&'])
                .find_map(|p| p.strip_prefix("descriptors="))
                .unwrap_or_default();
            // The real service answers a bare 404 to a query this long.
            if value.len() > 4000 {
                return Canned::status(404, "");
            }
            let named = value
                .replace("%2C", ",")
                .split(',')
                .map(|d| d.rsplit("-").next().unwrap().to_owned())
                .collect::<Vec<_>>();
            let groups = named
                .iter()
                .map(|n| json!({ "id": n, "providerDisplayName": format!("group {n}"), "isContainer": true }))
                .collect::<Vec<_>>();
            Canned::json(&json!({ "count": groups.len(), "value": groups }).to_string())
        } else {
            Canned::json(&no_value())
        }
    });
    let viewer = client(&fake).viewer(&remote()).await.unwrap();
    let expected: Vec<String> = (0..52).map(|i| format!("group {i:09}")).collect();
    assert_eq!(viewer.teams, expected);
    assert!(requests_to(&fake, "/identities") > 2, "named by more than one request");
}

#[tokio::test]
async fn groups_the_organization_refuses_are_explained_once_and_not_asked_for_again_for_an_hour() {
    set_now(NOW);
    let capture = vet_log::capture::Capture::default();
    let _logging = capture.install(vet_log::filter(None).0);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::status(404, "")
        } else if path == PULLS {
            let mut asked = Doc::new(1);
            asked.reviewers = vec![reviewer(ada(), 0)];
            Canned::json(&list_of(&[asked]))
        } else {
            Canned::json(&no_value())
        }
    });
    let client = client(&fake);

    let rows = list_rows(&client, &remote(), &PullRequestQuery::new(QueueTab::WaitingForMe))
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 1, "a request to the user itself needs no groups");

    set_now(NOW + VIEWER_MAX_AGE + 1);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(requests_to(&fake, "/identities"), 1);

    set_now(NOW + TEAMS_RETRY_AFTER + 1);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(requests_to(&fake, "/identities"), 2, "asked again after an hour");
    assert_eq!(capture.text().matches("groups of the user cannot be read").count(), 1);
}

#[tokio::test]
async fn nothing_is_sent_without_a_credential_or_for_another_platforms_repository() {
    let fake = repository(vec![], vec![], vec![]);
    let error = client_with(&fake, Signed(None, CredentialSource::Pat))
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap_err();
    assert_eq!(error, PlatformError::SignedOut("dev.azure.com/contoso".to_owned()));

    let github = RemoteKey {
        host: "github.com".to_owned(),
        path: "acme/app".to_owned(),
    };
    let error = client(&fake)
        .list_pull_requests(&github, &all_open())
        .await
        .unwrap_err();
    assert!(matches!(error, PlatformError::Unexpected(_)));
    assert!(fake.requests().is_empty());
}

/// Hands out a token for the accounts it is told of.
struct Accounts(&'static [&'static str]);

impl CredentialProvider for Accounts {
    fn credential(&self, host: &str, _: i64) -> impl Future<Output = Option<Credential>> + Send {
        std::future::ready(self.0.contains(&host).then(|| Credential {
            token: Token::new("secret").unwrap(),
            source: CredentialSource::Pat,
        }))
    }
}

#[tokio::test]
async fn a_remote_is_reached_by_the_account_of_its_organization_and_by_no_other() {
    set_now(NOW);
    let fake = repository(vec![], vec![], vec![]);
    let client = client_with(&fake, Accounts(&["dev.azure.com/contoso"]));
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    let old_address = RemoteKey {
        host: "contoso.visualstudio.com".to_owned(),
        path: "contoso/Shop/api".to_owned(),
    };
    client.list_pull_requests(&old_address, &all_open()).await.unwrap();

    let other = RemoteKey {
        host: "dev.azure.com".to_owned(),
        path: "fabrikam/Shop/api".to_owned(),
    };
    let error = client.list_pull_requests(&other, &all_open()).await.unwrap_err();
    assert_eq!(error, PlatformError::SignedOut("dev.azure.com/fabrikam".to_owned()));
}

#[tokio::test]
async fn a_repository_whose_names_have_spaces_and_umlauts_is_asked_for_at_its_names_escaped_once() {
    set_now(NOW);
    let project = "/contoso/My%20Gesch%C3%A4ft/_apis";
    let pulls = format!("{project}/git/repositories/Back%20End/pullrequests");
    let fake = Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap();
        if path == "/contoso/_apis/connectionData" {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path == pulls {
            Canned::json(&list_of(&[Doc::new(1)]))
        } else if path == format!("{pulls}/1/statuses") || path == format!("{pulls}/1/threads") {
            Canned::json(&no_value())
        } else {
            Canned::status(404, "")
        }
    });
    for url in [
        "https://dev.azure.com/contoso/My%20Gesch%C3%A4ft/_git/Back%20End",
        "https://contoso.visualstudio.com/My Geschäft/_git/Back End",
        "git@ssh.dev.azure.com:v3/contoso/My%20Gesch%C3%A4ft/Back%20End",
    ] {
        let remote = parse_remote(url).unwrap();
        let found = client(&fake).list_pull_requests(&remote, &all_open()).await.unwrap();
        assert_eq!(found.pull_requests.len(), 1, "{url}: {:?}", fake.requests());
        assert!(
            found.pull_requests[0]
                .url
                .ends_with("/My%20Gesch%C3%A4ft/_git/Back%20End/pullrequest/1"),
            "{}",
            found.pull_requests[0].url
        );
    }
}

#[test]
fn a_pull_request_is_shown_at_its_organizations_address() {
    assert_eq!(
        web_address(&remote()),
        "https://dev.azure.com/contoso/Shop/_git/api/pullrequest/"
    );
    let old = parse_remote("https://contoso.visualstudio.com/My%20Shop/_git/Back%20End").unwrap();
    assert_eq!(
        web_address(&old),
        "https://contoso.visualstudio.com/My%20Shop/_git/Back%20End/pullrequest/"
    );
    let umlaut = parse_remote("git@ssh.dev.azure.com:v3/contoso/Gesch%C3%A4ft/Z%C3%A4hler").unwrap();
    assert_eq!(
        web_address(&umlaut),
        "https://dev.azure.com/contoso/Gesch%C3%A4ft/_git/Z%C3%A4hler/pullrequest/"
    );
}
