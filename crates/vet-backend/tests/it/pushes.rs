//! Taking a push in on a platform's pull request (B19): when the platform records a new state of
//! the source branch, the next fetch takes it as a new version and the review state is remapped
//! onto it (§5.2), so lines the push changed are not seen any more.
//!
//! These cover the ways Vet learns of a push — an explicit fetch, the fetch interval, the queue's
//! listing — on Azure DevOps (a branch in the same repository, a fork, and a platform whose own
//! `lastMergeSourceCommit` lags its newest iteration) and on GitHub. One test needs `python3` for a
//! fake language server: it is ignored and run by `ci/tool-tests.sh`.
//!
//! The project's remote is an ssh URL that names `dev.azure.com/contoso/Shop/api`, but the fetch
//! goes over HTTPS to a bare repository on disk (`Orchestrator::with_served_platforms`): no network
//! is touched, and `core.sshCommand` is a tripwire that fails the tests that run it. The platform's
//! HTTP API is the fake server the tests point the backend at
//! (`Orchestrator::with_platform_endpoints`).

#![allow(clippy::disallowed_methods)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use vet_backend::{Config, Orchestrator, open_store};
use vet_infra::platform::fake::{Canned, Fake};
use vet_protocol::{ChangesetKey, HunkState, Manifest, ProjectId, Request, Response, Secret, Side, UnavailableKind};

use crate::common::{Client, git, init, write};

