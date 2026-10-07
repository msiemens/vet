//! The client against a local server that answers with recorded documents.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use vet_app::platform::{CiState, ReviewRequest, ReviewVerdict, ViewerReview, is_waiting_for_me};

use super::*;
use crate::http::client_builder;
use crate::platform::fake::{Canned, Fake, Seen, Signed};

const SEARCH_PAGE_1: &str = include_str!("fixtures/search_page_1.json");
const SEARCH_PAGE_2: &str = include_str!("fixtures/search_page_2.json");
const DETAIL_PAGE_1: &str = include_str!("fixtures/detail_page_1.json");
const DETAIL_PAGE_2: &str = include_str!("fixtures/detail_page_2.json");

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

trait GraphQlRequest {
    fn variables(&self) -> Value;
}

impl GraphQlRequest for Seen {
    fn variables(&self) -> Value {
        serde_json::from_str::<Value>(&self.body).unwrap()["variables"].clone()
    }
}

trait GraphQlRequests {
    fn graphql_requests(&self) -> Vec<Seen>;
}

impl GraphQlRequests for Fake {
    fn graphql_requests(&self) -> Vec<Seen> {
        self.requests().into_iter().filter(|r| r.path == "/graphql").collect()
    }
}

fn client(fake: &Fake) -> GitHubClient<Signed> {
    client_with(fake, Signed::cli("gho_secret"))
}

fn client_with<C: CredentialProvider + Sync>(fake: &Fake, credentials: C) -> GitHubClient<C> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = client_builder(TIMEOUT).no_proxy().build().unwrap();
    GitHubClient {
        credentials,
        http: OnceLock::from(Ok(http)),
        endpoints: Some(Endpoints {
            graphql: fake.base(),
            rest: fake.base(),
        }),
        now,
        state: Mutex::default(),
    }
}

const PROBE: &str = "/repos/acme/app/pulls?state=all&sort=updated&direction=desc&per_page=1";

/// The pull request that was updated last, as the probe sees it.
fn newest(version: &str) -> String {
    format!(r#"[{{"number":7,"updated_at":"2024-05-01T12:00:00Z/{version}"}}]"#)
}

fn rate_limits(remaining: u64, reset: i64) -> String {
    let bucket = format!(
        r#"{{"limit":5000,"used":{},"remaining":{remaining},"reset":{reset}}}"#,
        5000 - remaining
    );
    format!(r#"{{"resources":{{"core":{bucket},"search":{bucket},"graphql":{bucket}}},"rate":{bucket}}}"#)
}

/// A user as GitHub documents one: `octocrab` needs every URL.
fn user_json(login: &str) -> String {
    let url = |what: &str| format!("https://api.github.com/users/{login}/{what}");
    format!(
        r#"{{"login":"{login}","id":1,"node_id":"U_1","avatar_url":"https://avatars.example/u/1","gravatar_id":"",
"url":"{}","html_url":"https://github.com/{login}","followers_url":"{}","following_url":"{}","gists_url":"{}",
"starred_url":"{}","subscriptions_url":"{}","organizations_url":"{}","repos_url":"{}","events_url":"{}",
"received_events_url":"{}","type":"User","site_admin":false,"name":null,"patch_url":null}}"#,
        url(""),
        url("followers"),
        url("following"),
        url("gists"),
        url("starred"),
        url("subscriptions"),
        url("orgs"),
        url("repos"),
        url("events"),
        url("received_events"),
    )
}

fn team_json(org: &str, slug: &str) -> String {
    let url = |what: &str| format!("https://api.github.com/{what}/{org}/{slug}");
    format!(
        r#"{{"id":5,"node_id":"T_5","url":"{}","html_url":"{}","name":"{slug}","slug":"{slug}","description":null,
"privacy":"closed","permission":"pull","members_url":"{}","repositories_url":"{}",
"organization":{{"login":"{org}","id":9,"node_id":"O_9","url":"{}","repos_url":"{}","events_url":"{}",
"hooks_url":"{}","issues_url":"{}","members_url":"{}","public_members_url":"{}","avatar_url":"https://avatars.example/o/9"}}}}"#,
        url("teams"),
        url("teams"),
        url("members"),
        url("repos"),
        url("orgs"),
        url("repos"),
        url("events"),
        url("hooks"),
        url("issues"),
        url("members"),
        url("public_members"),
    )
}

fn remote() -> RemoteKey {
    RemoteKey {
        host: "github.com".to_owned(),
        path: "acme/app".to_owned(),
    }
}

