//! "Remove project" (B37) through the protocol: everything held for a repository root goes,
//! whether or not its folder is still there, and a project that is open is closed first.

use std::path::Path;

use crate::common::{Client, answer_for, git, handler_over_store, hold, init, repo_with, review, snapshot_of, write};
use vet_app::changeset::Source;
use vet_app::ports::{ChangesetStore, LiveStore, RecentStore, ReviewStore, ReviewWrites, Store};
use vet_core::content::{BlobId, Hash};
use vet_infra::clock::unix_now;
use vet_infra::semantic_cache::SqliteSemanticCache;
use vet_infra::storage::layout::{ROOT, SOURCES, project_dir};
use vet_protocol::{Event, HunkState, Request, Response, ServerMessage, View};
use vet_semantic::ports::{BackendId, ServerIdentity};
use vet_semantic::reuse::{Key, SemanticCache, Value};

const GONE: &str = "/gone/repo";
const OTHER: &str = "/other/repo";

fn heard(client: &mut Client) -> Vec<Event> {
    std::iter::from_fn(|| client.inbox.try_recv().ok())
        .filter_map(|m| match m {
            ServerMessage::Event { event } => Some(event),
            _ => None,
        })
        .collect()
}

/// Opens the repository at `repo`; the project and the root it answers with.
async fn open(client: &Client, repo: &Path) -> (u32, String) {
    let repo = repo.to_str().unwrap().to_owned();
    match client.call(Request::OpenProject { repo }).await.unwrap() {
        Response::Project { project, root } => (project, root),
        other => panic!("{other:?}"),
    }
}

async fn manifest(client: &Client, project: u32) -> Result<Response, vet_protocol::Failure> {
    let view = View::Changeset;
    client.call(Request::Manifest { project, view }).await
}

/// A project directory the way a project's files are kept: its root, and a tree under it.
fn keep_files(data: &Path, root: &str) -> std::path::PathBuf {
    let dir = project_dir(data, root);
    std::fs::create_dir_all(dir.join(SOURCES).join("a1b2-fetch").join("tree")).unwrap();
    std::fs::write(dir.join(ROOT), root).unwrap();
    std::fs::write(dir.join(SOURCES).join("a1b2-fetch").join("tree").join("file"), "x").unwrap();
    dir
}

#[tokio::test]
async fn a_project_whose_folder_is_gone_is_removed_with_what_is_held_for_it_and_no_other_project() {
    let (handler, store, data) = handler_over_store().await;
    let mut client = Client::new(&handler);
    client.call(Request::Subscribe).await.unwrap();
    let now = unix_now();
    let mine = hold(&store, &[("a.txt", b"only the removed project has this")], now).await;
    let theirs = hold(&store, &[("b.txt", b"the other project's content")], now).await;
    review(&store, GONE, Source::WorkingTree, mine.id).await;
    review(&store, OTHER, Source::WorkingTree, theirs.id).await;
    store.writes().set_project_trusted(GONE.into(), true).await.unwrap();
    for root in [GONE, OTHER] {
        client
            .call(Request::SetPreference {
                scope: Some(root.into()),
                key: "tabs".into(),
                value: Some("[]".into()),
            })
            .await
            .unwrap();
        client
            .call(Request::RecordRecentProject { path: root.into() })
            .await
            .unwrap();
    }
    let dir = keep_files(data.path(), GONE);
    let other_dir = keep_files(data.path(), OTHER);
    let cache = SqliteSemanticCache::open(data.path(), 256 << 20).await.unwrap();
    let text = Key::Text {
        server: ServerIdentity {
            backend: BackendId("rust".into()),
            fingerprint: Hash([1; 32]),
            label: "rust-analyzer".into(),
        },
        extension: b"rs".to_vec(),
        blob: BlobId::of(b"fn main() {}"),
    };
    for key in [answer_for(mine.id), answer_for(theirs.id)] {
        cache.put(&key, &Value::Hover(Some("an answer".into())));
    }
    cache.put(&text, &Value::Symbols(vec![]));
    cache.flush().await;
    heard(&mut client);

    let removed = client.call(Request::RemoveProject { root: GONE.into() }).await;

    let Ok(Response::ProjectRemoved { freed }) = removed else {
        panic!("{removed:?}");
    };
    assert!(freed > 0, "what the project took is reported");
    assert_eq!(store.changeset(GONE.into(), Source::WorkingTree).await.unwrap(), None);
    assert!(!store.project_trusted(GONE.into()).await.unwrap());
    assert!(
        store
            .changeset(OTHER.into(), Source::WorkingTree)
            .await
            .unwrap()
            .is_some()
    );
    let recent: Vec<_> = store
        .recent_projects()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.path)
        .collect();
    assert_eq!(recent, [OTHER]);
    assert!(!dir.exists(), "the project's directory is gone");
    assert!(other_dir.exists());
    assert!(
        cache.get(&answer_for(mine.id)).await.is_none(),
        "its snapshot's answers"
    );
    assert!(
        cache.get(&answer_for(theirs.id)).await.is_some(),
        "another project's answers"
    );
    assert!(cache.get(&text).await.is_some(), "answers about a text");
    assert!(
        store.snapshot(mine.id).await.unwrap().is_some(),
        "a snapshot stored a moment ago is kept for the grace period"
    );
    let events = heard(&mut client);
    for expected in [
        Event::ProjectRemoved { root: GONE.into() },
        Event::RecentProjectsChanged,
        Event::StorageChanged,
    ] {
        assert!(events.contains(&expected), "{expected:?} not in {events:?}");
    }
}

