//! Fetching the remotes of an open project, against local bare repositories
//! reached over `file://`: on demand, at open and on a timer, with the base's
//! "behind" following and a remote that fails leaving the review working.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{Client, config, git, init, write};
use tempfile::TempDir;
use vet_backend::{Config, Orchestrator, open_store};
use vet_protocol::{Event, ProjectId, Request, Response, ServerMessage, TaskKind, TaskState, View};

/// A project cloned from a bare `origin`, and the working repository that feeds that `origin`.
struct Remote {
    _dir: TempDir,
    bare: PathBuf,
    work: PathBuf,
    project: PathBuf,
}

impl Remote {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (work, bare, project) = (
            dir.path().join("work"),
            dir.path().join("origin.git"),
            dir.path().join("project"),
        );
        std::fs::create_dir_all(&work).unwrap();
        init(&work);
        write(&work, "a.txt", "base\n");
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "base"]);
        git(
            dir.path(),
            &["clone", "-q", "--bare", work.to_str().unwrap(), bare.to_str().unwrap()],
        );
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                &format!("file://{}", bare.display()),
                project.to_str().unwrap(),
            ],
        );
        git(&project, &["config", "core.autocrlf", "false"]);
        git(&project, &["config", "core.excludesFile", "/dev/null"]);
        Self {
            _dir: dir,
            bare,
            work,
            project,
        }
    }

    /// A commit on the remote's `main`.
    fn push_commit(&self, message: &str) {
        write(&self.work, "a.txt", message);
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "-m", message]);
        git(&self.work, &["push", "-q", self.bare.to_str().unwrap(), "main"]);
    }

    fn rev(&self, name: &str) -> String {
        git(&self.project, &["rev-parse", name])
    }
}

async fn orchestrator(fetch_interval: Duration) -> (Arc<Orchestrator>, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval,
        ..config()
    };
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config));
    (handler, data)
}

/// An interval that does not fire during a test: the fetch at open is the only periodic one.
const LONG: Duration = Duration::from_secs(3600);

/// Events until one `wanted` accepts; they are all kept in `seen`.
async fn until<T>(client: &mut Client, seen: &mut Vec<Event>, wanted: impl Fn(&Event) -> Option<T>) -> T {
    let found = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let Some(ServerMessage::Event { event }) = client.inbox.recv().await else {
                continue;
            };
            let hit = wanted(&event);
            seen.push(event);
            if let Some(hit) = hit {
                return hit;
            }
        }
    })
    .await;
    found.unwrap_or_else(|_| panic!("the event did not arrive; seen: {seen:#?}"))
}

fn behind(event: &Event) -> Option<Option<u32>> {
    match event {
        Event::BaseChanged { base, .. } => Some(base.behind),
        _ => None,
    }
}

async fn base_behind(client: &Client, project: ProjectId) -> Option<u32> {
    match client
        .call(Request::Manifest {
            project,
            view: View::Changeset,
        })
        .await
        .unwrap()
    {
        Response::Manifest(m) => m.base.behind,
        other => panic!("{other:?}"),
    }
}

fn fetch_task_states(seen: &[Event]) -> Vec<TaskState> {
    seen.iter()
        .filter_map(|e| match e {
            Event::Task(t) if t.kind == TaskKind::Fetch => Some(t.state.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_fetch_on_demand_brings_the_remote_commits_and_the_base_shows_how_far_behind_it_is() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    let mut seen = Vec::new();
    assert_eq!(
        until(&mut client, &mut seen, behind).await,
        None,
        "the fetch at open found nothing new"
    );
    let (main, head) = (remote.rev("refs/heads/main"), remote.rev("HEAD"));

    remote.push_commit("one");
    remote.push_commit("two");
    assert_eq!(base_behind(&client, project).await, None, "not fetched yet");
    let done = client.call(Request::Fetch { project }).await.unwrap();

    assert!(matches!(done, Response::Done));
    assert_eq!(
        until(&mut client, &mut seen, behind).await,
        Some(2),
        "subscribers are told"
    );
    assert_eq!(base_behind(&client, project).await, Some(2));
    assert_eq!(
        remote.rev("refs/remotes/origin/main"),
        git(&remote.work, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        (remote.rev("refs/heads/main"), remote.rev("HEAD")),
        (main, head),
        "only refs/remotes moved"
    );
    let states = fetch_task_states(&seen);
    assert_eq!(states.first(), Some(&TaskState::Running));
    assert_eq!(states.last(), Some(&TaskState::Finished));
}

#[tokio::test]
async fn opening_a_project_fetches_its_remotes() {
    let remote = Remote::new();
    remote.push_commit("one");
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);

    let project = client.open(&remote.project).await;

    let mut seen = Vec::new();
    assert_eq!(until(&mut client, &mut seen, behind).await, Some(1));
    assert_eq!(base_behind(&client, project).await, Some(1));
}

#[tokio::test]
async fn a_remote_that_fails_fails_its_task_and_the_review_goes_on() {
    let remote = Remote::new();
    std::fs::remove_dir_all(&remote.bare).unwrap();
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    let mut seen = Vec::new();

    let failed = until(&mut client, &mut seen, |e| match e {
        Event::Task(t) if t.kind == TaskKind::Fetch => match &t.state {
            TaskState::Failed { message } => Some(message.clone()),
            _ => None,
        },
        _ => None,
    })
    .await;
    let again = client.call(Request::Fetch { project }).await;

    assert!(failed.starts_with("origin: "), "names the remote: {failed}");
    assert!(
        matches!(again, Ok(Response::Done)),
        "asking again is not an error: {again:?}"
    );
    client.revision(project).await;
    assert_eq!(base_behind(&client, project).await, None, "the review still works");
}

#[tokio::test]
async fn the_periodic_fetch_fires_on_its_interval() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(Duration::from_millis(200)).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    let mut seen = Vec::new();
    until(&mut client, &mut seen, behind).await;

    remote.push_commit("one");

    assert_eq!(
        until(&mut client, &mut seen, |e| behind(e).filter(Option::is_some)).await,
        Some(1)
    );
    assert_eq!(base_behind(&client, project).await, Some(1));
    let finished = fetch_task_states(&seen)
        .iter()
        .filter(|s| **s == TaskState::Finished)
        .count();
    assert!(finished >= 2, "fetched at open and again: {seen:#?}");
}

#[tokio::test]
async fn a_closed_project_stops_fetching() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(Duration::from_millis(100)).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    let mut seen = Vec::new();
    until(&mut client, &mut seen, behind).await;
    client.call(Request::CloseProject { project }).await.unwrap();

    remote.push_commit("one");
    tokio::time::sleep(Duration::from_millis(600)).await;

    assert_ne!(
        remote.rev("refs/remotes/origin/main"),
        git(&remote.work, &["rev-parse", "HEAD"])
    );
}