fn is_probe(request: &Seen) -> bool {
    request.path.starts_with("/repos/acme/app/pulls")
}

/// A server that holds the repository's pull requests as recorded: two search pages and
/// a probe that sees a different newest pull request when `version` changes.
fn repository(version: Arc<Mutex<&'static str>>) -> Fake {
    Fake::start(move |request, _| {
        if is_probe(request) {
            let current = *version.lock().unwrap();
            let tag = format!("\"{current}\"");
            if request.header("if-none-match") == Some(tag.as_str()) {
                return Canned::status(304, "");
            }
            return Canned::json(&newest(current)).with("ETag", tag);
        }
        let after = request.variables()["after"].clone();
        Canned::json(if after.is_null() { SEARCH_PAGE_1 } else { SEARCH_PAGE_2 })
    })
}

fn all_open() -> PullRequestQuery {
    PullRequestQuery::new(QueueTab::AllOpen)
}

#[tokio::test]
async fn a_list_follows_the_pages_and_reads_what_the_queue_shows() {
    set_now(NOW);
    let fake = repository(Arc::new(Mutex::new("v1")));
    let found = client(&fake).list_pull_requests(&remote(), &all_open()).await.unwrap();

    assert_eq!(found.iter().map(|p| p.number).collect::<Vec<_>>(), [101, 102, 103]);
    let graphql = fake.graphql_requests();
    assert_eq!(graphql.len(), 2);
    assert_eq!(graphql[0].variables()["after"], Value::Null);
    assert_eq!(graphql[1].variables()["after"], "cursor-1");
    assert_eq!(graphql[0].header("authorization"), Some("Bearer gho_secret"));

    let first = &found[0];
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
            first.updated_at,
        ),
        (
            "Add a cache to the resolver",
            "grace",
            "resolver-cache",
            "main",
            "c2c2c2",
            false,
            Some(CiState::Failure),
            2,
            1_714_564_800,
        )
    );
    assert_eq!(first.labels, ["performance", "needs-review"]);
    assert_eq!(first.assignees, ["ada"]);
    assert_eq!(
        first.review_requests,
        [
            ReviewRequest::User("ada".to_owned()),
            ReviewRequest::Team("acme/reviewers".to_owned())
        ]
    );
    assert_eq!(
        first.viewer_review,
        Some(ViewerReview {
            verdict: ReviewVerdict::Commented,
            commit: Some("c1c1c1".to_owned())
        })
    );

    let draft = &found[1];
    assert_eq!(draft.author, "ghost", "an account that is gone");
    assert!(draft.draft);
    assert_eq!(draft.ci, None);
    assert_eq!(draft.review_requests, [], "reviewers that are neither users nor teams");
    assert_eq!(draft.viewer_review, None, "a review that was not submitted");
    assert_eq!(found[2].review_requests, [ReviewRequest::User("former-ada".to_owned())]);
    assert_eq!(found[2].ci, Some(CiState::Success));
}

#[tokio::test]
async fn what_is_listed_decides_who_is_waiting() {
    set_now(NOW);
    let fake = repository(Arc::new(Mutex::new("v1")));
    let found = client(&fake).list_pull_requests(&remote(), &all_open()).await.unwrap();
    let viewer = Viewer {
        login: "ada".to_owned(),
        teams: vec!["acme/reviewers".to_owned()],
    };
    // Requested, and reviewed at c1c1c1 while the head is c2c2c2.
    assert!(is_waiting_for_me(&found[0], &viewer));
    assert!(!is_waiting_for_me(&found[1], &viewer));
}

#[tokio::test]
async fn an_unchanged_repository_is_answered_from_the_last_list_for_a_while() {
    set_now(NOW);
    let fake = repository(Arc::new(Mutex::new("v1")));
    let client = client(&fake);
    let first = client.list_pull_requests(&remote(), &all_open()).await.unwrap();

    set_now(NOW + 60);
    let second = client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(second, first);
    assert_eq!(fake.graphql_requests().len(), 2, "no search for the second list");
    let probes: Vec<_> = fake.requests().into_iter().filter(is_probe).collect();
    assert_eq!(probes.len(), 2);
    assert_eq!(probes[0].header("if-none-match"), None);
    assert_eq!(probes[1].header("if-none-match"), Some("\"v1\""));

    // What a probe cannot see, a check run finishing, is picked up after a while.
    set_now(NOW + LIST_MAX_AGE + 1);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(fake.graphql_requests().len(), 4);
}

