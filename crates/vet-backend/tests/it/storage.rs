//! What the projects take on disk (B37), asked through the protocol.

use std::path::Path;

use crate::common::{Client, answer_for, handler_over_store, hold, noise, repo_with, review};
use vet_app::changeset::Source;
use vet_app::ports::{LiveStore, ReviewWrites};
use vet_core::content::SnapshotId;
use vet_core::review::{CommentTarget, Message, Outgoing, PendingVote, RemoteComment, ThreadStatus, Verdict};
use vet_infra::materialize::projects::ProjectTrees;
use vet_infra::semantic_cache::SqliteSemanticCache;
use vet_protocol::{ProjectStorage, Request, Response};
use vet_semantic::policy::RestoreKind;
use vet_semantic::reuse::{SemanticCache, Value};

async fn project(client: &Client, root: &str) -> ProjectStorage {
    match client.call(Request::ProjectStorage { root: root.into() }).await {
        Ok(Response::ProjectStorage { project }) => project,
        other => panic!("{other:?}"),
    }
}

/// Keeps a tree of `bytes` bytes under the project at `root`.
async fn keep_tree(data: &Path, root: &str, bytes: usize) {
    ProjectTrees::for_test(data.to_owned())
        .of(root)
        .await
        .acquire(
            SnapshotId(vet_core::content::Hash([bytes as u8; 32])),
            RestoreKind::Fetch,
            |dest| async move { std::fs::write(dest.join("file"), vec![b'x'; bytes]) },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn every_project_is_listed_off_the_recent_list_and_without_its_folder() {
    let repo = repo_with(&[("a.txt", "one\n")], &[]);
    let (handler, store, data) = handler_over_store().await;
    let client = Client::new(&handler);
    let root = match client
        .call(Request::OpenProject {
            repo: repo.path().to_str().unwrap().into(),
        })
        .await
        .unwrap()
    {
        Response::Project { root, .. } => root,
        other => panic!("{other:?}"),
    };
    client
        .call(Request::RecordRecentProject { path: root.clone() })
        .await
        .unwrap();
    let snapshot = hold(&store, &[("a.txt", b"x")], 0).await;
    review(&store, "/gone/stored", Source::WorkingTree, snapshot.id).await;
    keep_tree(data.path(), "/gone/files", 10).await;

    let Ok(Response::ProjectsStorage { projects, app }) = client.call(Request::ProjectsStorage).await else {
        panic!("no storage");
    };

    let listed: Vec<_> = projects
        .iter()
        .map(|p| (p.root.as_str(), p.folder_exists, p.open, p.recent))
        .collect();
    assert_eq!(
        listed,
        [
            ("/gone/files", false, false, false),
            ("/gone/stored", false, false, false),
            (root.as_str(), true, true, true),
        ]
    );
    assert!(projects[2].last_opened.is_some());
    assert!(app.databases > 0);
    let unknown = project(&client, "/never/known").await;
    assert_eq!(
        (unknown.folder_exists, unknown.review_data, unknown.source_trees),
        (false, 0, 0)
    );
}

#[tokio::test]
async fn a_project_counts_only_the_content_trees_and_answers_that_are_its_own() {
    let (handler, store, data) = handler_over_store().await;
    let client = Client::new(&handler);
    let (mine, shared, theirs) = (noise(1000, 1), noise(3000, 2), noise(2000, 3));
    let a = hold(&store, &[("own", &mine), ("shared", &shared)], 0).await;
    let b = hold(&store, &[("own", &theirs), ("shared", &shared)], 0).await;
    review(&store, "/work/a", Source::WorkingTree, a.id).await;
    review(&store, "/work/b", Source::WorkingTree, b.id).await;
    keep_tree(data.path(), "/work/a", 700).await;
    keep_tree(data.path(), "/work/b", 5000).await;
    let cache = SqliteSemanticCache::open(data.path(), 256 << 20).await.unwrap();
    for (snapshot, text) in [
        (a.id, "short"),
        (b.id, "a considerably longer answer than the other one"),
    ] {
        cache.put(&answer_for(snapshot), &Value::Hover(Some(text.into())));
    }
    cache.flush().await;

    let (a, b) = (project(&client, "/work/a").await, project(&client, "/work/b").await);

    assert_eq!(
        (a.review_data, b.review_data),
        (1000, 2000),
        "not the content both reach"
    );
    assert_eq!(
        (a.source_trees, b.source_trees),
        (700, 5000),
        "each its own directory's trees"
    );
    assert_eq!((a.source_trees_in_use, b.source_trees_in_use), (0, 0));
    assert!(
        a.semantic_cache > 0 && b.semantic_cache > a.semantic_cache,
        "{a:?} {b:?}"
    );
}

#[tokio::test]
async fn what_removing_a_project_would_lose_counts_only_what_only_vet_has() {
    let (handler, store, _data) = handler_over_store().await;
    let client = Client::new(&handler);
    let root = "/work/loss";
    let snapshot = hold(&store, &[("a.txt", b"x")], 0).await;
    let (changeset, open) = review(&store, root, Source::WorkingTree, snapshot.id).await;
    let writes = store.writes();
    let local = |body: &str| Message::written("me".into(), body.into(), 0);
    let posted = Message {
        mine: false,
        remote: Some(RemoteComment {
            id: "C1".into(),
            synced: "posted".into(),
            conflict: None,
        }),
        ..local("posted")
    };
    for message in [local("one"), local("two"), posted] {
        writes
            .add_thread(
                changeset,
                Some(open),
                CommentTarget::Changeset,
                message,
                None,
                ThreadStatus::Open,
            )
            .await
            .unwrap();
    }
    writes
        .set_pending_vote(
            root.into(),
            Source::WorkingTree,
            Some(PendingVote {
                verdict: Verdict::Approve,
                outgoing: Outgoing::Posting { since: 0 },
            }),
        )
        .await
        .unwrap();
    writes.set_project_trusted(root.into(), true).await.unwrap();

    let found = project(&client, root).await;

    assert_eq!(
        (
            found.open_reviews,
            found.unposted_comments,
            found.pending_votes,
            found.trusted
        ),
        (1, 2, 1, true)
    );
}
