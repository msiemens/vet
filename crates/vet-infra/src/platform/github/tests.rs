//! The client against a local server that answers with recorded documents.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use vet_app::platform::{
    ChangedLines, ChangesAsked, ChangesRequest, CiState, Phase, PipelineAction, ReviewDecision, ReviewRequest,
    ReviewVerdict, ViewerReview,
};

use super::*;
use crate::http::client_builder;
use crate::platform::fake::{Canned, Fake, Seen, Signed};

mod discussion;

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
        http: LazyClient::ready(TIMEOUT, http),
        endpoints: Some(Endpoints {
            graphql: fake.base(),
            rest: fake.base(),
        }),
        now,
        state: Mutex::new(State::new()),
        limits: Arc::default(),
    }
}

const PROBE: &str = "/repos/acme/app/pulls?state=all&sort=updated&direction=desc&per_page=1";

/// The pull request that was updated last, as the probe sees it.
fn newest(version: &str) -> String {
    format!(r#"[{{"number":7,"updated_at":"2024-05-01T12:00:00Z/{version}"}}]"#)
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
    let listing = client(&fake).list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(!listing.truncated, "the last page says there is no next one");
    let found = listing.pull_requests;

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
    assert_eq!(first.fork.as_deref(), Some("grace/app"));
    assert_eq!(
        first.lines,
        Some(ChangedLines {
            added: 120,
            removed: 34
        })
    );
    assert_eq!(first.comments, 5);
    assert_eq!(first.labels, ["performance", "needs-review"]);
    assert_eq!(first.assignees, ["ada"]);
    assert_eq!(
        (first.author.as_str(), first.author_name.as_str()),
        ("grace", "Grace Hopper"),
        "the login to compare by, the profile name to show"
    );
    assert_eq!(found[2].author_name, "dependabot", "a bot has no profile name");
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
    assert_eq!(draft.fork.as_deref(), Some("a deleted repository"));
    assert_eq!(draft.comments, 0);
    assert_eq!(found[2].fork, None, "a branch of the repository itself");

    assert_eq!(
        first.changes_requested,
        [
            ChangesRequest {
                reviewer: "Linus Torvalds".to_owned(),
                how: ChangesAsked::Requested
            },
            ChangesRequest {
                reviewer: "ghost".to_owned(),
                how: ChangesAsked::Requested
            }
        ],
        "who requested changes, not who approved"
    );
    assert_eq!(draft.changes_requested, []);
    assert_eq!(
        (first.decision, first.blocked, first.author_avatar.as_deref()),
        (
            ReviewDecision::ChangesRequested,
            true,
            Some("https://avatars.githubusercontent.com/u/2?v=4")
        )
    );
    assert_eq!(
        (draft.decision, draft.blocked, draft.author_avatar.as_deref()),
        (ReviewDecision::Undecided, false, None),
        "nothing said, and no account to have a picture"
    );
}

#[tokio::test]
async fn a_list_that_stops_at_the_most_it_holds_says_it_was_cut_short() {
    set_now(NOW);
    // Every page says there is another.
    let fake = Fake::start(|request, _| {
        if is_probe(request) {
            return Canned::json(&newest("v1"));
        }
        Canned::json(SEARCH_PAGE_1)
    });
    let listing = client(&fake).list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(listing.truncated);
    assert_eq!(listing.pull_requests.len(), MAX_LISTED);
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

/// `canned` with the headers GitHub puts on every response.
fn counted(canned: Canned, remaining: u64, reset: i64) -> Canned {
    canned
        .with("x-ratelimit-remaining", remaining.to_string())
        .with("x-ratelimit-reset", reset.to_string())
}

fn rate_limited_until(reset: i64, now: i64) -> PlatformError {
    rate_limited("github.com", reset, now)
}

const SECONDARY: &str = r#"{"message":"You have exceeded a secondary rate limit. Please wait a few minutes."}"#;

#[tokio::test]
async fn a_rate_limit_is_reported_with_when_it_ends() {
    set_now(NOW);
    let reset = NOW + 600;
    let fake = Fake::start(move |request, _| {
        if request.path == "/graphql" {
            counted(
                Canned::json(
                    r#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded for user ID 1."}]}"#,
                ),
                0,
                reset,
            )
        } else {
            Canned::json(&newest("v1"))
        }
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

/// A host whose REST probe answers fine but says the bucket is used up.
fn used_up_rest(reset: i64) -> (Fake, GitHubClient<Signed>) {
    set_now(NOW);
    let fake = Fake::start(move |request, _| {
        if is_probe(request) {
            counted(Canned::json(&newest("v1")), 0, reset)
        } else {
            Canned::json(SEARCH_PAGE_2)
        }
    });
    let client = client(&fake);
    (fake, client)
}

#[tokio::test]
async fn a_used_up_bucket_is_not_asked_again_until_it_resets() {
    let reset = NOW + 600;
    let (fake, client) = used_up_rest(reset);
    // The answer that used the bucket up is still an answer, and GraphQL has a bucket of its own.
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    let sent = fake.requests().len();

    set_now(NOW + 100);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(reset, NOW + 100)
    );
    assert_eq!(
        client.viewer(&remote()).await.unwrap_err(),
        rate_limited_until(reset, NOW + 100)
    );
    assert_eq!(fake.requests().len(), sent, "nothing is sent to a used-up bucket");
}

#[tokio::test]
async fn requests_are_sent_again_once_the_limit_has_ended() {
    let reset = NOW + 600;
    let (fake, client) = used_up_rest(reset);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    set_now(reset - 1);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    let sent = fake.requests().len();

    set_now(reset);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(fake.requests().len() > sent, "the probe goes out again");
}

#[tokio::test]
async fn a_secondary_limit_is_waited_out_as_retry_after_says() {
    set_now(NOW);
    let fake = Fake::start(|_, _| counted(Canned::status(403, SECONDARY), 4000, NOW + 3000).with("retry-after", "120"));
    let client = client(&fake);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(NOW + 120, NOW),
        "the bucket still has points: the limit lasts as long as retry-after says"
    );
    let sent = fake.requests().len();

    set_now(NOW + 119);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(fake.requests().len(), sent);

    set_now(NOW + 120);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert!(fake.requests().len() > sent, "asked again when retry-after has passed");
}

#[tokio::test]
async fn a_429_is_not_sent_again_at_once() {
    set_now(NOW);
    let fake = Fake::start(|_, _| Canned::status(429, SECONDARY).with("retry-after", "30"));
    let error = client(&fake)
        .list_pull_requests(&remote(), &all_open())
        .await
        .unwrap_err();
    assert_eq!(error, rate_limited_until(NOW + 30, NOW));
    assert_eq!(
        fake.requests().len(),
        1,
        "a server error is tried again, a limit is not"
    );
}

#[tokio::test]
async fn a_refusal_as_rate_limited_without_a_time_waits_a_minute_and_a_refusal_to_read_is_not_one() {
    set_now(NOW);
    let fake = Fake::start(|request, _| match request.path.as_str() {
        PROBE => Canned::status(403, SECONDARY),
        _ => Canned::status(
            403,
            r#"{"message":"Resource protected by organization SAML enforcement."}"#,
        ),
    });
    let client = client(&fake);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(NOW + 60, NOW),
        "GitHub asks for at least a minute when it says nothing else"
    );
    let error = client.pull_request(&remote(), 1).await.unwrap_err();
    assert!(matches!(error, PlatformError::Forbidden(_)), "{error:?}");
    assert!(error.to_string().contains("SAML"), "{error}");
}

#[tokio::test]
async fn a_limit_is_logged_once_for_its_window() {
    set_now(NOW);
    let fake = Fake::start(|_, _| counted(Canned::status(403, SECONDARY), 4000, NOW + 3000).with("retry-after", "120"));
    let client = client(&fake);
    let log = vet_log::capture::Capture::default();
    let _guard = log.install(vet_log::filter(None).0);
    let mentions = || log.text().matches("rate limit is used up").count();

    for _ in 0..5 {
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    }
    assert_eq!(mentions(), 1, "{}", log.text());
    assert!(!log.text().contains("GitHub request failed"), "{}", log.text());

    set_now(NOW + 121);
    for _ in 0..5 {
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    }
    assert_eq!(mentions(), 2, "the next window is told of again: {}", log.text());
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

/// No credential, because the `gh` that lists the host could not deliver its token.
struct UnusableGh;

impl CredentialProvider for UnusableGh {
    fn credential(&self, _: &str, _: i64) -> impl Future<Output = Option<Credential>> + Send {
        std::future::ready(None)
    }

    fn cli_unusable(&self, _: &str) -> Option<String> {
        Some("HTTP 401: Bad credentials".to_owned())
    }
}

#[tokio::test]
async fn a_gh_that_cannot_deliver_its_token_is_not_called_signed_out() {
    let fake = Fake::start(|_, _| Canned::json("{}"));
    let client = client_with(&fake, UnusableGh);
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the gh sign-in for github.com is unusable, as reported by gh: HTTP 401: Bad credentials"
    );
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
    assert_eq!(detail.pull_request.fork.as_deref(), Some("grace/app"));
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

const RUNS: &str = r#"{"total_count":3,"check_runs":[
{"id":11,"name":"test (ubuntu)","status":"completed","conclusion":"failure","started_at":"2024-05-01T12:00:00Z","completed_at":"2024-05-01T12:04:40Z","html_url":"https://github.com/acme/app/actions/runs/5/job/11","app":{"slug":"github-actions"}},
{"id":12,"name":"build","status":"completed","conclusion":"success","started_at":"2024-05-01T12:00:00Z","completed_at":"2024-05-01T12:03:12Z","html_url":"https://github.com/acme/app/actions/runs/5/job/12","app":{"slug":"github-actions"}},
{"id":13,"name":"lint","status":"completed","conclusion":"cancelled","started_at":"2024-05-01T12:00:00Z","completed_at":"2024-05-01T12:00:05Z","html_url":null,"app":{"slug":"github-actions"}}
]}"#;

const STATUSES: &str = r#"{"state":"success","statuses":[
{"id":21,"context":"deploy/preview","state":"success","target_url":"https://preview.example/1","created_at":"2024-05-01T12:00:00Z","updated_at":"2024-05-01T12:02:00Z"}
]}"#;

