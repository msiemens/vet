//! Background progress: opening answers before the first scan, and the scan
//! and the diff are announced as tasks that end.

use crate::common::{Client, handler, init, repo_with};
use vet_protocol::{Event, Request, Response, ServerMessage, Task, TaskKind, TaskState};

/// The task events waiting for the client, in order.
fn announced(client: &mut Client) -> Vec<Task> {
    std::iter::from_fn(|| client.inbox.try_recv().ok())
        .filter_map(|message| match message {
            ServerMessage::Event {
                event: Event::Task(task),
            } => Some(task),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn opening_answers_before_the_first_scan_and_the_first_request_that_needs_it_reports_why_it_failed() {
    // A repository without a commit cannot be scanned: opening it used to fail.
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);

    let project = client.open(repo.path()).await;
    let Ok(Response::Tasks { .. }) = client.call(Request::Tasks { project }).await else {
        panic!("the tasks of a project that has not scanned are listed");
    };

    let first = client
        .call(Request::Manifest {
            project,
            view: vet_protocol::View::Changeset,
        })
        .await
        .unwrap_err();
    let again = client
        .call(Request::Manifest {
            project,
            view: vet_protocol::View::Changeset,
        })
        .await
        .unwrap_err();
    assert!(!first.is_empty());
    assert_eq!(first, again);
}

#[tokio::test]
async fn the_scan_and_the_diff_are_announced_as_tasks_that_end() {
    let repo = repo_with(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let (handler, _data) = handler().await;
    let mut client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client.revision(project).await;

    let tasks = announced(&mut client);
    for kind in [TaskKind::Scan, TaskKind::Diff] {
        let of_kind: Vec<_> = tasks.iter().filter(|t| t.kind == kind).collect();
        let states: Vec<_> = of_kind.iter().map(|t| (t.project, &t.state)).collect();
        assert_eq!(
            states.first(),
            Some(&(project, &TaskState::Running)),
            "{kind:?} starts: {tasks:?}"
        );
        assert_eq!(
            states.last(),
            Some(&(project, &TaskState::Finished)),
            "{kind:?} ends: {tasks:?}"
        );
        assert!(
            of_kind.iter().all(|t| t.id == of_kind[0].id),
            "one task per step: {tasks:?}"
        );
        assert!(!of_kind[0].title.is_empty());
    }

    let Response::Tasks { tasks: running } = client.call(Request::Tasks { project }).await.unwrap() else {
        panic!("not tasks");
    };
    assert!(
        running
            .iter()
            .all(|t| !matches!(t.kind, TaskKind::Scan | TaskKind::Diff))
    );
}