/// `n` lines, with the given ones edited.
fn text(n: usize, edited: &[usize]) -> String {
    (0..n)
        .map(|i| {
            if edited.contains(&i) {
                format!("edited {i}\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect()
}

/// The states of the source branch the fake lists, oldest first.
#[derive(Default)]
pub(crate) struct Mock {
    head: String,
    /// The base commit every iteration shares.
    base: String,
    /// The source branch is in a fork's repository.
    fork: bool,
    iterations: Vec<(i32, &'static str, String)>,
    /// The list leaves the pull request out, as it does one that is neither the viewer's nor
    /// waiting for them; the pull request itself still answers.
    unlisted: bool,
}

fn identity(id: &str, address: &str) -> Value {
    json!({ "id": id, "displayName": "Grace", "uniqueName": address, "isContainer": false })
}

fn profile() -> String {
    json!({ "authenticatedUser": {
        "id": "me-id",
        "providerDisplayName": "Ada Lovelace",
        "properties": { "Account": { "$type": "System.String", "$value": "ada@contoso.com" } }
    } })
    .to_string()
}

fn no_value() -> String {
    r#"{"count":0,"value":[]}"#.to_owned()
}

/// One pull request, as the fake answers it, with the given head and source states.
fn pull_request(mock: &Mock, number: u64) -> String {
    let fork = mock.fork.then(|| {
        json!({
            "name": "refs/heads/feature", "objectId": mock.head,
            "repository": { "id": "f", "name": "api-fork", "url": "https://dev.azure.com/x",
                            "project": { "name": "Shop", "visibility": "private" } }
        })
    });
    json!({
        "pullRequestId": number,
        "title": format!("PR {number}"),
        "description": "Why it exists.",
        "createdBy": identity("grace-id", "grace@contoso.com"),
        "creationDate": "2024-05-01T12:00:00Z",
        "isDraft": false,
        "labels": [],
        "sourceRefName": "refs/heads/feature",
        "targetRefName": "refs/heads/main",
        "status": "active",
        "url": format!("https://dev.azure.com/contoso/_apis/git/repositories/api/pullRequests/{number}"),
        "repository": {
            "id": "r", "name": "api",
            "url": "https://dev.azure.com/contoso/_apis/git/repositories/api",
            "project": { "name": "Shop", "visibility": "private" }
        },
        "lastMergeSourceCommit": { "commitId": mock.head },
        "forkSource": fork,
        "reviewers": [],
    })
    .to_string()
}

fn iterations(mock: &Mock) -> String {
    let value: Vec<Value> = mock
        .iterations
        .iter()
        .map(|(id, reason, commit)| {
            json!({
                "id": id,
                "reason": reason,
                "createdDate": format!("2024-05-0{id}T12:00:00Z"),
                "sourceRefCommit": { "commitId": commit },
                "commonRefCommit": { "commitId": mock.base },
            })
        })
        .collect();
    json!({ "count": value.len(), "value": value }).to_string()
}

const PULLS: &str = "/contoso/shop/_apis/git/repositories/api/pullrequests";

/// A server that answers like Azure DevOps for the pull request `number`: the viewer, the pull
/// request and its iterations, and no statuses or threads.
fn azure_devops(mock: Arc<Mutex<Mock>>, number: u64) -> Fake {
    Fake::start(move |request, _| {
        let path = request.path.split('?').next().unwrap().to_lowercase();
        let path = path.as_str();
        if path.ends_with("/_apis/connectiondata") {
            return Canned::json(&profile());
        }
        if path.ends_with("/_apis/identities") {
            return Canned::json(&no_value());
        }
        if path == format!("{PULLS}/{number}/iterations") {
            return Canned::json(&iterations(&mock.lock().unwrap()));
        }
        if path == format!("{PULLS}/{number}") {
            return Canned::json(&pull_request(&mock.lock().unwrap(), number));
        }
        if path == PULLS {
            let mock = mock.lock().unwrap();
            if mock.unlisted {
                return Canned::json(&no_value());
            }
            let value = serde_json::from_str::<Value>(&pull_request(&mock, number)).unwrap();
            return Canned::json(&json!({ "count": 1, "value": [value] }).to_string());
        }
        if path.ends_with("/statuses") || path.ends_with("/threads") {
            return Canned::json(&no_value());
        }
        let message = json!({ "message": format!("unexpected {}", request.path) }).to_string();
        Canned::status(404, &message)
    })
}

/// What the fake repository serves, and its work tree.
pub(crate) struct Served {
    base: String,
    head1: String,
    head2: String,
    work: PathBuf,
    bare: PathBuf,
}

/// A bare repository on disk, at the first push to its source branch, with the second push
/// committed but not yet pushed. `main` is at the base commit.
fn served(dir: &Path) -> (Served, PathBuf) {
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    init(&work);
    write(&work, "a.txt", &text(30, &[]));
    write(&work, "b.txt", &text(30, &[]));
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "base"]);
    let base = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["checkout", "-q", "-b", "feature"]);
    write(&work, "a.txt", &text(30, &[3, 20]));
    write(&work, "b.txt", &text(30, &[5]));
    // A file the pull request adds, which the second push edits.
    write(&work, "c.txt", &text(30, &[4]));
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "first push"]);
    let head1 = git(&work, &["rev-parse", "HEAD"]);
    let bare = dir.join("dev.azure.com/contoso/Shop/_git/api");
    std::fs::create_dir_all(&bare).unwrap();
    git(dir, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    // The earlier push is fetched by commit, as Azure DevOps serves it.
    git(&bare, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    git(
        &work,
        &[
            "push",
            "-q",
            bare.to_str().unwrap(),
            "main",
            "feature:refs/heads/feature",
        ],
    );
    // The second push rewrites the line 3 that was seen and adds line 25: a normal fast-forward.
    let second = text(30, &[20, 25]).replace("line 3\n", "changed 3\n");
    write(&work, "a.txt", &second);
    write(&work, "c.txt", &text(30, &[4, 15]));
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "second push"]);
    let head2 = git(&work, &["rev-parse", "HEAD"]);
    let wrapper = tripwire(dir);
    (
        Served {
            base,
            head1,
            head2,
            work,
            bare,
        },
        wrapper,
    )
}

