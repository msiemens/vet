//! Fetching the remotes of an open project, against local bare repositories
//! reached over `file://`: on demand, at open and on a timer, with the branches the
//! subscribers hear of and a remote that fails leaving the review working.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[cfg(debug_assertions)]
use crate::common::background_ended;
use crate::common::{Client, config, git, init, write};
use tempfile::TempDir;
use vet_backend::{Config, Orchestrator, open_store};
use vet_protocol::{
    ChangesetKey, Event, Manifest, ProjectId, Request, Response, ServerMessage, TaskKind, TaskState, VersionInfo, View,
};

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
        // Vet's fetch runs in-process and writes reflog entries, which need an identity from the repository's configuration.
        git(&project, &["config", "user.name", "t"]);
        git(&project, &["config", "user.email", "t@t"]);
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

    /// A commit adding `file` on the remote's branch `branch`, created from `main` if it is new.
    fn push_branch_commit(&self, branch: &str, file: &str) {
        let bare = self.bare.to_str().unwrap();
        if git(&self.work, &["branch", "--list", branch]).is_empty() {
            git(&self.work, &["checkout", "-q", "-b", branch, "main"]);
        } else {
            git(&self.work, &["checkout", "-q", branch]);
        }
        write(&self.work, file, &format!("{file} on {branch}\n"));
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "-m", &format!("{file} on {branch}")]);
        git(&self.work, &["push", "-q", bare, branch]);
        git(&self.work, &["checkout", "-q", "main"]);
    }

    fn rev(&self, name: &str) -> String {
        git(&self.project, &["rev-parse", name])
    }
}

fn branch_key(name: &str) -> ChangesetKey {
    ChangesetKey::Branch { name: name.into() }
}

async fn listed(client: &Client, project: ProjectId) -> Vec<ChangesetKey> {
    match client.call(Request::ListChangesets { project }).await.unwrap() {
        Response::Changesets { rows, .. } => rows.into_iter().map(|r| r.key).collect(),
        other => panic!("{other:?}"),
    }
}

async fn manifest(client: &Client, project: ProjectId) -> Manifest {
    match client
        .call(Request::Manifest {
            project,
            view: View::Changeset,
        })
        .await
        .unwrap()
    {
        Response::Manifest(m) => *m,
        other => panic!("{other:?}"),
    }
}

async fn versions(client: &Client, project: ProjectId) -> Vec<VersionInfo> {
    match client.call(Request::Versions { project }).await.unwrap() {
        Response::Versions { versions } => versions,
        other => panic!("{other:?}"),
    }
}

async fn open_changeset(client: &Client, project: ProjectId, changeset: ChangesetKey) {
    client
        .call(Request::OpenChangeset { project, changeset })
        .await
        .unwrap();
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

/// A fetch brought something: what the subscribers are told.
fn fetched(event: &Event) -> Option<()> {
    matches!(event, Event::ChangesetsChanged { .. }).then_some(())
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
async fn a_fetch_on_demand_brings_the_remote_commits_and_tells_the_subscribers() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    let mut seen = Vec::new();
    until(&mut client, &mut seen, fetched).await;
    let (main, head) = (remote.rev("refs/heads/main"), remote.rev("HEAD"));

    remote.push_commit("one");
    remote.push_commit("two");
    assert_ne!(
        remote.rev("refs/remotes/origin/main"),
        git(&remote.work, &["rev-parse", "HEAD"]),
        "not fetched yet"
    );
    let done = client.call(Request::Fetch { project }).await.unwrap();

    assert!(matches!(done, Response::Done));
    until(&mut client, &mut seen, fetched).await;
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
async fn a_branch_that_exists_only_on_the_remote_is_a_changeset_compared_from_where_it_left_main() {
    let remote = Remote::new();
    remote.push_branch_commit("feature/x", "x.txt");
    remote.push_branch_commit("feature/y", "y.txt");
    let (handler, _data) = orchestrator(LONG).await;
    let client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    client.call(Request::Fetch { project }).await.unwrap();
    let first = remote.rev("refs/remotes/origin/main");
    // A local branch of its own, and one that follows `origin/feature/y` at the same commit.
    git(&remote.project, &["checkout", "-q", "-b", "mine"]);
    write(&remote.project, "m.txt", "mine\n");
    git(&remote.project, &["add", "-A"]);
    git(&remote.project, &["commit", "-q", "-m", "mine"]);
    git(&remote.project, &["checkout", "-q", "main"]);
    git(&remote.project, &["branch", "--track", "feature/y", "origin/feature/y"]);
    let head = remote.rev("HEAD");

    let mut rows = listed(&client, project).await;
    let remote_rows = rows.split_off(3);
    rows[1..].sort_by_key(|k| format!("{k:?}"));
    assert_eq!(
        (rows, remote_rows),
        (
            vec![ChangesetKey::WorkingTree, branch_key("feature/y"), branch_key("mine")],
            vec![branch_key("origin/feature/x")]
        ),
        "local branches first; the remote one a local branch follows is listed once; the default branch is not offered"
    );

    open_changeset(&client, project, branch_key("origin/feature/x")).await;
    let m = manifest(&client, project).await;
    assert_eq!(m.target.name, "origin/feature/x");
    assert_eq!(m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["x.txt"]);
    assert_eq!(m.base.commit.id, first, "the base is where it left the default branch");
    assert_eq!(
        (remote.rev("HEAD"), git(&remote.project, &["status", "--porcelain"])),
        (head, String::new()),
        "nothing was checked out or changed"
    );

    // The remote's main moves on: the branch is still compared from where it left it.
    remote.push_commit("one");
    client.call(Request::Fetch { project }).await.unwrap();
    assert_eq!(manifest(&client, project).await.base.commit.id, first);
}

#[tokio::test]
async fn a_fetch_that_moves_a_remote_branch_is_a_new_version_of_its_changeset() {
    let remote = Remote::new();
    remote.push_branch_commit("feature/x", "x.txt");
    let (handler, _data) = orchestrator(LONG).await;
    let client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    client.call(Request::Fetch { project }).await.unwrap();
    open_changeset(&client, project, branch_key("origin/feature/x")).await;
    let m = manifest(&client, project).await;
    client
        .call(Request::ToggleHunk {
            project,
            view: View::Changeset,
            revision: m.revision,
            path: "x.txt".into(),
            hunk: 0,
        })
        .await
        .unwrap();
    assert_eq!(versions(&client, project).await.len(), 1);

    remote.push_branch_commit("feature/x", "more.txt");
    client.call(Request::Fetch { project }).await.unwrap();
    open_changeset(&client, project, ChangesetKey::WorkingTree).await;
    open_changeset(&client, project, branch_key("origin/feature/x")).await;

    let versions = versions(&client, project).await;
    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions[1].tip.as_ref().map(|t| t.id.clone()),
        Some(remote.rev("refs/remotes/origin/feature/x"))
    );
    let m = manifest(&client, project).await;
    assert_eq!(
        m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["more.txt", "x.txt"]
    );
}

#[tokio::test]
async fn opening_a_project_fetches_its_remotes() {
    let remote = Remote::new();
    remote.push_commit("one");
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);

    client.open(&remote.project).await;

    let mut seen = Vec::new();
    until(&mut client, &mut seen, fetched).await;
    assert_eq!(
        remote.rev("refs/remotes/origin/main"),
        git(&remote.work, &["rev-parse", "HEAD"])
    );
}