#[tokio::test]
async fn a_changed_repository_is_searched_again() {
    set_now(NOW);
    let version = Arc::new(Mutex::new("v1"));
    let fake = repository(version.clone());
    let client = client(&fake);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();

    *version.lock().unwrap() = "v2";
    set_now(NOW + 10);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(fake.graphql_requests().len(), 4);
}

#[tokio::test]
async fn another_search_of_an_unchanged_repository_is_not_answered_from_a_different_list() {
    set_now(NOW);
    let fake = repository(Arc::new(Mutex::new("v1")));
    let client = client(&fake);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    client
        .list_pull_requests(&remote(), &PullRequestQuery::new(QueueTab::Mine))
        .await
        .unwrap();
    let searches: Vec<String> = fake
        .graphql_requests()
        .iter()
        .filter(|r| r.variables()["after"].is_null())
        .map(|r| r.variables()["q"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        searches,
        [
            "repo:acme/app is:pr is:open sort:updated-desc",
            "repo:acme/app is:pr is:open sort:updated-desc author:@me"
        ]
    );
}

#[test]
fn the_search_says_what_the_tab_and_the_filters_say() {
    let mut query = PullRequestQuery::new(QueueTab::WaitingForMe);
    query.author = Some("grace".to_owned());
    query.label = Some("needs review".to_owned());
    query.draft = Some(false);
    assert_eq!(
        search_string("acme", "app", &query),
        "repo:acme/app is:pr is:open sort:updated-desc review-requested:@me author:grace label:\"needs review\" draft:false"
    );

    // A second author would widen the search instead of narrowing it; a quote cannot be said.
    let mut mine = PullRequestQuery::new(QueueTab::Mine);
    mine.author = Some("grace".to_owned());
    mine.label = Some("say \"hi\"".to_owned());
    assert_eq!(
        search_string("acme", "app", &mine),
        "repo:acme/app is:pr is:open sort:updated-desc author:@me"
    );
}

#[tokio::test]
async fn a_rate_limit_is_reported_with_when_it_ends() {
    set_now(NOW);
    let reset = NOW + 600;
    let fake = Fake::start(move |request, _| match request.path.as_str() {
        "/rate_limit" => Canned::json(&rate_limits(0, reset)),
        p if p == PROBE => Canned::json(&newest("v1")),
        _ => Canned::json(r#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded for user ID 1."}]}"#),
    });
    let error = client(&fake)
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        PlatformError::RateLimited {
            message: "github.com's rate limit is used up; it resets in 10 minutes".to_owned(),
            retry_at: Some(reset)
        }
    );
}

#[tokio::test]
async fn a_secondary_limit_says_to_wait_and_a_refusal_to_read_is_not_one() {
    set_now(NOW);
    let fake = Fake::start(|request, _| match request.path.as_str() {
        "/rate_limit" => Canned::json(&rate_limits(4000, NOW + 3000)),
        PROBE => Canned::status(
            403,
            r#"{"message":"You have exceeded a secondary rate limit. Please wait a few minutes."}"#,
        ),
        _ => Canned::status(
            403,
            r#"{"message":"Resource protected by organization SAML enforcement."}"#,
        ),
    });
    let client = client(&fake);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        PlatformError::RateLimited {
            message: "github.com's rate limit is used up; try again in a minute".to_owned(),
            retry_at: None
        },
        "the bucket still has points, so the limit has no time to give"
    );
    let error = client.pull_request(&remote(), 1).await.unwrap_err();
    assert!(matches!(error, PlatformError::Forbidden(_)), "{error:?}");
    assert!(error.to_string().contains("SAML"), "{error}");
}