/// An `ssh` that records that it ran, in `dir/ssh-runs`, and fails: a platform fetch never runs it.
fn tripwire(dir: &Path) -> PathBuf {
    let wrapper = dir.join("ssh-fake.sh");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\necho run >> '{}'\nexit 1\n", dir.join("ssh-runs").display()),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    wrapper
}

/// A project whose `origin` is `dev.azure.com/contoso/Shop/api`, fetched through `wrapper`.
fn project(wrapper: &Path) -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    write(repo.path(), "a.txt", &text(30, &[]));
    write(repo.path(), "b.txt", &text(30, &[]));
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-qm", "base"]);
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "ssh://git@ssh.dev.azure.com/v3/contoso/Shop/api",
        ],
    );
    git(repo.path(), &["config", "core.sshCommand", wrapper.to_str().unwrap()]);
    git(repo.path(), &["config", "ssh.variant", "ssh"]);
    git(
        repo.path(),
        &[
            "config",
            "vet.servedPlatforms",
            wrapper.parent().unwrap().to_str().unwrap(),
        ],
    );
    repo
}

fn config() -> Config {
    config_fetching_every(Config::default().fetch_interval)
}

fn config_fetching_every(fetch_interval: Duration) -> Config {
    Config {
        debounce: Duration::from_millis(150),
        poll: Duration::from_millis(300),
        orphan_grace: Duration::from_secs(60),
        fetch_interval,
        prefetch_pace: Duration::ZERO,
        ..Config::default()
    }
}

/// The remote key of the fake repository.
pub(crate) fn remote() -> &'static str {
    "dev.azure.com/contoso/Shop/api"
}

pub(crate) struct Rig {
    pub(crate) client: Client,
    pub(crate) project: ProjectId,
    _repo: tempfile::TempDir,
    _data: tempfile::TempDir,
    _scratch: tempfile::TempDir,
}

/// A backend whose Azure DevOps answers at `fake`, with the credential the fake accepts.
async fn backend(data: &Path, fake: &Fake) -> (Arc<Orchestrator>, Client) {
    backend_fetching_every(data, fake, config().fetch_interval).await
}

/// A backend whose remotes are fetched every `fetch_interval`.
async fn backend_fetching_every(data: &Path, fake: &Fake, fetch_interval: Duration) -> (Arc<Orchestrator>, Client) {
    let orchestrator = Orchestrator::new(open_store(data).await.unwrap(), config_fetching_every(fetch_interval))
        .with_platform_endpoints(None, Some(&fake.base()))
        .with_served_platforms();
    signed_in(orchestrator).await
}

/// `orchestrator` with a client that holds the credential the fake accepts.
async fn signed_in(orchestrator: Orchestrator) -> (Arc<Orchestrator>, Client) {
    let orchestrator = Arc::new(orchestrator);
    let client = Client::new(&orchestrator);
    client
        .call(Request::SetCredentials {
            host: "dev.azure.com/contoso".into(),
            token: Secret("pat_secret".into()),
            validate: false,
        })
        .await
        .unwrap();
    (orchestrator, client)
}

impl Rig {
    pub(crate) async fn start(fake: &Fake, repo: tempfile::TempDir, scratch: tempfile::TempDir) -> Self {
        let data = tempfile::tempdir().unwrap();
        let (_orchestrator, client) = backend(data.path(), fake).await;
        let project = client.open(repo.path()).await;
        Self {
            client,
            project,
            _repo: repo,
            _data: data,
            _scratch: scratch,
        }
    }