const WORKFLOW_RUNS: &str = r#"{"total_count":1,"workflow_runs":[
{"id":5,"name":"CI","run_number":42,"run_attempt":2,"event":"pull_request","status":"completed","conclusion":"failure"}
]}"#;

fn checks_server(runs: Arc<Mutex<String>>) -> Fake {
    Fake::start(move |request, _| {
        if request.path.starts_with("/repos/acme/app/commits/abc123/check-runs") {
            Canned::json(&runs.lock().unwrap())
        } else if request.path.starts_with("/repos/acme/app/commits/abc123/status") {
            Canned::json(STATUSES)
        } else if request.path.starts_with("/repos/acme/app/actions/runs?head_sha=abc123") {
            Canned::json(WORKFLOW_RUNS)
        } else {
            Canned::status(404, r#"{"message":"Not Found"}"#)
        }
    })
}

fn requests_to(fake: &Fake, what: &str) -> usize {
    fake.requests().iter().filter(|r| r.path.contains(what)).count()
}

#[tokio::test]
async fn the_checks_of_a_commit_are_its_runs_and_its_statuses() {
    let fake = checks_server(Arc::new(Mutex::new(RUNS.to_owned())));
    let found = client(&fake).checks(&remote(), "abc123").await.unwrap();

    let summary: Vec<_> = found
        .checks
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.state,
                c.finished.zip(c.started).map(|(f, s)| f - s),
                c.log,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("test (ubuntu)", CiState::Failure, Some(280), true),
            ("build", CiState::Success, Some(192), true),
            ("lint", CiState::Neutral, Some(5), true),
            ("deploy/preview", CiState::Success, Some(120), false),
        ]
    );
    assert_eq!(found.state(), Some(CiState::Failure));
    assert_eq!(found.checks[0].id, "11");
    assert_eq!(
        found.checks[0].url.as_deref(),
        Some("https://github.com/acme/app/actions/runs/5/job/11")
    );
    assert_eq!(found.checks[3].url.as_deref(), Some("https://preview.example/1"));
    assert_eq!(
        found.url.as_deref(),
        Some("https://github.com/acme/app/commit/abc123/checks")
    );
    assert_eq!(fake.requests()[0].header("authorization"), Some("Bearer gho_secret"));
}

