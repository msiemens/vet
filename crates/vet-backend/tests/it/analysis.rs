//! The background analysis of the changed symbols and problems follows the state of the project:
//! it runs for the revision on view while the review screen is shown, starts again when the revision
//! moves, whatever moved it, and ends with the screen or the project.

#![cfg(debug_assertions)]

use std::time::Duration;

use crate::common::{Client, background_live, config, write};
use crate::pushes::{Rig, fixture, push_again, remote};
use crate::review_screen;
use vet_backend::{Config, Orchestrator};
use vet_protocol::{ChangesetKey, ProjectId, Request, ReviewFilter};

/// What a case does to the project while the review screen shows a pull request.
#[derive(Clone, Copy, Debug)]
enum Act {
    FinishReview,
    SetFilter,
    Narrow,
    Exclude,
    /// The platform records a push to the pull request on view and a fetch takes it in.
    FetchMovesPullRequest,
    OpenChangeset,
    RefreshProblems,
}

/// How often the symbols and the problems analysis were started.
fn starts(client: &Client, project: ProjectId) -> (usize, usize) {
    (
        client.handler.background_counts(project, "symbols").0,
        client.handler.background_counts(project, "problems").0,
    )
}

/// Waits until both analyses have run and none is running, and answers how often they started.
async fn at_rest(client: &Client, project: ProjectId) -> (usize, usize) {
    for _ in 0..6000 {
        let (symbols, problems) = (
            client.handler.background_counts(project, "symbols"),
            client.handler.background_counts(project, "problems"),
        );
        if symbols.0 > 0 && symbols.1 == 0 && problems.0 > 0 && problems.1 == 0 {
            return (symbols.0, problems.0);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the analyses never came to rest");
}

/// Waits until `started` says the analyses were started again, and they are at rest.
async fn restarted(client: &Client, project: ProjectId, started: impl Fn((usize, usize)) -> bool) {
    for _ in 0..6000 {
        if started(starts(client, project)) {
            at_rest(client, project).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the analyses were not started again: {:?}", starts(client, project));
}

/// Waits until no run of `kind` is alive. A run that waits for a language server that never
/// answers only ends when it is stopped: its own request gives up after a minute.
async fn ended(handler: &Orchestrator, project: ProjectId, kind: &'static str) {
    for _ in 0..1000 {
        if handler.background_counts(project, kind).1 == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a {kind} run was not stopped");
}

fn narrowed() -> ReviewFilter {
    ReviewFilter {
        roots: vec!["a.txt".into()],
        ..Default::default()
    }
}

#[tokio::test]
async fn the_analysis_starts_again_for_whatever_moves_the_revision_or_asks_for_it_and_for_nothing_else() {
    let cases = [
        Act::FinishReview,
        Act::SetFilter,
        Act::Narrow,
        Act::Exclude,
        Act::FetchMovesPullRequest,
        Act::OpenChangeset,
        Act::RefreshProblems,
    ];
    for act in cases {
        let (mock, fake, repo, scratch, served) = fixture();
        let rig = Rig::start(&fake, repo, scratch).await;
        let (client, project) = (&rig.client, rig.project);
        rig.open().await;
        rig.toggle_file("a.txt").await;
        client
            .call(Request::SetReviewShown { project, shown: true })
            .await
            .unwrap();
        let before = at_rest(client, project).await;
        client.call(Request::WaitForQuiet { project }).await.unwrap();
        assert_eq!(
            starts(client, project),
            before,
            "{act:?}: the analysis ran again with nothing changed"
        );

        let pull_request = ChangesetKey::PullRequest {
            remote: remote().into(),
            number: 7,
        };
        match act {
            Act::FinishReview => {
                client.call(Request::FinishReview { project }).await.unwrap();
            }
            Act::SetFilter => {
                client
                    .call(Request::SetProjectFilter {
                        project,
                        filter: narrowed(),
                    })
                    .await
                    .unwrap();
            }
            Act::Narrow => {
                client
                    .call(Request::NarrowReview {
                        project,
                        changeset: pull_request,
                        filter: Some(narrowed()),
                    })
                    .await
                    .unwrap();
            }
            Act::Exclude => {
                client
                    .call(Request::ExcludePath {
                        project,
                        path: "a.txt".into(),
                        directory: false,
                    })
                    .await
                    .unwrap();
            }
            Act::FetchMovesPullRequest => {
                push_again(&mock, &served);
                rig.fetch().await;
            }
            Act::OpenChangeset => {
                client
                    .call(Request::OpenChangeset {
                        project,
                        changeset: ChangesetKey::WorkingTree,
                    })
                    .await
                    .unwrap();
            }
            Act::RefreshProblems => {
                client.call(Request::RefreshProblems { project }).await.unwrap();
            }
        }

        // The restart is decided before the reply goes out.
        let now = starts(client, project);
        assert!(
            now.1 > before.1,
            "{act:?}: the problems were not started before the reply"
        );
        if !matches!(act, Act::RefreshProblems) {
            assert!(
                now.0 > before.0,
                "{act:?}: the symbols were not started before the reply"
            );
        }

        match act {
            // Only the problems can be asked for again.
            Act::RefreshProblems => {
                restarted(client, project, |now| now.1 > before.1).await;
                assert_eq!(starts(client, project).0, before.0, "{act:?}: the symbols ran again");
            }
            _ => restarted(client, project, |now| now.0 > before.0 && now.1 > before.1).await,
        }
    }
}

/// An edit of the working tree settles into a new revision without any request to tell the backend,
/// and the analysis follows it.
#[tokio::test]
async fn a_revision_the_settled_working_tree_brings_restarts_the_analysis() {
    let rig = review_screen::Rig::start().await;
    let (client, project) = (&rig.client, rig.project);
    rig.show_review(true).await;
    // A clean tree has nothing to analyse: the first runs end at once.
    let before = at_rest(client, project).await;

    write(rig.repo.path(), "src/lib.rs", "pub fn f() -> u32 {\n    7\n}\n");

    for _ in 0..6000 {
        let now = starts(client, project);
        if now.0 > before.0 && now.1 > before.1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the settled edit did not start the analysis again");
}

#[tokio::test]
async fn hiding_the_review_screen_stops_the_analysis_and_closing_the_project_ends_it_for_a_server_that_never_answers() {
    let rig = review_screen::Rig::start().await;
    rig.branch("a", 2);
    rig.open("a").await;
    rig.show_review(true).await;
    // The server was launched and never answers, so both analyses wait for it.
    rig.launched_beyond(0).await;
    let (client, project) = (&rig.client, rig.project);
    background_live(client, project, "symbols", 1).await;
    background_live(client, project, "problems", 1).await;

    rig.show_review(false).await;
    ended(&client.handler, project, "symbols").await;
    ended(&client.handler, project, "problems").await;

    // The screen that comes back analyses again.
    let before = starts(client, project);
    rig.show_review(true).await;
    background_live(client, project, "symbols", 1).await;
    background_live(client, project, "problems", 1).await;
    assert!(starts(client, project).0 > before.0 && starts(client, project).1 > before.1);

    tokio::time::timeout(Duration::from_secs(30), client.call(Request::CloseProject { project }))
        .await
        .expect("the project closes")
        .unwrap();
    ended(&client.handler, project, "symbols").await;
    ended(&client.handler, project, "problems").await;
}

/// A window that disappears without saying so leaves the review screen "shown" to nobody, and
/// nothing tells the backend; closing the orphaned project is what ends the analysis, though the
/// server it waits for never answers.
#[tokio::test]
async fn a_project_closed_after_its_window_vanished_ends_an_analysis_waiting_for_a_server() {
    let config = Config {
        orphan_grace: Duration::from_millis(200),
        ..config()
    };
    let rig = review_screen::Rig::start_with(config).await;
    rig.branch("a", 2);
    rig.open("a").await;
    rig.show_review(true).await;
    rig.launched_beyond(0).await;
    let project = rig.project;
    background_live(&rig.client, project, "symbols", 1).await;
    background_live(&rig.client, project, "problems", 1).await;

    let Client { handler, events, inbox } = rig.client;
    drop((events, inbox));

    ended(&handler, project, "symbols").await;
    ended(&handler, project, "problems").await;
}