    /// As `start`, the symbols analysis of `path` failing as `failures` says, each try pausing
    /// before the next: the analysis of a file stays under way for as long as those take.
    #[cfg(debug_assertions)]
    pub(crate) async fn start_failing(
        fake: &Fake,
        repo: tempfile::TempDir,
        scratch: tempfile::TempDir,
        path: &str,
        failures: Vec<vet_backend::Injected>,
    ) -> Self {
        let data = tempfile::tempdir().unwrap();
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), config())
            .with_platform_endpoints(None, Some(&fake.base()))
            .with_served_platforms()
            .with_analysis_failing(path, failures);
        let (_orchestrator, client) = signed_in(orchestrator).await;
        let project = client.open(repo.path()).await;
        Self {
            client,
            project,
            _repo: repo,
            _data: data,
            _scratch: scratch,
        }
    }

    pub(crate) async fn open(&self) {
        self.client
            .call(Request::OpenChangeset {
                project: self.project,
                changeset: ChangesetKey::PullRequest {
                    remote: remote().into(),
                    number: 7,
                },
            })
            .await
            .unwrap();
    }

    async fn manifest(&self) -> Manifest {
        match self
            .client
            .call(Request::Manifest {
                project: self.project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .unwrap()
        {
            Response::Manifest(m) => *m,
            other => panic!("{other:?}"),
        }
    }

    pub(crate) async fn toggle_file(&self, path: &str) {
        let revision = self.manifest().await.revision;
        self.client
            .call(Request::ToggleFileSeen {
                view: vet_protocol::View::Changeset,
                project: self.project,
                revision,
                path: path.into(),
            })
            .await
            .unwrap();
    }

    /// Marks every changed file of the changeset seen in one request, as the bulk action does.
    async fn set_all_seen(&self) -> Vec<String> {
        let m = self.manifest().await;
        let paths: Vec<String> = m.files.iter().map(|f| f.path.clone()).collect();
        self.client
            .call(Request::SetFilesSeen {
                view: vet_protocol::View::Changeset,
                project: self.project,
                revision: m.revision,
                paths: paths.clone(),
                seen: true,
            })
            .await
            .unwrap();
        paths
    }

    pub(crate) async fn fetch(&self) {
        self.client
            .call(Request::Fetch { project: self.project })
            .await
            .unwrap();
    }
}

fn hunks(m: &Manifest, path: &str) -> Vec<HunkState> {
    m.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is not in the manifest"))
        .review
        .hunks
        .clone()
}

async fn manifest_of(client: &Client, project: ProjectId) -> Manifest {
    match client
        .call(Request::Manifest {
            project,
            view: vet_protocol::View::Changeset,
        })
        .await
        .unwrap()
    {
        Response::Manifest(m) => *m,
        other => panic!("{other:?}"),
    }
}

/// The fake for a pull request 7 whose source branch is at its first push.
pub(crate) fn fixture() -> (Arc<Mutex<Mock>>, Fake, tempfile::TempDir, tempfile::TempDir, Served) {
    let scratch = tempfile::tempdir().unwrap();
    let (served, wrapper) = served(scratch.path());
    let mock = Arc::new(Mutex::new(Mock {
        head: served.head1.clone(),
        base: served.base.clone(),
        fork: false,
        iterations: vec![(1, "create", served.head1.clone())],
        unlisted: false,
    }));
    let fake = azure_devops(mock.clone(), 7);
    let repo = project(&wrapper);
    (mock, fake, repo, scratch, served)
}

#[tokio::test]
async fn a_push_that_changes_seen_lines_leaves_them_and_their_file_unseen() {
    let (mock, fake, repo, scratch, served) = fixture();
    push_case(&mock, &fake, repo, scratch, &served, push_again).await;
}

#[tokio::test]
async fn a_push_to_a_fork_leaves_the_changed_lines_unseen() {
    let (mock, fake, repo, scratch, served) = fixture();
    mock.lock().unwrap().fork = true;
    push_case(&mock, &fake, repo, scratch, &served, push_again).await;
}

#[tokio::test]
async fn a_push_the_platform_still_names_the_old_head_for_leaves_the_changed_lines_unseen() {
    let (mock, fake, repo, scratch, served) = fixture();
    push_case(&mock, &fake, repo, scratch, &served, push_branch_with_a_stale_detail).await;
}

/// B19: the source branch is in a fork, so its commit is fetched by id — the id the pull request
/// names, which Azure DevOps left behind the newest iteration it recorded. The push must still be
/// taken in, as the newest iteration's commit, and the line it changed must not stay seen.
#[tokio::test]
async fn a_fork_push_the_platform_names_an_old_head_for_is_taken_in() {
    let (mock, fake, repo, scratch, served) = fixture();
    mock.lock().unwrap().fork = true;
    push_case(&mock, &fake, repo, scratch, &served, push_branch_with_a_stale_detail).await;
}

/// The feed lists again on every fetch and tick, and the platform's list still names the old head:
/// the newer head a push brought in stays where it is.
#[tokio::test]
async fn a_feed_cycle_leaves_a_head_newer_than_the_platforms_list_in_place() {
    let (mock, fake, repo, scratch, served) = fixture();
    mock.lock().unwrap().fork = true;
    let rig = Rig::start(&fake, repo, scratch).await;
    rig.open().await;
    rig.toggle_file("a.txt").await;

    push_branch_with_a_stale_detail(&mock, &served);
    rig.fetch().await;
    assert_eq!(hunks(&rig.manifest().await, "a.txt").len(), 3, "the push is taken in");

    rig.fetch().await;
    rig.client
        .call(Request::WaitForQuiet { project: rig.project })
        .await
        .unwrap();

    let refs = git(
        rig._repo.path(),
        &["for-each-ref", "--format=%(refname) %(objectname)", "refs/vet/pr/"],
    );
    let held = refs
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find_map(|(name, commit)| name.ends_with("/7/head").then_some(commit))
        .unwrap_or_else(|| panic!("no head ref of #7 in {refs}"));
    assert_eq!(held, served.head2, "the ref is still at the newest push");
    assert_eq!(hunks(&rig.manifest().await, "a.txt").len(), 3);
}

/// The same, with every changed file marked seen in one bulk request rather than one by one.
#[tokio::test]
async fn a_push_after_marking_everything_seen_leaves_the_changed_lines_unseen() {
    let (mock, fake, repo, scratch, served) = fixture();
    let rig = Rig::start(&fake, repo, scratch).await;
    rig.open().await;
    let paths = rig.set_all_seen().await;
    assert_eq!(paths.len(), 3, "every changed file");
    assert_eq!(
        hunks(&rig.manifest().await, "a.txt"),
        [HunkState::Seen, HunkState::Seen]
    );
    assert_eq!(hunks(&rig.manifest().await, "b.txt"), [HunkState::Seen]);

    push_again(&mock, &served);
    rig.fetch().await;

    let m = rig.manifest().await;
    assert_eq!(
        hunks(&m, "a.txt"),
        [HunkState::Unseen, HunkState::Seen, HunkState::Unseen]
    );
    assert_eq!(hunks(&m, "b.txt"), [HunkState::Seen], "an untouched file stays seen");
}

/// Opens pull request 7, marks both files seen, pushes a third edit and takes it in: the pushed
/// line is unseen, the lines the push did not change stay seen, and the untouched file stays seen.
async fn push_case(
    mock: &Arc<Mutex<Mock>>,
    fake: &Fake,
    repo: tempfile::TempDir,
    scratch: tempfile::TempDir,
    served: &Served,
    push: fn(&Arc<Mutex<Mock>>, &Served),
) {
    let rig = Rig::start(fake, repo, scratch).await;
    rig.open().await;

    // Two changed files and one the pull request adds, marked seen as files.
    let m = rig.manifest().await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Unseen, HunkState::Unseen]);
    assert_eq!(hunks(&m, "b.txt"), [HunkState::Unseen]);
    assert_eq!(
        hunks(&m, "c.txt"),
        [HunkState::Unseen],
        "the file the pull request adds"
    );
    rig.toggle_file("a.txt").await;
    rig.toggle_file("b.txt").await;
    rig.toggle_file("c.txt").await;
    assert_eq!(
        hunks(&rig.manifest().await, "a.txt"),
        [HunkState::Seen, HunkState::Seen]
    );
    assert_eq!(hunks(&rig.manifest().await, "b.txt"), [HunkState::Seen]);
    assert_eq!(hunks(&rig.manifest().await, "c.txt"), [HunkState::Seen]);

    push(mock, served);
    rig.fetch().await;

    // The push is taken in as a new version and the seen state remapped: the rewritten line is
    // unseen, the lines it left stay seen.
    let m = rig.manifest().await;
    assert_eq!(
        hunks(&m, "a.txt"),
        [HunkState::Unseen, HunkState::Seen, HunkState::Unseen],
        "the rewritten line is not seen"
    );
    assert_eq!(hunks(&m, "b.txt"), [HunkState::Seen], "an untouched file stays seen");
    assert_eq!(
        hunks(&m, "c.txt"),
        [HunkState::Unseen],
        "the added file's edited lines are not seen"
    );
}

