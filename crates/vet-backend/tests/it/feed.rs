//! The project's pull request feed as the backend wires it: one listing serves every read, the
//! timer, the refresh button and an account change list again, and the work it does is work a
//! client waiting for quiet waits for. Against the local GitHub fake of the prefetch tests.

#![allow(clippy::disallowed_methods)]

use std::time::Duration;

use vet_backend::Config;
use vet_protocol::{ChangesetKey, Event, ProjectId, Request, Response, RowReview, Secret, ServerMessage, View};

use crate::common::Client;
use crate::prefetch::{
    add_ninth, answered, backend, backend_with, config, fixture, list, many_prs, refresh, remote, rows,
};

/// A new token for the GitHub account, which is an account change.
async fn change_account(client: &Client) {
    client
        .call(Request::SetCredentials {
            host: "github.com".into(),
            token: Secret("gho_other".into()),
            validate: false,
        })
        .await
        .unwrap();
}

async fn quiet(client: &Client, project: ProjectId) {
    client.call(Request::WaitForQuiet { project }).await.unwrap();
}

fn list_changes(client: &mut Client) -> usize {
    std::iter::from_fn(|| client.inbox.try_recv().ok())
        .filter(|m| {
            matches!(
                m,
                ServerMessage::Event {
                    event: Event::PullRequestListChanged { .. }
                }
            )
        })
        .count()
}

#[tokio::test]
async fn two_listings_make_one_platform_call() {
    let (mock, fake, repo, _scratch) = fixture();
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    list(&client, project).await;

    assert_eq!(
        mock.lock().unwrap().lists,
        1,
        "the second listing was read from the feed"
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_repeated_refresh_of_an_unchanged_list_does_not_restart_the_prefetch() {
    let (mock, fake, repo, _scratch) = fixture();
    many_prs(&mock, 25);
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        prefetch_pace: Duration::from_millis(15),
        ..config()
    };
    let (_handler, client) = backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    // The run is under way when the next refreshes list the same pull requests again.
    refresh_without_waiting(&client, project).await;
    list(&client, project).await;
    refresh_without_waiting(&client, project).await;
    quiet(&client, project).await;

    assert_eq!(
        crate::common::background_started(&client, project, "prefetch"),
        1,
        "the run was not started over"
    );
    assert_eq!(answered(&mock), 25);
}

/// The refresh button, without waiting for what the refresh leaves running.
async fn refresh_without_waiting(client: &Client, project: ProjectId) {
    client.call(Request::Fetch { project }).await.unwrap();
}

#[tokio::test]
async fn waiting_for_quiet_waits_for_the_details_read_ahead() {
    let (mock, fake, repo, _scratch) = fixture();
    mock.lock().unwrap().delay = Duration::from_millis(150);
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;

    list(&client, project).await;
    quiet(&client, project).await;

    assert_eq!(
        answered(&mock),
        2,
        "both discussions were read before the project was quiet"
    );
}

#[tokio::test]
async fn waiting_for_quiet_waits_for_the_listing_and_head_fetch_an_account_change_asked_for() {
    let (mock, fake, repo, _scratch) = fixture();
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
    list(&client, project).await;
    quiet(&client, project).await;
    add_ninth(&mock);

    change_account(&client).await;
    quiet(&client, project).await;

    assert_eq!(mock.lock().unwrap().lists, 2, "the platform was asked again");
    let refs = crate::common::git(repo.path(), &["for-each-ref", "--format=%(refname)", "refs/vet/pr/"]);
    // The fake remote serves no head for #9, so only its target arrives: the sign of a head fetch.
    assert!(
        refs.lines().any(|r| r.ends_with("/9/target")),
        "the new pull request was fetched: {refs}"
    );
}

#[tokio::test]
async fn an_account_change_lists_again_and_announces_the_new_list() {
    let (mock, fake, repo, _scratch) = fixture();
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
    list(&client, project).await;
    quiet(&client, project).await;
    list_changes(&mut client);
    add_ninth(&mock);

    change_account(&client).await;
    quiet(&client, project).await;

    assert_eq!(list_changes(&mut client), 1);
    let numbers: Vec<u64> = rows(&client, project).await.iter().map(|(n, _)| *n).collect();
    assert!(numbers.contains(&9), "{numbers:?}");
}

#[tokio::test]
async fn a_fetch_answers_after_the_list_was_refreshed() {
    let (mock, fake, repo, _scratch) = fixture();
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
    list(&client, project).await;
    add_ninth(&mock);

    client.call(Request::Fetch { project }).await.unwrap();

    let numbers: Vec<u64> = rows(&client, project).await.iter().map(|(n, _)| *n).collect();
    assert!(
        numbers.contains(&9),
        "the list the answer left behind is the new one: {numbers:?}"
    );
}

async fn open_pull_request(client: &Client, project: ProjectId, number: u32) {
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::PullRequest {
                remote: remote().to_string(),
                number,
            },
        })
        .await
        .unwrap();
}

