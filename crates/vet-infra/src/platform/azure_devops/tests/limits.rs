//! The rate limits Azure DevOps announces in its responses hold requests back until they end.

use super::*;

fn rate_limited_until(until: i64, now: i64) -> PlatformError {
    PlatformError::RateLimited {
        message: format!(
            "dev.azure.com/contoso asks to slow down; try again in {}",
            match (until - now + 59) / 60 {
                1 => "a minute".to_owned(),
                minutes => format!("{minutes} minutes"),
            }
        ),
        retry_at: Some(until),
    }
}

/// An organization whose pull request list answers with `answer`, and the rest as usual.
fn limited(answer: impl Fn() -> Canned + Send + 'static) -> (Fake, AzureDevOpsClient<Signed>) {
    set_now(NOW);
    let fake = Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path.starts_with("/contoso/_apis/identities") {
            Canned::json(&no_value())
        } else {
            answer()
        }
    });
    let client = client(&fake);
    (fake, client)
}

/// The signals Azure DevOps puts on a response before it delays requests.
fn counted(canned: Canned, remaining: u32, reset: i64) -> Canned {
    canned
        .with("X-RateLimit-Resource", "Git (Shop)")
        .with("X-RateLimit-Limit", "200")
        .with("X-RateLimit-Remaining", remaining.to_string())
        .with("X-RateLimit-Reset", reset.to_string())
}

#[tokio::test]
async fn a_429_with_nothing_left_is_not_asked_again_until_the_reset() {
    let reset = NOW + 600;
    let (fake, client) = limited(move || counted(Canned::status(429, ""), 0, reset));
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(reset, NOW),
        "the answer that hit the limit tells the time the gate holds until"
    );
    let sent = fake.requests().len();

    set_now(NOW + 100);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(reset, NOW + 100)
    );
    assert_eq!(fake.requests().len(), sent, "nothing is sent to a limited organization");
}

#[tokio::test]
async fn requests_are_sent_again_once_the_limit_has_ended() {
    let reset = NOW + 600;
    let (fake, client) = limited(move || counted(Canned::status(429, ""), 0, reset));
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    set_now(reset - 1);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    let sent = fake.requests().len();

    set_now(reset);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert!(fake.requests().len() > sent, "the list is asked for again");
}

#[tokio::test]
async fn delayed_requests_that_succeed_are_not_held_back() {
    let (fake, client) = limited(|| counted(Canned::json(&no_value()), 0, NOW + 600));
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    let sent = fake.requests().len();
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(fake.requests().len() > sent, "a count of zero on a 200 is no block");
}

#[tokio::test]
async fn a_remaining_count_above_zero_holds_nothing_back() {
    let (fake, client) = limited(|| counted(Canned::json(&no_value()), 3, NOW + 600));
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    let sent = fake.requests().len();
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(fake.requests().len() > sent);
}

#[tokio::test]
async fn retry_after_is_waited_out_even_on_a_successful_answer() {
    let (fake, client) = limited(|| counted(Canned::json(&no_value()), 150, NOW + 3000).with("Retry-After", "120"));
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    let sent = fake.requests().len();

    set_now(NOW + 119);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(NOW + 120, NOW + 119),
        "the limit lasts as long as Retry-After says, not until the reset"
    );
    assert_eq!(fake.requests().len(), sent);

    set_now(NOW + 120);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap();
    assert!(fake.requests().len() > sent, "asked again when Retry-After has passed");
}

#[tokio::test]
async fn a_429_is_not_sent_again_at_once() {
    let (fake, client) = limited(|| Canned::status(429, "").with("Retry-After", "30"));
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(NOW + 30, NOW)
    );
    assert_eq!(requests_to(&fake, "/pullrequests"), 1);

    set_now(NOW + 10);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(NOW + 30, NOW + 10)
    );
    assert_eq!(
        requests_to(&fake, "/pullrequests"),
        1,
        "not asked while the limit holds"
    );
}

#[tokio::test]
async fn a_429_without_a_time_waits_a_minute() {
    let (fake, client) = limited(|| Canned::status(429, ""));
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(NOW + 60, NOW)
    );
    set_now(NOW + 59);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(requests_to(&fake, "/pullrequests"), 1);
}

#[tokio::test]
async fn a_server_error_that_says_retry_after_is_not_tried_again_at_once() {
    let (fake, client) = limited(|| Canned::status(503, "").with("Retry-After", "30"));
    let error = client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(error, rate_limited_until(NOW + 30, NOW));
    assert_eq!(
        requests_to(&fake, "/pullrequests"),
        1,
        "a server error is tried again, a limit is not"
    );
}

#[tokio::test]
async fn a_limit_the_profile_request_hears_of_holds_back_the_rest() {
    set_now(NOW);
    let fake = Fake::start(|_, _| Canned::status(429, "").with("Retry-After", "90"));
    let client = client(&fake);
    client.viewer(&remote()).await.unwrap_err();
    assert_eq!(fake.requests().len(), 1);
    assert_eq!(
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err(),
        rate_limited_until(NOW + 90, NOW)
    );
    assert_eq!(fake.requests().len(), 1);
}

#[tokio::test]
async fn a_limit_is_logged_once_for_its_window() {
    // The statuses and the threads of a pull request are asked for together: both are on their way
    // when the first answer says the limit is used up.
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap();
        if path.ends_with("/_apis/connectionData") {
            Canned::json(&profile())
        } else if path.starts_with("/contoso/_apis/identities") {
            Canned::json(&no_value())
        } else if path == PULLS {
            Canned::json(&list_of(&[Doc::new(1)]))
        } else {
            Canned::status(429, "").with("Retry-After", "120")
        }
    });
    let client = client(&fake);
    let log = vet_log::capture::Capture::default();
    let _guard = log.install(vet_log::filter(None).0);
    let mentions = || log.text().matches("rate limit is used up").count();

    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(requests_to(&fake, "/statuses"), 1);
    assert_eq!(requests_to(&fake, "/threads"), 1);
    assert_eq!(mentions(), 1, "{}", log.text());

    for _ in 0..3 {
        client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    }
    assert_eq!(mentions(), 1, "{}", log.text());

    set_now(NOW + 121);
    client.list_pull_requests(&remote(), &all_open()).await.unwrap_err();
    assert_eq!(mentions(), 2, "the next window is told of again: {}", log.text());
}