#[tokio::test]
async fn the_jobs_of_a_workflow_run_are_grouped_under_it_and_a_status_belongs_to_none() {
    let queued = RUNS
        .replace(
            r#""name":"build","status":"completed","conclusion":"success""#,
            r#""name":"build","status":"queued","conclusion":null"#,
        )
        .replace(
            r#""html_url":null,"app":{"slug":"github-actions"}"#,
            r#""html_url":"https://github.com/acme/app/actions/runs/9/job/13","app":{"slug":"other"}"#,
        );
    let fake = checks_server(Arc::new(Mutex::new(queued)));
    let found = client(&fake).checks(&remote(), "abc123").await.unwrap();

    let jobs: Vec<_> = found
        .checks
        .iter()
        .map(|c| (c.name.as_str(), c.run.as_ref().map(|r| r.id.as_str()), c.phase))
        .collect();
    assert_eq!(
        jobs,
        [
            ("test (ubuntu)", Some("5"), Phase::Finished),
            ("build", Some("5"), Phase::Queued),
            ("lint", None, Phase::Cancelled),
            ("deploy/preview", None, Phase::Finished),
        ],
        "only a job of the Actions app is part of a run, and a conclusion of cancelled is its own phase"
    );
    let run = found.checks[0].run.as_ref().unwrap();
    assert_eq!(
        (run.name.as_str(), run.detail.as_str(), run.attempt),
        ("CI", "#42 · pull_request", 2)
    );
}

