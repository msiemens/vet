//! Fetching the remotes of an open project, against local bare repositories
//! reached over `file://`: on demand, at open and on a timer, with the branches the
//! subscribers hear of and a remote that fails leaving the review working.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{Client, config, git, init, write};
#[cfg(debug_assertions)]
use crate::common::{background_ended, background_live, background_started};
use tempfile::TempDir;
use vet_backend::{Config, Orchestrator, open_store};
#[cfg(debug_assertions)]
use vet_infra::platform::fake::{Fake, Seen};
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

/// Everything the client has heard and not yet read.
#[cfg(debug_assertions)]
fn pending(client: &mut Client) -> Vec<Event> {
    let mut events = Vec::new();
    while let Ok(message) = client.inbox.try_recv() {
        if let ServerMessage::Event { event } = message {
            events.push(event);
        }
    }
    events
}

/// The HTTP requests the platform fake has answered.
#[cfg(debug_assertions)]
fn calls(fake: &Fake) -> usize {
    fake.requests().len()
}

/// The requests the fake answered after the first `before`.
#[cfg(debug_assertions)]
fn since(fake: &Fake, before: usize) -> Vec<Seen> {
    fake.requests().split_off(before)
}

/// A read of the pull request's own detail, which `follow_pull_request` makes: a push is looked
/// for there, not in the discussion.
#[cfg(debug_assertions)]
fn is_pull_request(request: &Seen) -> bool {
    request.body.contains("baseRefOid")
}

/// A read of the project's pull request list, which the feed makes on every tick.
#[cfg(debug_assertions)]
fn is_listing(request: &Seen) -> bool {
    request.path.starts_with("/repos/acme/app/pulls") || request.body.contains("search(query:")
}

/// A read of the pull request's discussion, which the comment sync makes.
#[cfg(debug_assertions)]
fn is_discussion(request: &Seen) -> bool {
    request.body.contains("body isDraft state")
}

/// Waits until the fake has seen no new request for long enough that the work of opening the
/// project, its pull request's own sync and its checks is over.
#[cfg(debug_assertions)]
async fn at_rest(fake: &Fake) {
    let mut last = calls(fake);
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let now = calls(fake);
        if now == last {
            return;
        }
        last = now;
    }
    panic!("the platform was still being asked");
}