/// The author's push, recorded by the platform, as the fake serves it.
pub(crate) fn push_again(mock: &Arc<Mutex<Mock>>, served: &Served) {
    push_branch(served);
    let mut mock = mock.lock().unwrap();
    mock.head = served.head2.clone();
    mock.iterations.push((2, "push", served.head2.clone()));
}

/// The push is on the source branch and the iterations, but the platform's single-pull-request
/// answer still names the old head (its `lastMergeSourceCommit` lags).
fn push_branch_with_a_stale_detail(mock: &Arc<Mutex<Mock>>, served: &Served) {
    push_branch(served);
    mock.lock().unwrap().iterations.push((2, "push", served.head2.clone()));
}

fn push_branch(served: &Served) {
    git(
        &served.work,
        &[
            "push",
            "-q",
            served.bare.to_str().unwrap(),
            "feature:refs/heads/feature",
        ],
    );
}

#[tokio::test]
async fn a_push_noticed_by_the_feed_listing_leaves_the_changed_lines_unseen() {
    let (mock, fake, repo, scratch, served) = fixture();
    let rig = Rig::start(&fake, repo, scratch).await;
    // The feed lists the pull request, which fetches its head; opening it shows the first push.
    rig.client
        .call(Request::ListPullRequests { project: rig.project })
        .await
        .unwrap();
    rig.open().await;
    rig.toggle_file("a.txt").await;
    assert_eq!(
        hunks(&rig.manifest().await, "a.txt"),
        [HunkState::Seen, HunkState::Seen]
    );

    // The author pushes; a new token for the account has the feed list again, which fetches the
    // new head (no git fetch of the remotes is made).
    push_again(&mock, &served);
    rig.client
        .call(Request::SetCredentials {
            host: "dev.azure.com/contoso".into(),
            token: Secret("pat_secret_2".into()),
            validate: false,
        })
        .await
        .unwrap();

    // The push is taken in without being asked for explicitly: the review is remapped onto it.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let m = rig.manifest().await;
        if hunks(&m, "a.txt").len() == 3 {
            assert_eq!(
                hunks(&m, "a.txt"),
                [HunkState::Unseen, HunkState::Seen, HunkState::Unseen]
            );
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the push was not taken in: {:?}",
            hunks(&m, "a.txt")
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The pull request on view is not in the platform's list, and a push to it is taken in by the
/// feed's cycle all the same: its head is fetched with the listed ones.
#[tokio::test]
async fn a_push_to_the_unlisted_pull_request_on_view_is_taken_in_by_the_feed() {
    let (mock, fake, repo, scratch, served) = fixture();
    mock.lock().unwrap().unlisted = true;
    let rig = Rig::start(&fake, repo, scratch).await;
    rig.open().await;
    rig.toggle_file("a.txt").await;

    push_again(&mock, &served);
    rig.client
        .call(Request::SetCredentials {
            host: "dev.azure.com/contoso".into(),
            token: Secret("pat_secret_2".into()),
            validate: false,
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let m = rig.manifest().await;
        if hunks(&m, "a.txt").len() == 3 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the push was not taken in: {:?}",
            hunks(&m, "a.txt")
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The whole reported path, without asking for anything: an Azure DevOps pull request from a branch
/// in the same repository, a review open in the review view on All changes with its files seen, a
/// normal push, and Vet left to take it in by its fetch interval. The new version and revision must
/// settle, the window must be told, and the changed lines must not be seen any more.
#[tokio::test]
async fn a_normal_push_taken_in_by_the_fetch_interval_settles_and_tells_the_window() {
    let (mock, fake, repo, _scratch, served) = fixture();
    let data = tempfile::tempdir().unwrap();
    let (_backend, mut client) = backend_fetching_every(data.path(), &fake, Duration::from_millis(200)).await;
    let project = client.open(repo.path()).await;
    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::PullRequest {
                remote: remote().into(),
                number: 7,
            },
        })
        .await
        .unwrap();
    let revision = manifest_of(&client, project).await.revision;
    for path in ["a.txt", "b.txt"] {
        client
            .call(Request::ToggleFileSeen {
                view: vet_protocol::View::Changeset,
                project,
                revision,
                path: path.into(),
            })
            .await
            .unwrap();
    }
    assert_eq!(
        hunks(&manifest_of(&client, project).await, "a.txt"),
        [HunkState::Seen, HunkState::Seen]
    );

    push_again(&mock, &served);

    // Nothing is asked for: the fetch interval brings the push in. The window hears that a new
    // revision settled, which is what makes it read the manifest again.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut settled = None;
    while tokio::time::Instant::now() < deadline {
        while let Ok(message) = client.inbox.try_recv() {
            if let vet_protocol::ServerMessage::Event {
                event: vet_protocol::Event::SnapshotMoved { revision, .. },
            } = message
            {
                settled = Some(revision);
            }
        }
        if settled.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(settled.is_some(), "the window was never told that a revision settled");

    let m = manifest_of(&client, project).await;
    assert_eq!(
        hunks(&m, "a.txt"),
        [HunkState::Unseen, HunkState::Seen, HunkState::Unseen],
        "the changed line is not seen"
    );
    assert_eq!(hunks(&m, "b.txt"), [HunkState::Seen], "an untouched file stays seen");
    assert!(
        m.revision > revision,
        "the revision did not move: {} -> {}",
        revision,
        m.revision
    );
}

/// The same flow on GitHub, against the fake the prefetch tests use: an ordinary push to
/// `refs/pull/7/head` must leave the line it changed unseen too.
#[tokio::test]
async fn a_push_to_a_github_pull_request_leaves_the_changed_lines_unseen() {
    let (mock, fake, repo, scratch) = crate::prefetch::fixture();
    let (_backend, client) = crate::prefetch::backend(repo.path(), &fake.base()).await;
    let project = client.open(repo.path()).await;
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
    let revision = manifest_of(&client, project).await.revision;
    client
        .call(Request::ToggleFileSeen {
            view: vet_protocol::View::Changeset,
            project,
            revision,
            path: "a.txt".into(),
        })
        .await
        .unwrap();
    assert_eq!(hunks(&manifest_of(&client, project).await, "a.txt"), [HunkState::Seen]);

    // The author pushes a new head that changes the line the pull request added.
    let work = scratch.path().join("work");
    write(&work, "a.txt", "base\npushed\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "a push"]);
    let bare = crate::prefetch::served_repo(scratch.path());
    git(
        &work,
        &["push", "-q", bare.to_str().unwrap(), "feature:refs/pull/7/head"],
    );
    let head = git(&work, &["rev-parse", "HEAD"]);
    for pr in &mut mock.lock().unwrap().prs {
        if pr.number == 7 {
            pr.head = head.clone();
        }
    }
    client.call(Request::Fetch { project }).await.unwrap();

    let m = manifest_of(&client, project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Unseen], "the pushed line is not seen");
    assert!(
        m.revision > revision,
        "the revision did not move: {} -> {}",
        revision,
        m.revision
    );
}

/// Like `served`, but the source branch carries a Rust file, so a language server starts for the
/// pull request on view.
fn served_rust(dir: &Path) -> (Served, PathBuf) {
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    init(&work);
    write(
        &work,
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&work, "src/lib.rs", "pub fn f() -> u32 {\n    1\n}\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "base"]);
    let base = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["checkout", "-q", "-b", "feature"]);
    write(&work, "src/lib.rs", "pub fn f() -> u32 {\n    2\n}\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "first push"]);
    let head1 = git(&work, &["rev-parse", "HEAD"]);
    let bare = dir.join("dev.azure.com/contoso/Shop/_git/api");
    std::fs::create_dir_all(&bare).unwrap();
    git(dir, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    git(&bare, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    git(
        &work,
        &[
            "push",
            "-q",
            bare.to_str().unwrap(),
            "main",
            "feature:refs/heads/feature",
        ],
    );
    write(&work, "src/lib.rs", "pub fn f() -> u32 {\n    3\n}\n");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "second push"]);
    let head2 = git(&work, &["rev-parse", "HEAD"]);
    let wrapper = tripwire(dir);
    (
        Served {
            base,
            head1,
            head2,
            work,
            bare,
        },
        wrapper,
    )
}

/// §6.5: a push voids "Trust this PR". The server that ran under the trusted policy must stop, and
/// the pushed head must be served only by servers that may not execute project code.
#[tokio::test]
#[ignore = "needs python3"]
async fn a_push_that_voids_trust_stops_the_executing_server() {
    let scratch = tempfile::tempdir().unwrap();
    let (served, wrapper) = served_rust(scratch.path());
    let mock = Arc::new(Mutex::new(Mock {
        head: served.head1.clone(),
        base: served.base.clone(),
        fork: false,
        iterations: vec![(1, "create", served.head1.clone())],
        unlisted: false,
    }));
    let fake = azure_devops(mock.clone(), 7);
    let repo = project(&wrapper);
    let data = tempfile::tempdir().unwrap();
    let (_backend, client) = backend(data.path(), &fake).await;
    let project = client.open(repo.path()).await;

    // The rust server is the fake one, which names its process in a hover.
    let tools = tempfile::tempdir().unwrap();
    let script = crate::restart::fake_rust_server(tools.path());
    client
        .call(Request::SetServerPath {
            project,
            language: "rust".into(),
            path: Some(script.to_str().unwrap().into()),
        })
        .await
        .unwrap();
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::PullRequest {
                remote: remote().into(),
                number: 7,
            },
        })
        .await
        .unwrap();
    client.call(Request::TrustPullRequest { project }).await.unwrap();

    // The trusted pull request runs the rust server.
    let revision = manifest_of(&client, project).await.revision;
    let (text, unavailable) = client
        .hover(project, revision, Side::Target, "src/lib.rs", (0, 7))
        .await;
    assert!(unavailable.is_none(), "{unavailable:?}");
    let pid: u32 = text
        .expect("no answer from the trusted server")
        .rsplit(' ')
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    // The author's push voids the trust.
    push_again(&mock, &served);
    client.call(Request::Fetch { project }).await.unwrap();

    // The server that ran under the withdrawn trust is stopped.
    assert!(
        crate::restart::gone(pid).await,
        "the server of the voided trust is still running"
    );

    // The pushed head is served only by servers that may not execute project code: the rust
    // server, which does, is not started for it.
    let revision = manifest_of(&client, project).await.revision;
    let (text, unavailable) = client
        .hover(project, revision, Side::Target, "src/lib.rs", (0, 7))
        .await;
    assert_eq!(text, None);
    assert_eq!(unavailable.map(|why| why.kind), Some(UnavailableKind::AnalysisLimited));
}