#[tokio::test]
async fn a_repository_without_workflow_runs_still_has_its_checks() {
    let fake = Fake::start(|request, _| {
        if request.path.starts_with("/repos/acme/app/commits/abc123/check-runs") {
            Canned::json(RUNS)
        } else if request.path.starts_with("/repos/acme/app/commits/abc123/status") {
            Canned::json(STATUSES)
        } else {
            Canned::status(404, r#"{"message":"Not Found"}"#)
        }
    });
    let found = client(&fake).checks(&remote(), "abc123").await.unwrap();
    assert_eq!(found.checks.len(), 4);
    assert!(found.checks.iter().all(|c| c.run.is_none()));
}

const JOB_WITH_STEPS: &str = r#"{"id":11,"steps":[
{"name":"Set up job","status":"completed","conclusion":"success","started_at":"2024-05-01T12:00:00Z","completed_at":"2024-05-01T12:00:02Z"},
{"name":"Test","status":"in_progress","conclusion":null,"started_at":"2024-05-01T12:00:02Z","completed_at":null},
{"name":"Post","status":"pending","conclusion":null,"started_at":null,"completed_at":null}]}"#;

#[tokio::test]
async fn the_steps_of_a_job_say_which_is_running_and_which_waits() {
    let fake = Fake::start(|request, _| {
        if request.path == "/repos/acme/app/actions/jobs/11" {
            Canned::json(JOB_WITH_STEPS)
        } else {
            Canned::status(404, r#"{"message":"Not Found"}"#)
        }
    });
    let steps = client(&fake).steps(&remote(), "11").await.unwrap();

    let summary: Vec<_> = steps.iter().map(|s| (s.name.as_str(), s.state, s.phase)).collect();
    assert_eq!(
        summary,
        [
            ("Set up job", CiState::Success, Phase::Finished),
            ("Test", CiState::Pending, Phase::Running),
            ("Post", CiState::Pending, Phase::Queued),
        ]
    );
    assert_eq!(steps[0].finished.zip(steps[0].started).map(|(f, s)| f - s), Some(2));
}

fn accepting(path: &'static str) -> Fake {
    Fake::start(move |request, _| {
        if request.method == "POST" && request.path == path {
            Canned::status(201, "{}")
        } else if request.path.starts_with("/repos/acme/app/commits/abc123/check-runs") {
            Canned::json(RUNS)
        } else if request.path.starts_with("/repos/acme/app/commits/abc123/status") {
            Canned::json(STATUSES)
        } else {
            Canned::status(404, r#"{"message":"Not Found"}"#)
        }
    })
}

#[tokio::test]
async fn each_action_on_a_run_is_the_call_for_it_and_makes_the_commit_be_read_again() {
    let cases = [
        (
            PipelineAction::RerunFailed { run: "5".to_owned() },
            "/repos/acme/app/actions/runs/5/rerun-failed-jobs",
        ),
        (
            PipelineAction::RerunAll { run: "5".to_owned() },
            "/repos/acme/app/actions/runs/5/rerun",
        ),
        (
            PipelineAction::RerunJob { check: "11".to_owned() },
            "/repos/acme/app/actions/jobs/11/rerun",
        ),
        (
            PipelineAction::Cancel { run: "5".to_owned() },
            "/repos/acme/app/actions/runs/5/cancel",
        ),
    ];
    for (action, path) in cases {
        let fake = accepting(path);
        let client = client(&fake);
        client.checks(&remote(), "abc123").await.unwrap();
        client.pipeline_action(&remote(), &action).await.unwrap();
        client.checks(&remote(), "abc123").await.unwrap();

        assert_eq!(
            fake.requests()
                .iter()
                .filter(|r| r.method == "POST")
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            [path]
        );
        assert_eq!(
            requests_to(&fake, "check-runs"),
            2,
            "checks that were done change with a re-run, so what was kept is dropped: {action:?}"
        );
    }
}

#[tokio::test]
async fn a_token_that_may_not_run_workflows_is_told_so() {
    let fake =
        Fake::start(|_, _| Canned::status(403, r#"{"message":"Resource not accessible by personal access token"}"#));
    let error = client(&fake)
        .pipeline_action(&remote(), &PipelineAction::Cancel { run: "5".to_owned() })
        .await
        .unwrap_err();
    assert!(matches!(error, PlatformError::Forbidden(_)), "{error:?}");
}

#[tokio::test]
async fn something_that_is_not_a_run_is_not_sent() {
    let fake = accepting("/never");
    let error = client(&fake)
        .pipeline_action(&remote(), &PipelineAction::Cancel { run: "../x".to_owned() })
        .await
        .unwrap_err();
    assert!(matches!(error, PlatformError::Unexpected(_)), "{error:?}");
    assert!(fake.requests().is_empty());
}

const FORK_RUNS: &str = r#"{"total_count":2,"check_runs":[
{"id":31,"name":"test","status":"completed","conclusion":"action_required","started_at":null,"completed_at":null,"html_url":"https://github.com/acme/app/actions/runs/6","app":{"slug":"github-actions"}},
{"id":32,"name":"build","status":"action_required","conclusion":null,"started_at":null,"completed_at":null,"html_url":null,"app":{"slug":"github-actions"}}
]}"#;

#[tokio::test]
async fn runs_waiting_for_a_maintainer_to_approve_them_are_not_failures() {
    let runs = Arc::new(Mutex::new(FORK_RUNS.to_owned()));
    let fake = checks_server(runs.clone());
    let client = client(&fake);

    let waiting = client.checks(&remote(), "abc123").await.unwrap();
    let runs_only: Vec<_> = waiting.checks.iter().take(2).map(|c| c.state).collect();
    assert_eq!(runs_only, [CiState::NeedsApproval, CiState::NeedsApproval]);
    assert_eq!(waiting.state(), Some(CiState::NeedsApproval));

    client.checks(&remote(), "abc123").await.unwrap();
    assert_eq!(
        requests_to(&fake, "check-runs"),
        2,
        "approval can come any time, so the commit is asked again"
    );

    *runs.lock().unwrap() = RUNS.to_owned();
    assert_eq!(
        client.checks(&remote(), "abc123").await.unwrap().state(),
        Some(CiState::Failure)
    );
}

#[tokio::test]
async fn checks_that_are_done_are_asked_for_once_and_running_ones_again() {
    let running = RUNS.replace(
        r#""status":"completed","conclusion":"success""#,
        r#""status":"in_progress","conclusion":null"#,
    );
    let runs = Arc::new(Mutex::new(running));
    let fake = checks_server(runs.clone());
    let client = client(&fake);

    let first = client.checks(&remote(), "abc123").await.unwrap();
    assert_eq!(first.state(), Some(CiState::Pending));
    client.checks(&remote(), "abc123").await.unwrap();
    assert_eq!(
        requests_to(&fake, "check-runs"),
        2,
        "a commit with a running check is asked again"
    );

    *runs.lock().unwrap() = RUNS.to_owned();
    let done = client.checks(&remote(), "abc123").await.unwrap();
    assert_eq!(done.state(), Some(CiState::Failure));
    assert_eq!(client.checks(&remote(), "abc123").await.unwrap(), done);
    assert_eq!(
        requests_to(&fake, "check-runs"),
        3,
        "a commit whose checks are done is kept"
    );
}

#[tokio::test]
async fn something_that_is_not_a_commit_id_is_not_sent() {
    let fake = checks_server(Arc::new(Mutex::new(RUNS.to_owned())));
    let error = client(&fake).checks(&remote(), "../orgs").await.unwrap_err();
    assert!(matches!(error, PlatformError::Unexpected(_)), "{error:?}");
    assert!(fake.requests().is_empty());
}

const JOB: &str = r#"{"id":11,"steps":[
{"name":"Set up job","number":1,"conclusion":"success","started_at":"2024-05-01T12:00:00.000Z"},
{"name":"Test","number":2,"conclusion":"failure","started_at":"2024-05-01T12:00:10.000Z"},
{"name":"Post job cleanup","number":3,"conclusion":"success","started_at":"2024-05-01T12:00:14.000Z"}]}"#;