/// Waits for one periodic fetch to run and end; its announcement and its platform calls are part
/// of it, so once it has ended they have happened.
#[cfg(debug_assertions)]
async fn next_tick(client: &Client, project: ProjectId) {
    let before = background_started(client, project, "fetch");
    while background_started(client, project, "fetch") <= before {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    background_live(client, project, "fetch", 0).await;
}

#[tokio::test]
async fn a_fetch_on_demand_brings_the_remote_commits_and_tells_the_subscribers() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    let mut seen = Vec::new();
    // The timer has nothing to fetch: the remote is no platform's.
    client.call(Request::WaitForQuiet { project }).await.unwrap();
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

/// The message a fetch task failed with.
fn fetch_failure(event: &Event) -> Option<String> {
    match event {
        Event::Task(t) if t.kind == TaskKind::Fetch => match &t.state {
            TaskState::Failed { message } => Some(message.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn opening_a_project_fetches_its_platform_remote_over_https_and_never_runs_ssh() {
    let (_mock, fake, repo, scratch) = crate::prefetch::fixture();
    let main = crate::prefetch::push_to_main(scratch.path());
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: LONG,
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;

    let project = client.open(repo.path()).await;

    let mut seen = Vec::new();
    until(&mut client, &mut seen, fetched).await;
    client.call(Request::WaitForQuiet { project }).await.unwrap();
    assert_eq!(git(repo.path(), &["rev-parse", "refs/remotes/origin/main"]), main);
    assert_eq!(crate::prefetch::ssh_runs(scratch.path()), 0);
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

    let again = client.call(Request::Fetch { project }).await;
    let failed = until(&mut client, &mut seen, fetch_failure).await;

    assert!(failed.starts_with("origin: "), "names the remote: {failed}");
    assert!(matches!(again, Ok(Response::Done)), "asking is not an error: {again:?}");
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

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_platform_remote_that_cannot_be_reached_fails_the_fetch_at_open() {
    let (_mock, fake, repo, scratch) = crate::prefetch::fixture();
    std::fs::remove_dir_all(crate::prefetch::served_repo(scratch.path())).unwrap();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: LONG,
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;

    client.open(repo.path()).await;

    let mut seen = Vec::new();
    let failed = until(&mut client, &mut seen, fetch_failure).await;
    assert!(failed.starts_with("origin: "), "names the remote: {failed}");
}

/// The report of a fetch of the platform remotes, once it has ended.
fn remote_fetch_report(event: &Event) -> Option<vet_protocol::Task> {
    match event {
        Event::Task(t) if t.kind == TaskKind::RemoteFetch && t.state != TaskState::Running => Some(t.clone()),
        _ => None,
    }
}

/// The outcomes of the reports of fetches of the platform remotes, in the order announced.
fn remote_fetch_states(events: &[Event]) -> Vec<TaskState> {
    events.iter().filter_map(remote_fetch_report).map(|t| t.state).collect()
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_failed_fetch_shows_a_failed_report_and_a_later_fetch_clears_it() {
    let (_mock, fake, repo, scratch) = crate::prefetch::fixture();
    let served = crate::prefetch::served_repo(scratch.path());
    let aside = scratch.path().join("served-aside");
    std::fs::rename(&served, &aside).unwrap();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: LONG,
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;
    let mut seen = Vec::new();

    let failed = until(&mut client, &mut seen, remote_fetch_report).await;
    assert_eq!(failed.title, "Fetch");
    assert!(
        matches!(&failed.state, TaskState::Failed { message } if message.starts_with("origin: ")),
        "names the remote: {failed:?}"
    );

    std::fs::rename(&aside, &served).unwrap();
    client.call(Request::Fetch { project }).await.unwrap();
    client.call(Request::WaitForQuiet { project }).await.unwrap();

    seen.extend(pending(&mut client));
    let states = remote_fetch_states(&seen);
    assert_eq!(states.len(), 2, "one report per fetch: {states:?}");
    assert!(matches!(states[0], TaskState::Failed { .. }), "{states:?}");
    assert_eq!(states[1], TaskState::Finished);
}

/// A timer fetch that has no platform remote to take reports nothing.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_fetch_with_nothing_to_fetch_reports_nothing() {
    let remote = Remote::new();
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    background_ended(&client, project, "fetch").await;

    let heard = pending(&mut client);

    assert!(
        !heard
            .iter()
            .any(|e| matches!(e, Event::Task(t) if matches!(t.kind, TaskKind::Fetch | TaskKind::RemoteFetch))),
        "a fetch of nothing was reported: {heard:#?}"
    );
}

/// The footer counts the platform remotes only: the viewer's fetch of a remote on another host that
/// fails is a task of its own and no report.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_viewer_fetch_of_other_hosts_only_reports_nothing() {
    let remote = Remote::new();
    std::fs::remove_dir_all(&remote.bare).unwrap();
    let (handler, _data) = orchestrator(LONG).await;
    let mut client = Client::new(&handler);
    let project = client.open(&remote.project).await;
    background_ended(&client, project, "fetch").await;
    pending(&mut client);

    client.call(Request::Fetch { project }).await.unwrap();

    let heard = pending(&mut client);
    assert!(
        heard.iter().any(|e| fetch_failure(e).is_some()),
        "the viewer's fetch did not fail: {heard:#?}"
    );
    assert_eq!(remote_fetch_states(&heard), []);
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn the_periodic_fetch_fires_on_its_interval() {
    let (_mock, fake, repo, scratch) = crate::prefetch::fixture();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: Duration::from_millis(1500),
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;
    settle(&mut client, &fake, project, false).await;

    let main = crate::prefetch::push_to_main(scratch.path());
    next_tick(&client, project).await;

    assert_eq!(git(repo.path(), &["rev-parse", "refs/remotes/origin/main"]), main);
}

/// Opens pull request 7 of the fake platform on view.
#[cfg(debug_assertions)]
async fn open_pr(client: &Client, project: ProjectId) {
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::PullRequest {
                remote: crate::prefetch::remote().to_string(),
                number: 7,
            },
        })
        .await
        .unwrap();
}

/// Waits for what opening the project and its pull request left running: the fetch at open, the
/// pull request's own sync and, when there is one, its checks. Then nothing but a periodic tick
/// asks the platform.
#[cfg(debug_assertions)]
async fn settle(client: &mut Client, fake: &Fake, project: ProjectId, checks: bool) {
    let mut seen = Vec::new();
    until(client, &mut seen, fetched).await;
    background_live(client, project, "fetch", 0).await;
    if checks {
        background_ended(client, project, "checks").await;
    }
    at_rest(fake).await;
    pending(client);
}

/// A tick that moved no ref announces nothing and does not follow the pull request on view, but
/// still syncs its comments, which change without a push, and has the feed list again.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_periodic_fetch_that_moved_no_ref_syncs_only_the_comments() {
    let (_mock, fake, repo, _scratch) = crate::prefetch::fixture();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: Duration::from_millis(1500),
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;
    open_pr(&client, project).await;
    settle(&mut client, &fake, project, true).await;

    let before = calls(&fake);
    next_tick(&client, project).await;
    client.call(Request::WaitForQuiet { project }).await.unwrap();
    let heard = pending(&mut client);
    let asked = since(&fake, before);

    assert!(
        !heard.iter().any(|e| matches!(e, Event::ChangesetsChanged { .. })),
        "a tick with no ref moved recounted the queue: {heard:#?}"
    );
    assert!(
        !asked.iter().any(is_pull_request),
        "a tick with no ref moved still looked at the pull request: {asked:#?}"
    );
    assert!(
        asked.iter().any(is_discussion),
        "a tick with no ref moved did not sync the comments: {heard:#?}"
    );
    assert!(
        asked.iter().any(is_listing),
        "a tick did not have the feed list again: {asked:#?}"
    );
    assert_eq!(
        asked.len(),
        asked.iter().filter(|r| is_discussion(r) || is_listing(r)).count(),
        "a tick with no ref moved asked the platform for more than the comments and the list: {asked:#?}"
    );
}

/// A tick that moved no ref with no pull request on view asks the platform for the list only, and
/// announces a list that did not change no more than a recount.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_periodic_fetch_that_moved_no_ref_without_a_pull_request_asks_only_for_the_list() {
    let (_mock, fake, repo, _scratch) = crate::prefetch::fixture();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: Duration::from_millis(1500),
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;
    settle(&mut client, &fake, project, false).await;

    let before = calls(&fake);
    next_tick(&client, project).await;
    client.call(Request::WaitForQuiet { project }).await.unwrap();
    let heard = pending(&mut client);
    let asked = since(&fake, before);

    assert!(
        !heard.iter().any(|e| matches!(e, Event::ChangesetsChanged { .. })),
        "a tick with no ref moved recounted the queue: {heard:#?}"
    );
    assert!(
        !heard.iter().any(|e| matches!(e, Event::PullRequestListChanged { .. })),
        "a tick that found the same list announced it: {heard:#?}"
    );
    assert!(
        !asked.is_empty() && asked.iter().all(is_listing),
        "a tick with no pull request on view asked for more than the list: {asked:#?}"
    );
}

/// A tick lists again, and a list that changed meanwhile is announced.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_periodic_tick_announces_the_list_when_it_changed() {
    let (mock, fake, repo, _scratch) = crate::prefetch::fixture();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: Duration::from_millis(1500),
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;
    settle(&mut client, &fake, project, false).await;

    crate::prefetch::add_ninth(&mock);
    next_tick(&client, project).await;
    client.call(Request::WaitForQuiet { project }).await.unwrap();

    let heard = pending(&mut client);
    assert_eq!(
        heard
            .iter()
            .filter(|e| matches!(e, Event::PullRequestListChanged { .. }))
            .count(),
        1,
        "{heard:#?}"
    );
}

/// A tick that moved a ref announces it, follows the pull request on view and syncs its comments.
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_periodic_fetch_that_moved_a_ref_announces_follows_and_syncs() {
    let (_mock, fake, repo, scratch) = crate::prefetch::fixture();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: Duration::from_millis(1500),
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;
    open_pr(&client, project).await;
    settle(&mut client, &fake, project, true).await;

    // The remote's `main` moves on, as a push to the repository does.
    git(
        &scratch.path().join("work"),
        &[
            "push",
            "-q",
            crate::prefetch::served_repo(scratch.path()).to_str().unwrap(),
            "feature:refs/heads/main",
        ],
    );

    let before = calls(&fake);
    next_tick(&client, project).await;
    let heard = pending(&mut client);
    let asked = since(&fake, before);

    assert!(
        heard.iter().any(|e| matches!(e, Event::ChangesetsChanged { .. })),
        "a tick that moved a ref did not recount the queue: {heard:#?}"
    );
    assert!(
        asked.iter().any(is_pull_request),
        "a tick that moved a ref did not look at the pull request: {asked:#?}"
    );
    assert!(
        asked.iter().any(is_discussion),
        "a tick that moved a ref did not sync the comments: {asked:#?}"
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_closed_project_stops_fetching() {
    let (_mock, fake, repo, scratch) = crate::prefetch::fixture();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: Duration::from_millis(500),
        ..crate::prefetch::config()
    };
    let (_backend, mut client) = crate::prefetch::backend_with(data.path(), &fake.base(), config).await;
    let project = client.open(repo.path()).await;
    let mut seen = Vec::new();
    until(&mut client, &mut seen, fetched).await;
    client.call(Request::CloseProject { project }).await.unwrap();
    // The periodic task ends with the project: it cannot fetch again.
    background_ended(&client, project, "fetch").await;

    let main = crate::prefetch::push_to_main(scratch.path());
    assert_ne!(git(repo.path(), &["rev-parse", "refs/remotes/origin/main"]), main);
}

/// A project whose `origin` is reached through a wrapper that notes every fetch in a log and holds
/// it until the gate file exists, so a fetch can be seen to be under way and counted.
#[cfg(debug_assertions)]
struct Gated {
    remote: Remote,
    log: PathBuf,
    gate: PathBuf,
}

#[cfg(debug_assertions)]
impl Gated {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let remote = Remote::new();
        let dir = remote._dir.path();
        let (log, gate, wrapper) = (dir.join("fetches"), dir.join("gate"), dir.join("ssh-gated.sh"));
        // The wait ends with the directory, so a fetch nobody opened the gate for does not outlive
        // the test.
        let script = format!(
            "#!/bin/sh\necho run >> '{}'\nwhile [ ! -e '{}' ] && [ -d '{}' ]; do sleep 0.02; done\n\
             exec git-upload-pack '{}'\n",
            log.display(),
            gate.display(),
            dir.display(),
            remote.bare.display()
        );
        std::fs::write(&wrapper, script).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let project = &remote.project;
        git(project, &["remote", "set-url", "origin", "ssh://git@example.com/x.git"]);
        git(project, &["config", "core.sshCommand", wrapper.to_str().unwrap()]);
        git(project, &["config", "ssh.variant", "ssh"]);
        Self { remote, log, gate }
    }

    /// How many fetches reached the remote.
    fn fetches(&self) -> usize {
        std::fs::read_to_string(&self.log).map_or(0, |text| text.lines().count())
    }

    async fn until_fetching(&self, fetches: usize) {
        let waiting = async {
            while self.fetches() < fetches {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(30), waiting)
            .await
            .expect("the remote was never fetched");
    }

    fn open_gate(&self) {
        std::fs::write(&self.gate, "").unwrap();
    }
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_remote_on_another_host_is_fetched_only_when_the_viewer_asks() {
    let gated = Gated::new();
    gated.open_gate();
    gated.remote.push_commit("one");
    let (handler, _data) = orchestrator(LONG).await;
    let client = Client::new(&handler);
    let project = client.open(&gated.remote.project).await;
    background_ended(&client, project, "fetch").await;

    assert_eq!(
        gated.fetches(),
        0,
        "the timer reached a remote no platform account covers"
    );
    assert_ne!(
        gated.remote.rev("refs/remotes/origin/main"),
        git(&gated.remote.work, &["rev-parse", "HEAD"])
    );

    client.call(Request::Fetch { project }).await.unwrap();

    assert_eq!(gated.fetches(), 1);
    assert_eq!(
        gated.remote.rev("refs/remotes/origin/main"),
        git(&gated.remote.work, &["rev-parse", "HEAD"])
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn the_timer_counts_down_again_after_a_viewer_fetch() {
    let gated = Gated::new();
    gated.open_gate();
    let (handler, _data) = orchestrator(Duration::from_millis(2000)).await;
    let client = Client::new(&handler);
    let project = client.open(&gated.remote.project).await;
    background_ended(&client, project, "fetch").await;
    let started = || background_started(&client, project, "fetch");

    // Halfway to the next tick the viewer fetches: the tick that was due at 2 s is not.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    client.call(Request::Fetch { project }).await.unwrap();
    assert_eq!(started(), 2);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(started(), 2, "the countdown went on from the first fetch");

    let waiting = async {
        while started() < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .expect("the timer fetched again");
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn closing_the_project_ends_a_slow_fetch_within_the_bound() {
    let gated = Gated::new();
    let data = tempfile::tempdir().unwrap();
    let config = Config {
        fetch_interval: LONG,
        shutdown_bound: Duration::from_millis(100),
        ..config()
    };
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config));
    let client = Client::new(&handler);
    let project = client.open(&gated.remote.project).await;

    // The viewer's fetch is the one that reaches the remote, and it waits at the gate.
    let (_, closing) = tokio::join!(client.call(Request::Fetch { project }), async {
        gated.until_fetching(1).await;
        let closing = std::time::Instant::now();
        client.call(Request::CloseProject { project }).await.unwrap();
        closing.elapsed()
    });

    assert!(closing < Duration::from_secs(1), "closing took {closing:?}");
    assert_eq!(
        handler.background_counts(project, "fetch").1,
        0,
        "the fetch outlived the project"
    );
    gated.open_gate();
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
