//! The background analysis of the changed symbols and problems follows the state of the project:
//! it runs for the revision on view while the review screen is shown, starts again when the revision
//! moves, whatever moved it, and ends with the screen or the project.

#![cfg(debug_assertions)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::common::{Client, background_live, background_started, config, write};
use crate::pushes::{Mock, Rig, Served, fixture, push_again, remote};
use crate::review_screen;
use vet_backend::{Config, Injected, Orchestrator};
use vet_infra::platform::fake::Fake;
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

/// Does `act` to the project of `rig`, whose pull request 7 is on view.
async fn perform(act: Act, rig: &Rig, mock: &Arc<Mutex<Mock>>, served: &Served) {
    let (client, project) = (&rig.client, rig.project);
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
                    changeset: ChangesetKey::PullRequest {
                        remote: remote().into(),
                        number: 7,
                    },
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
            push_again(mock, served);
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

        perform(act, &rig, &mock, &served).await;

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

/// Waits for quiet, then says how often each analysis started and how many of them are alive.
async fn quiet_analysis(client: &Client, project: ProjectId) -> ((usize, usize), (usize, usize)) {
    client.call(Request::WaitForQuiet { project }).await.unwrap();
    (
        client.handler.background_counts(project, "symbols"),
        client.handler.background_counts(project, "problems"),
    )
}

#[tokio::test]
async fn quiet_after_finish_review_is_answered_only_once_the_new_revisions_analysis_ended() {
    let (_mock, fake, repo, scratch, _served) = fixture();
    // Every try of a.txt fails for a reason that can pass, so each run of the symbols analysis
    // stays under way for the pauses between its tries: one run takes three failures.
    let failures = vec![Injected::Store; 6];
    let rig = Rig::start_failing(&fake, repo, scratch, "a.txt", failures).await;
    let (client, project) = (&rig.client, rig.project);
    rig.open().await;
    rig.toggle_file("a.txt").await;
    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();
    let before = at_rest(client, project).await;

    client.call(Request::FinishReview { project }).await.unwrap();
    assert_eq!(
        client.handler.background_counts(project, "symbols").1,
        1,
        "the new revision's analysis is under way when the reply goes out"
    );
    let (symbols, problems) = quiet_analysis(client, project).await;

    assert!(
        symbols.0 > before.0 && problems.0 > before.1,
        "the analysis of the new revision did not start: {symbols:?} {problems:?}"
    );
    assert_eq!((symbols.1, problems.1), (0, 0), "quiet was answered while it ran");
}

#[tokio::test]
async fn quiet_right_after_the_review_screen_comes_up_waits_for_the_analysis_it_started() {
    let (_mock, fake, repo, scratch, _served) = fixture();
    let rig = Rig::start(&fake, repo, scratch).await;
    let (client, project) = (&rig.client, rig.project);
    rig.open().await;
    rig.toggle_file("a.txt").await;
    client.call(Request::WaitForQuiet { project }).await.unwrap();
    assert_eq!(starts(client, project), (0, 0), "no analysis off the review screen");

    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();
    let (symbols, problems) = quiet_analysis(client, project).await;

    assert!(symbols.0 > 0 && problems.0 > 0, "the analysis never started");
    assert_eq!((symbols.1, problems.1), (0, 0), "quiet was answered while it ran");
}

/// An edit of the working tree settles into a new revision without any request to tell the backend,
/// and the analysis and the work that follows the snapshots follow it.
#[tokio::test]
async fn a_revision_the_settled_working_tree_brings_restarts_the_analysis() {
    let rig = review_screen::Rig::start().await;
    let (client, project) = (&rig.client, rig.project);
    rig.show_review(true).await;
    // A clean tree has nothing to analyse: the first runs end at once.
    let before = at_rest(client, project).await;
    let work = snapshot_work(client, project);

    write(rig.repo.path(), "src/lib.rs", "pub fn f() -> u32 {\n    7\n}\n");

    for _ in 0..6000 {
        let now = starts(client, project);
        if now.0 > before.0 && now.1 > before.1 {
            let now = snapshot_work(client, project);
            assert!(
                now[0] > work[0] && now[1] > work[1],
                "the settled snapshot did not retire and start the servers again: {work:?} {now:?}"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the settled edit did not start the analysis again");
}

/// How often the retiring of servers, the start of servers and the checks started, which follow the
/// snapshots.
fn snapshot_work(client: &Client, project: ProjectId) -> [usize; 3] {
    ["retire", "servers", "checks"].map(|kind| background_started(client, project, kind))
}

/// A pull request on view on the review screen, its analysis at rest.
async fn shown_pull_request() -> (Rig, Fake, Arc<Mutex<Mock>>, Served) {
    let (mock, fake, repo, scratch, served) = fixture();
    let rig = Rig::start(&fake, repo, scratch).await;
    rig.open().await;
    rig.toggle_file("a.txt").await;
    rig.client
        .call(Request::SetReviewShown {
            project: rig.project,
            shown: true,
        })
        .await
        .unwrap();
    at_rest(&rig.client, rig.project).await;
    rig.client
        .call(Request::WaitForQuiet { project: rig.project })
        .await
        .unwrap();
    (rig, fake, mock, served)
}

/// A new filter changes the files the analysis covers and no snapshot, so only the analysis starts
/// again: the servers, their retiring and the checks do not depend on the filter.
#[tokio::test]
async fn a_new_filter_restarts_the_analysis_but_not_the_work_that_follows_the_snapshots() {
    for act in [Act::SetFilter, Act::Narrow, Act::Exclude] {
        let (rig, _fake, mock, served) = shown_pull_request().await;
        let (client, project) = (&rig.client, rig.project);
        let analysis = starts(client, project);
        let work = snapshot_work(client, project);
        assert!(
            work.iter().all(|started| *started > 0),
            "{act:?}: the work was not started before: {work:?}"
        );

        perform(act, &rig, &mock, &served).await;

        restarted(client, project, |now| now.0 > analysis.0 && now.1 > analysis.1).await;
        client.call(Request::WaitForQuiet { project }).await.unwrap();
        assert_eq!(
            snapshot_work(client, project),
            work,
            "{act:?}: work that does not depend on the filter started again"
        );
    }
}

#[tokio::test]
async fn a_snapshot_that_moves_restarts_the_analysis_and_the_work_that_follows_the_snapshots() {
    let (rig, _fake, mock, served) = shown_pull_request().await;
    let (client, project) = (&rig.client, rig.project);
    let analysis = starts(client, project);
    let work = snapshot_work(client, project);

    perform(Act::FetchMovesPullRequest, &rig, &mock, &served).await;

    restarted(client, project, |now| now.0 > analysis.0 && now.1 > analysis.1).await;
    let now = snapshot_work(client, project);
    assert!(
        now.iter().zip(work).all(|(now, before)| *now > before),
        "not all of retire, servers and checks started again: {work:?} {now:?}"
    );
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

#[tokio::test]
async fn closing_the_project_ends_its_settle_loop() {
    let rig = review_screen::Rig::start().await;
    let (client, project) = (&rig.client, rig.project);
    background_live(client, project, "settling", 1).await;
    assert_eq!(client.handler.background_counts(project, "settling").0, 1);

    tokio::time::timeout(Duration::from_secs(30), client.call(Request::CloseProject { project }))
        .await
        .expect("the project closes")
        .unwrap();

    assert_eq!(client.handler.background_counts(project, "settling"), (1, 0));
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