#[tokio::test]
async fn a_remote_that_fails_fails_its_task_and_the_review_goes_on() {
    let remote = Remote::new();
    std::fs::remove_dir_all(&remote.bare).unwrap();
    write(&remote.project, "a.txt", "edited\n");
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
    let m = manifest(&client, project).await;
    assert_eq!(m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    let marked = client
        .call(Request::ToggleHunk {
            project,
            view: View::Changeset,
            revision: m.revision,
            path: "a.txt".into(),
            hunk: 0,
        })
        .await;
    assert!(
        matches!(marked, Ok(Response::Review(_))),
        "the review still works: {marked:?}"
    );
}

#[tokio::test]
async fn the_periodic_fetch_fires_on_its_interval() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(Duration::from_millis(200)).await;
    let mut client = Client::new(&handler);
    client.open(&remote.project).await;
    let mut seen = Vec::new();
    until(&mut client, &mut seen, fetched).await;

    remote.push_commit("one");

    until(&mut client, &mut seen, fetched).await;
    assert_eq!(
        remote.rev("refs/remotes/origin/main"),
        git(&remote.work, &["rev-parse", "HEAD"])
    );
    let finished = fetch_task_states(&seen)
        .iter()
        .filter(|s| **s == TaskState::Finished)
        .count();
    assert!(finished >= 2, "fetched at open and again: {seen:#?}");
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_closed_project_stops_fetching() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(Duration::from_millis(100)).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    let mut seen = Vec::new();
    until(&mut client, &mut seen, fetched).await;
    client.call(Request::CloseProject { project }).await.unwrap();
    // The periodic task ends with the project: it cannot fetch again.
    background_ended(&client, project, "fetch").await;

    remote.push_commit("one");
    assert_ne!(
        remote.rev("refs/remotes/origin/main"),
        git(&remote.work, &["rev-parse", "HEAD"])
    );
}

/// A local branch `feature` with a commit of its own, off the clone's `main`, opened as a changeset.
async fn open_feature(remote: &Remote, client: &Client, project: ProjectId) {
    git(&remote.project, &["checkout", "-q", "-b", "feature"]);
    write(&remote.project, "f.txt", "feature\n");
    git(&remote.project, &["add", "-A"]);
    git(&remote.project, &["commit", "-q", "-m", "feature"]);
    git(&remote.project, &["checkout", "-q", "main"]);
    open_changeset(client, project, branch_key("feature")).await;
}

#[tokio::test]
async fn a_fetch_that_moves_the_default_branch_leaves_a_branch_compared_from_where_it_left_it() {
    for origin_head in [true, false] {
        let remote = Remote::new();
        if !origin_head {
            git(&remote.project, &["remote", "set-head", "origin", "-d"]);
        }
        let (handler, _data) = orchestrator(LONG).await;
        let client = Client::new(&handler);
        let project = client.open(&remote.project).await;
        open_feature(&remote, &client, project).await;
        let first = manifest(&client, project).await;

        remote.push_commit("one");
        remote.push_commit("two");
        client.call(Request::Fetch { project }).await.unwrap();

        let m = manifest(&client, project).await;
        assert_eq!(m.target.name, "feature");
        assert_eq!(
            m.base.commit.id, first.base.commit.id,
            "the base stays where it was, origin/HEAD: {origin_head}"
        );
        assert_eq!(m.files.len(), 1, "origin/HEAD: {origin_head}");
    }
}