#[tokio::test]
async fn a_rejected_token_a_missing_repository_and_a_down_server_have_readable_errors() {
    set_now(NOW);
    let status = Arc::new(Mutex::new((401, r#"{"message":"Bad credentials"}"#)));
    let answer = status.clone();
    let fake = Fake::start(move |_, _| {
        let (code, body) = *answer.lock().unwrap();
        Canned::status(code, body)
    });
    let client = client(&fake);
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::Rejected("github.com refused the token: Bad credentials (401 Unauthorized)".to_owned())
    );

    *status.lock().unwrap() = (404, r#"{"message":"Not Found"}"#);
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert!(matches!(error, PlatformError::NotFound(_)), "{error:?}");

    *status.lock().unwrap() = (502, r#"{"message":"Server Error"}"#);
    let sent = fake.requests().len();
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::Unreachable("github.com answered 502 Bad Gateway".to_owned())
    );
    assert_eq!(
        fake.requests().len() - sent,
        1 + RETRIES,
        "a server error is tried again"
    );
}

#[tokio::test]
async fn nothing_is_sent_without_a_credential() {
    let fake = Fake::start(|_, _| Canned::json("{}"));
    let client = client_with(&fake, Signed(None, vet_app::platform::CredentialSource::Cli));
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(error, PlatformError::SignedOut("github.com".to_owned()));
    assert_eq!(error.to_string(), "not signed in to github.com");
    assert!(fake.requests().is_empty());
}

#[tokio::test]
async fn a_server_that_cannot_be_reached_is_reported_with_its_host() {
    let fake = Fake::start(|_, _| Canned::json("{}"));
    let client = client(&fake);
    drop(fake);
    let error = client.viewer(&remote()).await.unwrap_err();
    assert!(
        matches!(&error, PlatformError::Unreachable(why) if why.starts_with("cannot reach github.com")),
        "{error:?}"
    );
}

fn viewer_server() -> Fake {
    Fake::start(|request, _| {
        match request.path.as_str() {
        "/user" | "/user/teams?per_page=100" | "/user/teams?per_page=100&page=2"
            if request.header("if-none-match").is_some() =>
        {
            Canned::status(304, "")
        }
        "/user" => Canned::json(&user_json("ada")).with("ETag", "\"user\""),
        "/user/teams?per_page=100" => Canned::json(&format!("[{}]", team_json("acme", "reviewers")))
            .with("ETag", "\"teams-1\"")
            .with(
                "Link",
                r#"<{base}/user/teams?per_page=100&page=2>; rel="next", <{base}/user/teams?per_page=100&page=2>; rel="last""#,
            ),
        "/user/teams?per_page=100&page=2" => {
            Canned::json(&format!("[{}]", team_json("acme", "security"))).with("ETag", "\"teams-2\"")
        }
        other => panic!("unexpected request {other}"),
    }
    })
}

#[tokio::test]
async fn the_viewer_is_their_login_and_every_team_on_every_page() {
    set_now(NOW);
    let fake = viewer_server();
    let client = client(&fake);
    let viewer = client.viewer(&remote()).await.unwrap();
    assert_eq!(
        viewer,
        Viewer {
            login: "ada".to_owned(),
            teams: vec!["acme/reviewers".to_owned(), "acme/security".to_owned()]
        }
    );
    assert_eq!(fake.requests().len(), 3);

    set_now(NOW + 60);
    assert_eq!(client.viewer(&remote()).await.unwrap(), viewer);
    assert_eq!(fake.requests().len(), 3, "remembered for a while");

    // Later it is asked again, as a conditional request for each document, and the
    // pages that were not modified are read from before, the link to the next included.
    set_now(NOW + VIEWER_MAX_AGE + 1);
    assert_eq!(client.viewer(&remote()).await.unwrap(), viewer);
    let again = &fake.requests()[3..];
    assert_eq!(again.len(), 3);
    assert!(again.iter().all(|r| r.header("if-none-match").is_some()), "{again:?}");
}

#[tokio::test]
async fn a_token_that_may_not_list_teams_still_has_a_viewer() {
    let fake = Fake::start(|request, _| {
        if request.path == "/user" {
            Canned::json(&user_json("ada"))
        } else {
            Canned::status(403, r#"{"message":"Resource not accessible by personal access token"}"#)
        }
    });
    let viewer = client(&fake).viewer(&remote()).await.unwrap();
    assert_eq!((viewer.login.as_str(), viewer.teams.len()), ("ada", 0));
}

#[tokio::test]
async fn a_link_to_another_host_is_followed_without_the_token() {
    let fake = Fake::start(|request, base| match request.path.as_str() {
        "/user" => Canned::json(&user_json("ada")),
        "/user/teams?per_page=100" => {
            // The same server under another name is another host.
            let elsewhere = base.replace("127.0.0.1", "localhost");
            Canned::json(&format!("[{}]", team_json("acme", "reviewers")))
                .with("Link", format!(r#"<{elsewhere}/user/teams?page=2>; rel="next""#))
        }
        _ => Canned::json(&format!("[{}]", team_json("acme", "security"))),
    });
    let viewer = client(&fake).viewer(&remote()).await.unwrap();
    assert_eq!(viewer.teams, ["acme/reviewers", "acme/security"]);
    let requests = fake.requests();
    assert!(requests[..2].iter().all(|r| r.header("authorization").is_some()));
    assert_eq!(requests[2].header("authorization"), None);
}

#[tokio::test]
async fn a_redirect_is_not_followed_with_the_token() {
    let fake = Fake::start(|_, _| Canned::status(301, "").with("Location", "https://elsewhere.example/"));
    let error = client(&fake).viewer(&remote()).await.unwrap_err();
    assert!(matches!(error, PlatformError::Unexpected(_)), "{error:?}");
}

#[tokio::test]
async fn a_pull_request_comes_with_its_pushes_from_every_page_of_force_pushes() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        if request.variables()["after"].is_null() {
            Canned::json(DETAIL_PAGE_1)
        } else {
            Canned::json(DETAIL_PAGE_2)
        }
    });
    let detail = client(&fake).pull_request(&remote(), 101).await.unwrap();

    let graphql = fake.graphql_requests();
    assert_eq!(graphql.len(), 2);
    assert_eq!(graphql[0].variables()["number"], 101);
    assert_eq!(graphql[0].variables()["owner"], "acme");
    assert_eq!(graphql[1].variables()["after"], "timeline-1");

    assert_eq!(detail.base, "b0b0b0");
    assert!(detail.from_fork);
    assert_eq!(detail.pull_request.head, "c4c4c4");
    assert_eq!(detail.pull_request.ci, Some(CiState::Pending));
    let pushes: Vec<_> = detail.pushes.iter().map(|p| (p.head.as_str(), p.forced)).collect();
    assert_eq!(
        pushes,
        [
            ("a1a1a1", false),
            ("a2a2a2", true),
            ("a3a3a3", false),
            ("a4a4a4", true),
            ("c4c4c4", false)
        ]
    );
    assert_eq!(detail.pushes[1].at, Some(1_713_607_200));
}

#[tokio::test]
async fn a_pull_request_that_is_not_there_is_not_found() {
    let fake = Fake::start(|_, _| {
        Canned::json(
            r#"{"data":{"repository":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND","message":"Could not resolve to a PullRequest"}]}"#,
        )
    });
    let error = client(&fake).pull_request(&remote(), 999).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::NotFound("acme/app#999 was not found on github.com, or the token cannot see it".to_owned())
    );
}

#[tokio::test]
async fn only_github_repositories_are_asked() {
    let fake = Fake::start(|_, _| Canned::json("{}"));
    let ado = RemoteKey {
        host: "dev.azure.com".to_owned(),
        path: "contoso/Shop/api".to_owned(),
    };
    let error = client(&fake).list_pull_requests(&ado, &all_open()).await.unwrap_err();
    assert!(matches!(error, PlatformError::Unexpected(_)));
    assert!(fake.requests().is_empty());
}

#[test]
fn enterprise_hosts_have_their_api_under_their_own_name() {
    assert_eq!(Endpoints::of("github.com").rest, "https://api.github.com");
    assert_eq!(
        Endpoints::of("ghe.corp.example"),
        Endpoints {
            graphql: "https://ghe.corp.example/api".to_owned(),
            rest: "https://ghe.corp.example/api/v3".to_owned()
        }
    );
}

/// Against the real GitHub with the `gh` CLI's token:
/// `cargo test -p vet-infra github_smoke -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "talks to github.com"]
async fn github_smoke() {
    let token = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .unwrap();
    let token: &'static str = Box::leak(
        String::from_utf8(token.stdout)
            .unwrap()
            .trim()
            .to_owned()
            .into_boxed_str(),
    );
    let client = GitHubClient::new(Signed::cli(token));
    let remote = RemoteKey {
        host: "github.com".to_owned(),
        path: std::env::var("VET_SMOKE_REPO").unwrap_or_else(|_| "cli/cli".to_owned()),
    };
    let viewer = client.viewer(&remote).await.unwrap();
    println!("viewer {viewer:?}");
    for tab in [
        QueueTab::AllOpen,
        QueueTab::WaitingForMe,
        QueueTab::Mine,
        QueueTab::Assigned,
    ] {
        let started = std::time::Instant::now();
        let found = client
            .list_pull_requests(&remote, &PullRequestQuery::new(tab))
            .await
            .unwrap();
        println!("{tab:?}: {} pull requests in {:?}", found.len(), started.elapsed());
        if let Some(first) = found.first() {
            println!("  {first:?}");
        }
    }
    let again = std::time::Instant::now();
    client.list_pull_requests(&remote, &all_open()).await.unwrap();
    println!("again: {:?}", again.elapsed());
    let found = client.list_pull_requests(&remote, &all_open()).await.unwrap();
    let detail = client.pull_request(&remote, found[0].number).await.unwrap();
    println!("{detail:?}");
}
