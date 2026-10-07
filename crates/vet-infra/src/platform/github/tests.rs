//! The client against a local server that answers with recorded documents.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use vet_app::platform::{
    CiState, Credential, CredentialSource, ReviewRequest, ReviewVerdict, Token, ViewerReview, is_waiting_for_me,
};

use super::*;
use crate::http::client_builder;

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

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Seen {
    fn variables(&self) -> Value {
        serde_json::from_str::<Value>(&self.body).unwrap()["variables"].clone()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

struct Canned {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

impl Canned {
    fn json(body: &str) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.to_owned(),
        }
    }

    fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            ..Self::json(body)
        }
    }

    fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// A server on a local port that answers each request as the handler says, one request
/// per connection.
struct Fake {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    /// `handler` gets the request and the server's base URL.
    fn start(handler: impl Fn(&Seen, &str) -> Canned + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (seen, stop) = (seen.clone(), stop.clone());
            std::thread::spawn(move || {
                let base = format!("http://{addr}");
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let Some(request) = read_request(&mut stream) else {
                        continue;
                    };
                    let canned = handler(&request, &base);
                    seen.lock().unwrap().push(request);
                    write_response(&mut stream, &canned, &base);
                }
            })
        };
        Self {
            addr,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn graphql_requests(&self) -> Vec<Seen> {
        self.requests().into_iter().filter(|r| r.path == "/graphql").collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let path = parts.nth(1)?.to_owned();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Seen {
        path,
        headers,
        body: String::from_utf8(body).ok()?,
    })
}

fn write_response(stream: &mut TcpStream, canned: &Canned, base: &str) {
    let body = canned.body.replace("{base}", base);
    let mut head = format!(
        "HTTP/1.1 {} X\r\nConnection: close\r\nContent-Length: {}\r\n",
        canned.status,
        body.len()
    );
    for (name, value) in &canned.headers {
        head.push_str(&format!("{name}: {}\r\n", value.replace("{base}", base)));
    }
    let _ = stream.write_all(format!("{head}\r\n{body}").as_bytes());
}