async fn numbers(client: &Client, project: ProjectId) -> Vec<u64> {
    let mut numbers: Vec<u64> = rows(client, project).await.iter().map(|(n, _)| *n).collect();
    numbers.sort_unstable();
    numbers
}

#[tokio::test]
async fn opening_a_pull_request_the_platform_does_not_list_adds_its_row_without_listing_and_leaving_drops_it() {
    let (mock, fake, repo, _scratch) = fixture();
    mock.lock().unwrap().unlisted = Some(8);
    let (_handler, mut client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
    assert_eq!(numbers(&client, project).await, [7]);
    quiet(&client, project).await;
    list_changes(&mut client);
    let asked = mock.lock().unwrap().lists;

    open_pull_request(&client, project, 8).await;

    assert_eq!(list_changes(&mut client), 1, "the window is told the list changed");
    assert_eq!(numbers(&client, project).await, [7, 8]);
    assert_eq!(mock.lock().unwrap().lists, asked, "opening listed nothing");

    // Switching to a pull request the platform lists leaves the other without a row.
    open_pull_request(&client, project, 7).await;
    assert_eq!(list_changes(&mut client), 1);
    assert_eq!(numbers(&client, project).await, [7]);
    assert_eq!(mock.lock().unwrap().lists, asked, "switching listed nothing");
}

#[tokio::test]
async fn a_refresh_keeps_the_row_of_the_unlisted_pull_request_on_view() {
    let (mock, fake, repo, _scratch) = fixture();
    mock.lock().unwrap().unlisted = Some(8);
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
    list(&client, project).await;
    open_pull_request(&client, project, 8).await;

    refresh(&client, project).await;

    assert_eq!(numbers(&client, project).await, [7, 8]);
}

#[tokio::test]
async fn a_started_review_shows_in_the_next_listing_without_asking_the_platform() {
    let (mock, fake, repo, _scratch) = fixture();
    let (_handler, client) = backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
    list(&client, project).await;
    quiet(&client, project).await;
    let asked = mock.lock().unwrap().lists;

    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::PullRequest {
                remote: remote().to_string(),
                number: 7,
            },
        })
        .await
        .unwrap();
    let revision = match client
        .call(Request::Manifest {
            project,
            view: View::Changeset,
        })
        .await
        .unwrap()
    {
        Response::Manifest(m) => m.revision,
        other => panic!("{other:?}"),
    };
    client
        .call(Request::ToggleFileSeen {
            project,
            view: View::Changeset,
            revision,
            path: "a.txt".into(),
        })
        .await
        .unwrap();

    let listed = rows(&client, project).await;
    let (_, seven) = listed.iter().find(|(n, _)| *n == 7).unwrap();
    assert!(matches!(seven.review, RowReview::Open { .. }), "{:?}", seven.review);
    assert_eq!(mock.lock().unwrap().lists, asked, "the platform was not asked");
}