#[tokio::test]
async fn removing_an_open_project_closes_it_for_every_connection_and_reopening_starts_from_nothing() {
    let repo = repo_with(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let (handler, _store, data) = handler_over_store().await;
    let (mut a, mut b) = (Client::new(&handler), Client::new(&handler));
    for client in [&a, &b] {
        client.call(Request::Subscribe).await.unwrap();
    }
    let (project, root) = open(&a, repo.path()).await;
    open(&b, repo.path()).await;
    a.call(Request::SetProjectTrusted { project, trusted: true })
        .await
        .unwrap();
    let revision = a.revision(project).await;
    a.call(Request::ToggleHunk {
        project,
        view: View::Changeset,
        revision,
        path: "a.txt".into(),
        hunk: 0,
    })
    .await
    .unwrap();
    let Ok(Response::Manifest(before)) = manifest(&a, project).await else {
        panic!("no manifest");
    };
    assert!(before.trust.project);
    assert_eq!(before.files[0].review.hunks, [HunkState::Seen]);
    let dir = keep_files(data.path(), &root);
    heard(&mut a);
    heard(&mut b);

    a.call(Request::RemoveProject { root: root.clone() }).await.unwrap();

    for client in [&mut a, &mut b] {
        let events = heard(client);
        assert!(
            events.contains(&Event::ProjectRemoved { root: root.clone() }),
            "{events:?}"
        );
    }
    assert!(
        manifest(&b, project).await.is_err(),
        "the project is closed for everyone"
    );
    assert!(!dir.exists());
    let (again, _) = open(&a, repo.path()).await;
    assert_ne!(again, project);
    let Ok(Response::Manifest(after)) = manifest(&a, again).await else {
        panic!("no manifest");
    };
    assert!(!after.trust.project, "the trust is forgotten");
    assert_eq!(after.files[0].review.hunks, [HunkState::Unseen], "the review is gone");
}

#[tokio::test]
async fn the_pull_request_refs_stay_while_another_project_shares_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    let (main, linked) = (canonical.join("main"), canonical.join("linked"));
    std::fs::create_dir_all(&main).unwrap();
    init(&main);
    write(&main, "a.txt", "one\n");
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-q", "-m", "base"]);
    git(
        &main,
        &["worktree", "add", "-q", "-b", "side", linked.to_str().unwrap()],
    );
    let (handler, store, _data) = handler_over_store().await;
    let client = Client::new(&handler);
    let snapshot = snapshot_of(&[("a.txt", b"one\n")]);
    let (main_root, linked_root) = (main.to_str().unwrap(), linked.to_str().unwrap());
    let (first, _) = review(&store, main_root, Source::WorkingTree, snapshot.id).await;
    let (second, _) = review(&store, linked_root, Source::WorkingTree, snapshot.id).await;
    for name in [
        format!("refs/vet/changesets/{}/1/tip", first.0),
        format!("refs/vet/changesets/{}/1/tip", second.0),
        "refs/vet/pr/7/head".to_owned(),
    ] {
        git(&main, &["update-ref", &name, "HEAD"]);
    }
    let vet_refs = || {
        git(&main, &["for-each-ref", "--format=%(refname)", "refs/vet/"])
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    client
        .call(Request::RemoveProject { root: main_root.into() })
        .await
        .unwrap();
    assert_eq!(
        vet_refs(),
        [
            format!("refs/vet/changesets/{}/1/tip", second.0),
            "refs/vet/pr/7/head".to_owned()
        ],
        "the removed project's changeset refs went, the linked worktree's and the pull requests' stay"
    );

    client
        .call(Request::RemoveProject {
            root: linked_root.into(),
        })
        .await
        .unwrap();
    assert_eq!(
        vet_refs(),
        Vec::<String>::new(),
        "no project shares the repository any more"
    );
}