const JOB_LOG: &str = "\
2024-05-01T12:00:00.0000000Z Current runner version
2024-05-01T12:00:10.0000000Z ##[group]Run cargo test
2024-05-01T12:00:12.0000000Z test expired_key_is_rejected ... FAILED
2024-05-01T12:00:13.0000000Z ##[error]Process completed with exit code 101.
2024-05-01T12:00:14.0000000Z Post job cleanup.
";

#[tokio::test]
async fn a_jobs_log_is_fetched_without_the_token_and_says_where_the_job_failed() {
    let fake = Fake::start(|request, base| {
        if request.path == "/repos/acme/app/actions/jobs/11" {
            Canned::json(JOB)
        } else if request.path == "/repos/acme/app/actions/jobs/11/logs" {
            Canned::status(302, "").with("Location", format!("{base}/signed/logs/11"))
        } else if request.path == "/signed/logs/11" {
            Canned::json(JOB_LOG)
        } else {
            Canned::status(404, r#"{"message":"Not Found"}"#)
        }
    });
    let log = client(&fake).check_log(&remote(), "11").await.unwrap();

    assert_eq!(
        log.text.lines().nth(3),
        Some("##[error]Process completed with exit code 101.")
    );
    assert_eq!(
        log.failure,
        Some(vet_app::platform::LogFailure {
            line: 3,
            step: Some("Test".to_owned())
        })
    );
    let blob = fake
        .requests()
        .into_iter()
        .find(|r| r.path == "/signed/logs/11")
        .unwrap();
    assert_eq!(
        blob.header("authorization"),
        None,
        "the signed address is not sent the token"
    );
}

#[tokio::test]
async fn a_log_of_a_job_the_platform_does_not_know_says_so() {
    let fake = Fake::start(|_, _| Canned::status(404, r#"{"message":"Not Found"}"#));
    let error = client(&fake).check_log(&remote(), "11").await.unwrap_err();
    assert!(matches!(error, PlatformError::NotFound(_)), "{error:?}");
}