struct Signed(Option<&'static str>);

impl CredentialProvider for Signed {
    fn credential(&self, _: &str, _: i64) -> impl Future<Output = Option<Credential>> + Send {
        std::future::ready(self.0.map(|token| Credential {
            token: Token::new(token).unwrap(),
            source: CredentialSource::Cli,
        }))
    }
}

fn client(fake: &Fake) -> GitHubClient<Signed> {
    client_with(fake, Signed(Some("gho_secret")))
}

fn client_with<C: CredentialProvider + Sync>(fake: &Fake, credentials: C) -> GitHubClient<C> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = client_builder(TIMEOUT).no_proxy().build().unwrap();
    GitHubClient {
        credentials,
        client: OnceLock::from(Ok(http)),
        endpoints: Some(Endpoints {
            graphql: format!("{}/graphql", fake.base()),
            rest: fake.base(),
        }),
        now,
        state: Mutex::default(),
    }
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
/// a probe whose tag changes when `etag` does.
fn repository(etag: Arc<Mutex<&'static str>>) -> Fake {
    Fake::start(move |request, _| {
        if is_probe(request) {
            let current = *etag.lock().unwrap();
            if request.header("if-none-match") == Some(current) {
                return Canned::status(304, "");
            }
            return Canned::json("[]").with("ETag", current);
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
    let fake = repository(Arc::new(Mutex::new("\"v1\"")));
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
    let fake = repository(Arc::new(Mutex::new("\"v1\"")));
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
    let fake = repository(Arc::new(Mutex::new("\"v1\"")));
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
    let etag = Arc::new(Mutex::new("\"v1\""));
    let fake = repository(etag.clone());
    let client = client(&fake);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();

    *etag.lock().unwrap() = "\"v2\"";
    set_now(NOW + 10);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert_eq!(fake.graphql_requests().len(), 4);
}

#[tokio::test]
async fn another_search_of_an_unchanged_repository_is_not_answered_from_a_different_list() {
    set_now(NOW);
    let fake = repository(Arc::new(Mutex::new("\"v1\"")));
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
async fn a_rate_limit_is_reported_with_when_it_ends_and_then_not_tried_against() {
    set_now(NOW);
    let reset = NOW + 600;
    let fake = Fake::start(move |request, _| {
        if is_probe(request) {
            return Canned::json("[]").with("ETag", "\"v1\"");
        }
        Canned::json(r#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded for user ID 1."}]}"#)
            .with("x-ratelimit-remaining", "0")
            .with("x-ratelimit-reset", reset.to_string())
    });
    let client = client(&fake);

    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::RateLimited {
            message: "github.com's rate limit is used up; it resets in 10 minutes".to_owned(),
            retry_at: Some(reset)
        }
    );
    let sent = fake.requests().len();

    set_now(NOW + 120);
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert!(error.to_string().contains("resets in 8 minutes"), "{error}");
    assert_eq!(
        fake.graphql_requests().len(),
        1,
        "nothing is sent while the limit is used up"
    );
    assert!(fake.requests().len() <= sent + 1, "at most the free probe");

    set_now(reset + 1);
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert!(matches!(error, PlatformError::RateLimited { .. }));
    assert_eq!(fake.graphql_requests().len(), 2, "asked again once the limit has reset");
}

#[tokio::test]
async fn a_secondary_limit_says_to_wait_and_a_refusal_to_read_is_not_one() {
    set_now(NOW);
    let fake = Fake::start(|request, _| match request.path.as_str() {
        "/repos/acme/app/pulls?state=all&sort=updated&direction=desc&per_page=1" => Canned::status(
            403,
            r#"{"message":"You have exceeded a secondary rate limit. Please wait a few minutes."}"#,
        )
        .with("retry-after", "90"),
        _ => Canned::status(
            403,
            r#"{"message":"Resource protected by organization SAML enforcement."}"#,
        ),
    });
    let client = client(&fake);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        PlatformError::RateLimited {
            message: "github.com's rate limit is used up; it resets in 2 minutes".to_owned(),
            retry_at: Some(NOW + 90)
        }
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

    *status.lock().unwrap() = (502, "<html>Bad gateway</html>");
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(
        error,
        PlatformError::Unreachable("github.com answered 502 Bad Gateway".to_owned())
    );
}

#[tokio::test]
async fn nothing_is_sent_without_a_credential() {
    let fake = Fake::start(|_, _| Canned::json("{}"));
    let client = client_with(&fake, Signed(None));
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
    let error = client.viewer("github.com").await.unwrap_err();
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
        "/user" => Canned::json(r#"{"login":"ada","id":1}"#).with("ETag", "\"user\""),
        "/user/teams?per_page=100" => Canned::json(r#"[{"slug":"reviewers","organization":{"login":"acme"}}]"#)
            .with("ETag", "\"teams-1\"")
            .with("Link", r#"<{base}/user/teams?per_page=100&page=2>; rel="next", <{base}/user/teams?per_page=100&page=2>; rel="last""#),
        "/user/teams?per_page=100&page=2" => {
            Canned::json(r#"[{"slug":"security","organization":{"login":"acme"}}]"#).with("ETag", "\"teams-2\"")
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
    let viewer = client.viewer("github.com").await.unwrap();
    assert_eq!(
        viewer,
        Viewer {
            login: "ada".to_owned(),
            teams: vec!["acme/reviewers".to_owned(), "acme/security".to_owned()]
        }
    );
    assert_eq!(fake.requests().len(), 3);

    set_now(NOW + 60);
    assert_eq!(client.viewer("github.com").await.unwrap(), viewer);
    assert_eq!(fake.requests().len(), 3, "remembered for a while");

    // Later it is asked again, as a conditional request for each document, and the
    // pages that were not modified are read from before, the link to the next included.
    set_now(NOW + VIEWER_MAX_AGE + 1);
    assert_eq!(client.viewer("github.com").await.unwrap(), viewer);
    let again = &fake.requests()[3..];
    assert_eq!(again.len(), 3);
    assert!(again.iter().all(|r| r.header("if-none-match").is_some()), "{again:?}");
}

#[tokio::test]
async fn a_token_that_may_not_list_teams_still_has_a_viewer() {
    let fake = Fake::start(|request, _| {
        if request.path == "/user" {
            Canned::json(r#"{"login":"ada"}"#)
        } else {
            Canned::status(403, r#"{"message":"Resource not accessible by personal access token"}"#)
        }
    });
    let viewer = client(&fake).viewer("github.com").await.unwrap();
    assert_eq!((viewer.login.as_str(), viewer.teams.len()), ("ada", 0));
}

#[tokio::test]
async fn a_link_to_another_host_is_not_followed() {
    let fake = Fake::start(|request, _| {
        if request.path == "/user" {
            Canned::json(r#"{"login":"ada"}"#)
        } else {
            Canned::json("[]").with("Link", r#"<https://evil.example/user/teams>; rel="next""#)
        }
    });
    let error = client(&fake).viewer("github.com").await.unwrap_err();
    assert!(
        matches!(&error, PlatformError::Unexpected(why) if why.contains("evil.example")),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_redirect_is_not_followed_with_the_token() {
    let fake = Fake::start(|_, _| Canned::status(301, "").with("Location", "https://elsewhere.example/"));
    let error = client(&fake).viewer("github.com").await.unwrap_err();
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
    assert_eq!(Endpoints::of("github.com").graphql, "https://api.github.com/graphql");
    assert_eq!(
        Endpoints::of("ghe.corp.example"),
        Endpoints {
            graphql: "https://ghe.corp.example/api/graphql".to_owned(),
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
    let client = GitHubClient::new(Signed(Some(token)));
    let remote = RemoteKey {
        host: "github.com".to_owned(),
        path: std::env::var("VET_SMOKE_REPO").unwrap_or_else(|_| "cli/cli".to_owned()),
    };
    let viewer = client.viewer(&remote.host).await.unwrap();
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
