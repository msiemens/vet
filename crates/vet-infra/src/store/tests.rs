use std::collections::BTreeMap;
use std::time::Duration;

use sea_orm::PaginatorTrait;
use sea_orm::sqlx;
use std::collections::HashSet;
use tempfile::TempDir;
use vet_app::changeset::{BaseChoice, ChangedFile, CommitId, Lens, RemoteKey, Snapshots, VersionChange, VersionOrigin};
use vet_app::gc::{Retention, collect, open_bases};
use vet_app::platform::{Discussion, PullRequestState, ReviewDecision};
use vet_app::ports::{GcReport, ReviewWrites, Store};
use vet_app::review::{NonTextAck, SpanOrigin};
use vet_core::content::{
    BlobId, Changeset, EntryKind, FileEntry, Hash, LineDelta, LineRange, NonTextKind, RepoPath, Snapshot, SnapshotId,
    SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::prepared::PreparedFile;
use vet_core::remap::{RemapMode, remap_span};
use vet_core::review::{
    Anchor, CommentTarget, EditConflict, FileRemapInput, Message, Outgoing, PendingVote, PlatformThread, RemoteComment,
    ReviewedSpan, Side, SpanState, ThreadStatus, Verdict,
};

use super::*;

const DAY: i64 = 24 * 3600;
/// A fixed "now", so retention and grace are deterministic.
const T0: i64 = 1_700_000_000;

fn h(n: u8) -> Hash {
    Hash([n; 32])
}

async fn open(dir: &TempDir) -> SqliteStore {
    SqliteStore::for_test(dir.path()).await.unwrap()
}

/// Collects with no grace period, so content is eligible as soon as it is unreferenced.
async fn gc(store: &SqliteStore, now: i64) -> GcReport {
    let retention = Retention {
        grace: Duration::ZERO,
        ..Default::default()
    };
    collect(store, now, &retention).await.unwrap()
}

fn file_entry(blob: u8) -> FileEntry {
    FileEntry {
        blob: BlobId::of(&[blob]),
        mode: 0o100644,
        kind: EntryKind::File,
        unfiltered: false,
    }
}

fn snapshot(files: &[(&str, u8)]) -> Snapshot {
    let entries = files
        .iter()
        .map(|(p, b)| (RepoPath(p.as_bytes().to_vec()), file_entry(*b)))
        .collect::<BTreeMap<_, _>>();
    Snapshot {
        id: merkle::snapshot_id(&entries),
        entries,
    }
}

fn anchor(side: Side, snapshot: u8) -> Anchor {
    Anchor {
        side,
        snapshot: SnapshotId(h(snapshot)),
        file: RepoPath(b"src/lib.rs".to_vec()),
        range: LineRange { start: 3, end: 9 },
        content_hash_raw: h(10),
        content_hash_norm: h(11),
        context_hashes: vec![h(12), h(13)],
    }
}

fn message(body: &str) -> Message {
    Message::written("me".into(), body.into(), 1_700_000_000)
}

async fn add_thread(
    store: &SqliteStore,
    changeset: ChangesetId,
    review: Option<ReviewId>,
    target: CommentTarget,
    opener: Message,
) -> ThreadId {
    store
        .writes()
        .add_thread(changeset, review, target, opener, None, ThreadStatus::Open)
        .await
        .unwrap()
        .0
}

fn commit(id: &str) -> CommitId {
    CommitId(id.into())
}

fn snapshots(base: u8, target: u8) -> Snapshots {
    Snapshots {
        base: SnapshotId(h(base)),
        target: SnapshotId(h(target)),
    }
}

fn version(n: u32, base: SnapshotId, target: SnapshotId) -> Version {
    Version {
        n,
        snapshots: Some(Snapshots { base, target }),
        base_commit: commit("abc123"),
        tip: commit("abc123"),
        at: i64::from(n) * 10,
        origin: VersionOrigin::Settled,
        change: VersionChange::default(),
    }
}

/// Opens a review of the working tree of `/repo`, whose version 1 is `base` and `target`.
async fn review(store: &SqliteStore, base: SnapshotId, target: SnapshotId) -> (ChangesetId, ReviewId) {
    store
        .writes()
        .start_review(
            "/repo".into(),
            Source::WorkingTree,
            BaseChoice::Commit { id: commit("abc123") },
            vec![version(1, base, target)],
            T0,
        )
        .await
        .unwrap()
}

fn sample_filter() -> ReviewFilter {
    ReviewFilter {
        roots: vec![RepoPath(b"crates".to_vec())],
        include: vec!["*.rs".into()],
        exclude: vec!["*.lock".into(), "gen/**".into()],
        hide_binary: true,
        review_generated: true,
    }
}

#[tokio::test]
async fn the_filter_is_stored_per_repository_and_replaced_as_a_whole() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    assert_eq!(store.filter("/a".into()).await.unwrap(), ReviewFilter::default());

    store.writes().set_filter("/a".into(), sample_filter()).await.unwrap();
    let other = ReviewFilter {
        exclude: vec!["b".into()],
        ..Default::default()
    };
    store.writes().set_filter("/b".into(), other.clone()).await.unwrap();
    let narrower = ReviewFilter {
        exclude: vec!["x".into()],
        ..Default::default()
    };
    store.writes().set_filter("/a".into(), narrower.clone()).await.unwrap();
    store.writes().set_filter("/b".into(), other.clone()).await.unwrap();

    drop(store);
    let store = open(&dir).await;
    assert_eq!(store.filter("/a".into()).await.unwrap(), narrower);
    assert_eq!(store.filter("/b".into()).await.unwrap(), other);
    assert_eq!(store.filter("/c".into()).await.unwrap(), ReviewFilter::default());

    store.writes().set_filter("/a".into(), sample_filter()).await.unwrap();
    assert_eq!(store.filter("/a".into()).await.unwrap(), sample_filter());
}

#[tokio::test]
async fn a_database_of_another_version_is_recreated_and_one_of_this_version_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let db = ReviewWriter::connect(dir.path()).await.unwrap().db;
    // What an earlier version left behind: its migrations and a differently shaped session table.
    for sql in [
        "CREATE TABLE seaql_migrations (version VARCHAR NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)",
        "INSERT INTO seaql_migrations VALUES ('schema_0000000000000000', 1)",
        "CREATE TABLE session (id INTEGER PRIMARY KEY, repo TEXT)",
        "INSERT INTO session VALUES (1, '/old')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    db.close().await.unwrap();

    let store = open(&dir).await;
    assert_eq!(store.changeset("/old".into(), Source::WorkingTree).await.unwrap(), None);
    let (changeset, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    drop(store);

    let store = open(&dir).await;
    assert_eq!(
        store.changeset("/repo".into(), Source::WorkingTree).await.unwrap(),
        Some(changeset)
    );
    assert_eq!(store.open_review(changeset).await.unwrap(), Some(id));
}

#[tokio::test]
async fn a_database_of_a_newer_version_is_refused_and_left_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    drop(store);
    let db = ReviewWriter::connect(dir.path()).await.unwrap().db;
    db.execute_unprepared("INSERT INTO seaql_migrations VALUES ('m29990101_000001_from_the_future', 1)")
        .await
        .unwrap();
    db.close().await.unwrap();

    let Err(refused) = SqliteStore::for_test(dir.path()).await else {
        panic!("a database of a newer version opened");
    };
    assert!(refused.0.contains("newer version"), "{}", refused.0);

    let db = ReviewWriter::connect(dir.path()).await.unwrap().db;
    db.execute_unprepared("DELETE FROM seaql_migrations WHERE version LIKE '%future'")
        .await
        .unwrap();
    db.close().await.unwrap();
    let store = open(&dir).await;
    assert_eq!(
        store.changeset("/repo".into(), Source::WorkingTree).await.unwrap(),
        Some(changeset)
    );
}

#[tokio::test]
async fn a_changeset_is_found_by_its_repository_and_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let pull_request = |host: &str, path: &str, number| Source::PullRequest {
        remote: RemoteKey {
            host: host.into(),
            path: path.into(),
        },
        number,
    };
    let sources = [
        ("/repo", Source::WorkingTree),
        ("/other", Source::WorkingTree),
        ("/repo", Source::Branch { name: "main".into() }),
        ("/repo", Source::Branch { name: "".into() }),
        ("/repo", pull_request("github.com", "o/r", 1)),
        ("/repo", pull_request("github.com", "o/r", 2)),
        ("/repo", pull_request("github.com", "o/x", 1)),
        ("/repo", pull_request("example.com", "o/r", 1)),
    ];
    let mut ids = Vec::new();
    for (repo, source) in &sources {
        assert_eq!(store.changeset((*repo).into(), source.clone()).await.unwrap(), None);
        let (id, _) = store
            .writes()
            .start_review(
                (*repo).into(),
                source.clone(),
                BaseChoice::Commit { id: commit("a") },
                vec![version(1, SnapshotId(h(1)), SnapshotId(h(2)))],
                T0,
            )
            .await
            .unwrap();
        ids.push(id);
    }
    drop(store);

    let store = open(&dir).await;
    for ((repo, source), id) in sources.iter().zip(&ids) {
        assert_eq!(
            store.changeset((*repo).into(), source.clone()).await.unwrap(),
            Some(*id)
        );
    }
    let mut distinct = ids.clone();
    distinct.sort_by_key(|id| id.0);
    distinct.dedup();
    assert_eq!(distinct.len(), sources.len(), "every source is a changeset of its own");
}

#[tokio::test]
async fn the_lens_is_remembered_per_changeset_before_and_after_a_review_of_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let feature = || Source::Branch { name: "feature".into() };
    assert_eq!(store.lens("/repo".into(), feature()).await.unwrap(), None);

    store
        .writes()
        .set_lens("/repo".into(), feature(), Lens::PerCommit)
        .await
        .unwrap();
    store
        .writes()
        .start_review(
            "/repo".into(),
            feature(),
            BaseChoice::Commit { id: commit("a") },
            vec![version(1, SnapshotId(h(1)), SnapshotId(h(2)))],
            T0,
        )
        .await
        .unwrap();
    drop(store);

    let store = open(&dir).await;
    assert_eq!(
        store.lens("/repo".into(), feature()).await.unwrap(),
        Some(Lens::PerCommit)
    );
    assert_eq!(
        store
            .lens("/repo".into(), Source::Branch { name: "other".into() })
            .await
            .unwrap(),
        None
    );
    store
        .writes()
        .set_lens("/repo".into(), feature(), Lens::Squash)
        .await
        .unwrap();
    assert_eq!(store.lens("/repo".into(), feature()).await.unwrap(), Some(Lens::Squash));
}

#[tokio::test]
async fn the_pinned_base_of_a_workspace_survives_a_restart_and_a_review_starting_and_is_cleared_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    assert_eq!(
        store.pinned_base("/repo".into(), Source::WorkingTree).await.unwrap(),
        None
    );
    store
        .writes()
        .set_pinned_base("/repo".into(), Source::WorkingTree, Some(commit("abc123")))
        .await
        .unwrap();
    review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    drop(store);

    let store = open(&dir).await;
    let pinned = |repo: &str| store.pinned_base(repo.into(), Source::WorkingTree);
    assert_eq!(pinned("/repo").await.unwrap(), Some(commit("abc123")));
    assert_eq!(pinned("/other").await.unwrap(), None);
    store
        .writes()
        .set_pinned_base("/repo".into(), Source::WorkingTree, None)
        .await
        .unwrap();
    assert_eq!(pinned("/repo").await.unwrap(), None);
}

#[tokio::test]
async fn a_pull_requests_title_is_kept_with_its_changeset_across_a_review_starting_and_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let pull_request = |number| Source::PullRequest {
        remote: RemoteKey {
            host: "github.com".into(),
            path: "acme/app".into(),
        },
        number,
    };
    let head = |title: &str, draft| PullRequestHead {
        title: title.into(),
        draft,
        fork: false,
        target_branch: "release/2.1".into(),
    };
    assert_eq!(
        store.pull_request_head("/repo".into(), pull_request(7)).await.unwrap(),
        None
    );

    store
        .writes()
        .set_pull_request_head("/repo".into(), pull_request(7), head("Add caching", true))
        .await
        .unwrap();
    store
        .writes()
        .set_lens("/repo".into(), pull_request(8), Lens::PerCommit)
        .await
        .unwrap();
    store
        .writes()
        .start_review(
            "/repo".into(),
            pull_request(7),
            BaseChoice::Commit { id: commit("a") },
            vec![version(1, SnapshotId(h(1)), SnapshotId(h(2)))],
            T0,
        )
        .await
        .unwrap();
    drop(store);

    let store = open(&dir).await;
    assert_eq!(
        store.pull_request_head("/repo".into(), pull_request(7)).await.unwrap(),
        Some(head("Add caching", true))
    );
    assert_eq!(
        store.pull_request_head("/other".into(), pull_request(7)).await.unwrap(),
        None
    );
    assert_eq!(
        store.pull_request_head("/repo".into(), pull_request(8)).await.unwrap(),
        None,
        "stored for its lens, it was never named"
    );
    store
        .writes()
        .set_pull_request_head("/repo".into(), pull_request(7), head("Add caching to the API", false))
        .await
        .unwrap();
    assert_eq!(
        store.pull_request_head("/repo".into(), pull_request(7)).await.unwrap(),
        Some(head("Add caching to the API", false))
    );
}

#[tokio::test]
async fn trust_is_kept_per_project_and_per_pull_request_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let pull_request = |number| Source::PullRequest {
        remote: RemoteKey {
            host: "github.com".into(),
            path: "acme/app".into(),
        },
        number,
    };
    let named = |fork| PullRequestHead {
        title: "Add caching".into(),
        draft: false,
        fork,
        target_branch: "main".into(),
    };
    let writes = store.writes();
    assert!(!store.project_trusted("/a".into()).await.unwrap());
    writes.set_project_trusted("/a".into(), true).await.unwrap();
    assert!(store.project_trusted("/a".into()).await.unwrap());
    assert!(!store.project_trusted("/b".into()).await.unwrap());

    // The trust of a pull request is unknown until the platform has named it.
    assert_eq!(
        store.pull_request_trust("/a".into(), pull_request(7)).await.unwrap(),
        None
    );
    writes
        .set_pull_request_head("/a".into(), pull_request(7), named(true))
        .await
        .unwrap();
    writes
        .set_pull_request_trusted_head("/a".into(), pull_request(7), Some(commit("c1")))
        .await
        .unwrap();
    drop(writes);
    drop(store);

    let store = open(&dir).await;
    assert!(store.project_trusted("/a".into()).await.unwrap());
    let trust = |number| store.pull_request_trust("/a".into(), pull_request(number));
    assert_eq!(
        trust(7).await.unwrap(),
        Some(PullRequestTrust {
            fork: true,
            trusted_head: Some(commit("c1"))
        })
    );
    assert_eq!(trust(8).await.unwrap(), None);

    // Naming the pull request again does not undo what was trusted; untrusting does.
    store
        .writes()
        .set_pull_request_head("/a".into(), pull_request(7), named(false))
        .await
        .unwrap();
    assert_eq!(
        trust(7).await.unwrap(),
        Some(PullRequestTrust {
            fork: false,
            trusted_head: Some(commit("c1"))
        })
    );
    store
        .writes()
        .set_pull_request_trusted_head("/a".into(), pull_request(7), None)
        .await
        .unwrap();
    store.writes().set_project_trusted("/a".into(), false).await.unwrap();
    assert_eq!(trust(7).await.unwrap().unwrap().trusted_head, None);
    assert!(!store.project_trusted("/a".into()).await.unwrap());
}

#[tokio::test]
async fn a_pending_vote_is_kept_per_pull_request_in_its_state_across_a_restart_and_a_closed_review_expiring() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let pull_request = |number| Source::PullRequest {
        remote: RemoteKey {
            host: "dev.azure.com".into(),
            path: "acme/shop/app".into(),
        },
        number,
    };
    let vote = |verdict: Verdict, outgoing| PendingVote { verdict, outgoing };
    let failed = Outgoing::Failed {
        error: "no access".into(),
    };
    assert_eq!(store.pending_vote("/repo".into(), pull_request(7)).await.unwrap(), None);

    let writes = store.writes();
    writes
        .set_pending_vote(
            "/repo".into(),
            pull_request(7),
            Some(vote(Verdict::Approve, Outgoing::Draft)),
        )
        .await
        .unwrap();
    writes
        .set_pending_vote(
            "/repo".into(),
            pull_request(7),
            Some(vote(Verdict::Reject, failed.clone())),
        )
        .await
        .unwrap();
    drop(writes);
    drop(store);

    let store = open(&dir).await;
    assert_eq!(
        store.pending_vote("/repo".into(), pull_request(7)).await.unwrap(),
        Some(vote(Verdict::Reject, failed)),
        "the vote chosen last, with why it failed"
    );
    assert_eq!(store.pending_vote("/repo".into(), pull_request(8)).await.unwrap(), None);
    assert_eq!(
        store.pending_vote("/other".into(), pull_request(7)).await.unwrap(),
        None
    );

    let posting = Outgoing::Posting { since: T0 };
    let writes = store.writes();
    writes
        .set_pending_vote(
            "/repo".into(),
            pull_request(7),
            Some(vote(Verdict::Reject, posting.clone())),
        )
        .await
        .unwrap();
    assert_eq!(
        store.pending_vote("/repo".into(), pull_request(7)).await.unwrap(),
        Some(vote(Verdict::Reject, posting))
    );

    // The closed review is old enough to expire; the vote keeps its changeset.
    let (changeset, review) = writes
        .start_review(
            "/repo".into(),
            pull_request(7),
            BaseChoice::Commit { id: commit("a") },
            vec![version(1, SnapshotId(h(1)), SnapshotId(h(2)))],
            T0,
        )
        .await
        .unwrap();
    writes.close_review(review, T0, false).await.unwrap();
    gc(&store, T0 + 100 * DAY).await;
    assert_eq!(
        store.changeset("/repo".into(), pull_request(7)).await.unwrap(),
        Some(changeset)
    );
    writes
        .set_pending_vote("/repo".into(), pull_request(7), None)
        .await
        .unwrap();
    assert_eq!(store.pending_vote("/repo".into(), pull_request(7)).await.unwrap(), None);
    gc(&store, T0 + 100 * DAY).await;
    assert_eq!(store.changeset("/repo".into(), pull_request(7)).await.unwrap(), None);
}

#[tokio::test]
async fn the_base_choice_is_stored_with_the_review_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (_, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let loaded = |store: &SqliteStore| {
        let store = store.clone();
        async move { store.review(id).await.unwrap().unwrap().base }
    };
    assert_eq!(loaded(&store).await, BaseChoice::Commit { id: commit("abc123") });

    drop(store);
    let store = open(&dir).await;
    assert_eq!(loaded(&store).await, BaseChoice::Commit { id: commit("abc123") });
}

#[tokio::test]
async fn spans_and_acks_belong_to_the_review_and_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (_, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let (_, other) = store
        .writes()
        .start_review(
            "/other".into(),
            Source::WorkingTree,
            BaseChoice::Commit { id: commit("abc123") },
            vec![version(1, SnapshotId(h(1)), SnapshotId(h(2)))],
            T0,
        )
        .await
        .unwrap();

    let spans = vec![
        ReviewedSpan {
            base: Some(anchor(Side::Base, 1)),
            target: Some(anchor(Side::Target, 2)),
            state: SpanState::Seen,
        },
        ReviewedSpan {
            base: None,
            target: Some(anchor(Side::Target, 2)),
            state: SpanState::SeenAgainstOldBase,
        },
        ReviewedSpan {
            base: Some(anchor(Side::Base, 1)),
            target: None,
            state: SpanState::Flagged,
        },
    ];
    let origins = [SpanOrigin::Squash, SpanOrigin::Commit(commit("c1")), SpanOrigin::Squash];
    let added = spans.iter().cloned().zip(origins.iter().cloned()).collect();
    let ids = store.writes().edit_spans(id, vec![], added).await.unwrap();
    assert_eq!(ids.len(), 3);

    let ack = NonTextAck {
        kind: NonTextKind::Binary,
        before: Some((RepoPath(b"x".to_vec()), file_entry(5))),
        after: None,
    };
    store.writes().acknowledge(id, ack.clone()).await.unwrap();
    store.writes().acknowledge(id, ack.clone()).await.unwrap();
    // Same entries, another kind: a separate item.
    let mode = NonTextAck {
        kind: NonTextKind::Mode,
        ..ack.clone()
    };
    store.writes().acknowledge(id, mode.clone()).await.unwrap();

    let loaded = store.review(id).await.unwrap().unwrap();
    assert_eq!(loaded.spans.iter().map(|r| r.span.clone()).collect::<Vec<_>>(), spans);
    assert_eq!(
        loaded.spans.iter().map(|r| r.origin.clone()).collect::<Vec<_>>(),
        origins
    );
    assert_eq!(loaded.acks, vec![ack.clone(), mode.clone()]);
    let elsewhere = store.review(other).await.unwrap().unwrap();
    assert!(elsewhere.spans.is_empty() && elsewhere.acks.is_empty());

    store.writes().unacknowledge(id, ack).await.unwrap();
    assert_eq!(store.review(id).await.unwrap().unwrap().acks, vec![mode.clone()]);
    store.writes().unacknowledge(id, mode).await.unwrap();
    store.writes().edit_spans(id, ids, vec![]).await.unwrap();
    let loaded = store.review(id).await.unwrap().unwrap();
    assert!(loaded.spans.is_empty() && loaded.acks.is_empty());
}

#[tokio::test]
async fn threads_belong_to_the_changeset_and_outlive_its_review() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, first) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;

    let range = add_thread(
        &store,
        changeset,
        Some(first),
        CommentTarget::Range(anchor(Side::Target, 2)),
        message("why?"),
    )
    .await;
    let file = add_thread(
        &store,
        changeset,
        Some(first),
        CommentTarget::File {
            side: Side::Base,
            path: RepoPath(b"a/b".to_vec()),
        },
        message("file"),
    )
    .await;
    let overall = add_thread(&store, changeset, None, CommentTarget::Changeset, message("overall")).await;
    store.writes().add_message(range, message("because")).await.unwrap();
    store
        .writes()
        .set_thread_status(file, ThreadStatus::Resolved)
        .await
        .unwrap();

    store.writes().close_review(first, T0, true).await.unwrap();
    let (again, second) = review(&store, SnapshotId(h(1)), SnapshotId(h(3))).await;
    assert_eq!(again, changeset);
    assert_ne!(second, first);

    let threads = store.threads(changeset).await.unwrap();
    assert_eq!(
        threads.iter().map(|t| t.thread.id).collect::<Vec<_>>(),
        [range, file, overall]
    );
    assert_eq!(
        threads.iter().map(|t| t.review).collect::<Vec<_>>(),
        [Some(first), Some(first), None],
        "a thread keeps the review it was opened in"
    );
    assert_eq!(threads[0].thread.target, CommentTarget::Range(anchor(Side::Target, 2)));
    let bodies: Vec<_> = threads[0].thread.messages.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, ["why?", "because"]);
    assert_eq!(threads[1].thread.status, ThreadStatus::Resolved);
    assert_eq!(
        threads[1].thread.target,
        CommentTarget::File {
            side: Side::Base,
            path: RepoPath(b"a/b".to_vec())
        }
    );
    assert_eq!(threads[2].thread.target, CommentTarget::Changeset);

    let moved = CommentTarget::Range(Anchor {
        range: LineRange { start: 20, end: 25 },
        ..anchor(Side::Target, 2)
    });
    store.writes().set_thread_target(range, moved.clone()).await.unwrap();
    store.writes().delete_thread(file).await.unwrap();
    let threads = store.threads(changeset).await.unwrap();
    assert_eq!(
        threads.iter().map(|t| &t.thread.target).collect::<Vec<_>>(),
        vec![&moved, &CommentTarget::Changeset]
    );
}

fn posted(id: &str, synced: &str, body: &str) -> Message {
    Message {
        mine: false,
        avatar: Some("https://avatars.example/lena".into()),
        remote: Some(RemoteComment {
            id: id.into(),
            synced: synced.into(),
            conflict: None,
        }),
        ..Message::written("lena".into(), body.into(), T0)
    }
}

#[tokio::test]
async fn what_a_platform_has_of_a_thread_survives_a_restart_and_a_thread_needs_no_review() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let source = Source::PullRequest {
        remote: RemoteKey {
            host: "github.com".into(),
            path: "acme/app".into(),
        },
        number: 7,
    };
    let changeset = store
        .writes()
        .ensure_changeset("/repo".into(), source.clone())
        .await
        .unwrap();
    assert_eq!(
        store
            .writes()
            .ensure_changeset("/repo".into(), source.clone())
            .await
            .unwrap(),
        changeset
    );
    assert_eq!(
        store.open_review(changeset).await.unwrap(),
        None,
        "no review is started by it"
    );

    let platform = PlatformThread {
        remote_id: Some("T1".into()),
        resolvable: true,
        resolved_by: Some("lena".into()),
        status_pending: false,
    };
    let (thread, opener) = store
        .writes()
        .add_thread(
            changeset,
            None,
            CommentTarget::Range(anchor(Side::Target, 2)),
            posted("C1", "why?", "why?"),
            Some(platform.clone()),
            ThreadStatus::Resolved,
        )
        .await
        .unwrap();
    // A draft of the viewer's is written after the platform's reply, which then comes second.
    let draft = store.writes().add_message(thread, message("agreed")).await.unwrap();
    let reply = store
        .writes()
        .add_message(thread, posted("C2", "no", "no"))
        .await
        .unwrap();
    let posting = Message {
        outgoing: Outgoing::Posting { since: T0 + 20 },
        ..message("on its way")
    };
    let posting = store.writes().add_message(thread, posting).await.unwrap();
    let failed = store.writes().add_message(thread, message("refused")).await.unwrap();
    let refused = Message {
        id: failed,
        outgoing: Outgoing::Failed {
            error: "the thread is locked".into(),
        },
        ..message("refused")
    };
    store.writes().replace_message(refused).await.unwrap();

    let edited = Message {
        id: opener,
        body: "why not?".into(),
        edited_at: Some(T0 + 5),
        remote: Some(RemoteComment {
            id: "C1".into(),
            synced: "why?".into(),
            conflict: Some(EditConflict {
                remote_body: "why so?".into(),
                remote_at: T0 + 9,
            }),
        }),
        ..posted("C1", "why?", "why?")
    };
    store.writes().replace_message(edited.clone()).await.unwrap();
    store.writes().delete_message(reply).await.unwrap();
    store
        .writes()
        .set_thread_platform(
            thread,
            PlatformThread {
                status_pending: true,
                ..platform.clone()
            },
        )
        .await
        .unwrap();

    drop(store);
    let store = open(&dir).await;
    assert_eq!(store.changeset("/repo".into(), source).await.unwrap(), Some(changeset));
    let [found] = <[_; 1]>::try_from(store.threads(changeset).await.unwrap()).expect("one thread");
    let found = found.thread;
    assert_eq!(found.status, ThreadStatus::Resolved);
    assert_eq!(
        found.platform,
        Some(PlatformThread {
            status_pending: true,
            ..platform
        })
    );
    assert!(found.is_posted());
    assert_eq!(found.messages.len(), 4, "the reply was removed");
    assert_eq!(found.messages[0], edited);
    assert_eq!(
        (found.messages[1].id, found.messages[1].remote.clone()),
        (draft, None),
        "a draft has no remote"
    );
    assert!(found.messages[1].mine);
    assert_eq!(
        found.messages[1..]
            .iter()
            .map(|m| (m.id, m.outgoing.clone()))
            .collect::<Vec<_>>(),
        [
            (draft, Outgoing::Draft),
            (posting, Outgoing::Posting { since: T0 + 20 }),
            (
                failed,
                Outgoing::Failed {
                    error: "the thread is locked".into()
                }
            ),
        ]
    );
}

#[tokio::test]
async fn a_thread_outside_a_platform_has_none_and_a_review_started_later_finds_the_changeset() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, first) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    add_thread(
        &store,
        changeset,
        Some(first),
        CommentTarget::Changeset,
        message("local"),
    )
    .await;

    let [found] = <[_; 1]>::try_from(store.threads(changeset).await.unwrap()).expect("one thread");

    assert_eq!(found.thread.platform, None);
    assert!(!found.thread.is_posted());
    let again = store
        .writes()
        .ensure_changeset("/repo".into(), Source::WorkingTree)
        .await
        .unwrap();
    assert_eq!(again, changeset);
}

#[tokio::test]
async fn the_open_bases_are_the_latest_versions_of_the_changesets_with_an_open_review() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (open_one, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    store
        .writes()
        .append_version(open_one, version(2, SnapshotId(h(3)), SnapshotId(h(2))))
        .await
        .unwrap();
    let (_, closed) = store
        .writes()
        .start_review(
            "/other".into(),
            Source::WorkingTree,
            BaseChoice::Commit { id: commit("abc123") },
            vec![version(1, SnapshotId(h(4)), SnapshotId(h(5)))],
            T0,
        )
        .await
        .unwrap();
    store.writes().close_review(closed, T0, true).await.unwrap();

    let bases = open_bases(&store).await.unwrap();

    assert_eq!(bases, HashSet::from([SnapshotId(h(3))]));
}

#[tokio::test]
async fn versions_are_numbered_per_changeset_and_a_new_review_starts_over() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    let changeset;
    {
        let store = open(&dir).await;
        (changeset, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
        let moved = Version {
            base_commit: commit("def456"),
            origin: VersionOrigin::Observed,
            ..version(3, SnapshotId(h(4)), SnapshotId(h(3)))
        };
        store
            .writes()
            .append_version(changeset, version(2, SnapshotId(h(1)), SnapshotId(h(3))))
            .await
            .unwrap();
        store.writes().append_version(changeset, moved).await.unwrap();
        assert!(
            store
                .writes()
                .append_version(changeset, version(3, SnapshotId(h(1)), SnapshotId(h(5))))
                .await
                .is_err(),
            "a number is used once"
        );
    }
    let store = open(&dir).await;
    let versions = store.versions(changeset).await.unwrap();
    assert_eq!(
        versions
            .iter()
            .map(|v| (v.n, v.snapshots, v.origin))
            .collect::<Vec<_>>(),
        [
            (1, Some(snapshots(1, 2)), VersionOrigin::Settled),
            (2, Some(snapshots(1, 3)), VersionOrigin::Settled),
            (3, Some(snapshots(4, 3)), VersionOrigin::Observed),
        ]
    );
    assert_eq!(
        (versions[2].base_commit.clone(), versions[2].tip.clone(), versions[2].at),
        (commit("def456"), commit("abc123"), 30)
    );

    store.writes().close_review(id, T0, false).await.unwrap();
    let (_, next) = review(&store, SnapshotId(h(6)), SnapshotId(h(7))).await;
    assert_ne!(next, id);
    let versions = store.versions(changeset).await.unwrap();
    assert_eq!(
        versions.iter().map(|v| (v.n, v.snapshots)).collect::<Vec<_>>(),
        [(1, Some(snapshots(6, 7)))]
    );
}

#[tokio::test]
async fn what_a_version_changed_is_stored_with_it_in_order_and_goes_with_its_review() {
    let dir = tempfile::tempdir().unwrap();
    let changed = |path: &str, added, removed| ChangedFile {
        path: RepoPath(path.as_bytes().to_vec()),
        delta: LineDelta { added, removed },
    };
    let change = VersionChange {
        files: vec![changed("a.rs", 3, 1), changed("b.rs", 0, 0), changed("c/d.rs", 40, 0)],
    };
    let changeset;
    {
        let store = open(&dir).await;
        (changeset, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
        let second = Version {
            change: change.clone(),
            ..version(2, SnapshotId(h(1)), SnapshotId(h(3)))
        };
        store.writes().append_version(changeset, second).await.unwrap();
    }

    let store = open(&dir).await;
    let versions = store.versions(changeset).await.unwrap();
    assert_eq!(versions[0].change, VersionChange::default());
    assert_eq!(versions[1].change, change);

    review(&store, SnapshotId(h(6)), SnapshotId(h(7))).await;
    let rows = version_file::Entity::find().all(&store.reader.db).await.unwrap();
    assert!(rows.is_empty(), "the files of the replaced versions are gone");
}

#[tokio::test]
async fn a_version_without_snapshots_is_stored_listed_and_collected_around() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let lost = Version {
        snapshots: None,
        ..version(2, SnapshotId(h(1)), SnapshotId(h(3)))
    };
    store.writes().append_version(changeset, lost.clone()).await.unwrap();

    let versions = store.versions(changeset).await.unwrap();
    assert_eq!(versions[1], lost);
    assert_eq!(versions[0].snapshots, Some(snapshots(1, 2)));
    gc(&store, T0 + 1).await;
    assert_eq!(store.versions(changeset).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_review_records_when_it_ended_and_what_was_last_reviewed() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let open_review = store.review(id).await.unwrap().unwrap();
    assert_eq!(
        (open_review.changeset, open_review.last_reviewed, open_review.opened_at),
        (changeset, None, T0)
    );
    assert_eq!((open_review.closed_at, open_review.complete), (None, None));
    assert_eq!(store.open_review(changeset).await.unwrap(), Some(id));
    assert_eq!(store.open_review(ChangesetId(changeset.0 + 1)).await.unwrap(), None);

    store.writes().set_last_reviewed(id, 2).await.unwrap();
    store.writes().close_review(id, 1, true).await.unwrap();
    assert_eq!(store.open_review(changeset).await.unwrap(), None);
    let closed = store.review(id).await.unwrap().unwrap();
    assert_eq!(
        (closed.last_reviewed, closed.closed_at, closed.complete),
        (Some(2), Some(1), Some(true))
    );
}

#[tokio::test]
async fn snapshots_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let snap = snapshot(&[("a", 1), ("b/c/d", 2), ("b/e", 3)]);
    let status = SnapshotStatus {
        path_collisions: vec![RepoPath(b"A".to_vec())],
        content_unavailable: vec![RepoPath(b"big".to_vec())],
    };
    {
        let store = open(&dir).await;
        store
            .writes()
            .put_snapshot(snap.clone(), status.clone(), T0)
            .await
            .unwrap();
        store
            .writes()
            .put_snapshot(snap.clone(), status.clone(), T0)
            .await
            .unwrap();
    }
    let store = open(&dir).await;
    assert_eq!(store.snapshot(snap.id).await.unwrap(), Some((snap, status)));
    assert_eq!(store.snapshot(SnapshotId(h(99))).await.unwrap(), None);
}

#[tokio::test]
async fn a_batch_of_blobs_is_answered_in_order_and_held_after_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    store.put_blobs(vec![vec![3]]).await.unwrap();
    // More blobs than the budget has units, a duplicate and one held already.
    let blobs: Vec<Vec<u8>> = (0..40u8)
        .map(|n| vec![n + 1; 1 + n as usize])
        .chain([vec![3], vec![7; 8]])
        .collect();
    let ids: Vec<BlobId> = blobs.iter().map(|b| BlobId::of(b)).collect();
    let missing = store.missing_blobs(ids.clone()).await.unwrap();
    assert_eq!(missing, [&ids[..40], &ids[41..]].concat());

    assert_eq!(store.put_blobs(blobs.clone()).await.unwrap(), ids);
    assert_eq!(store.missing_blobs(ids.clone()).await.unwrap(), []);
    drop(store);
    let store = open(&dir).await;
    for (id, bytes) in ids.iter().zip(&blobs) {
        assert_eq!(store.blob(*id).await.unwrap(), Some(StoredBlob::Bytes(bytes.clone())));
    }
}

#[tokio::test]
async fn blobs_read_together_are_those_read_one_by_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let held: Vec<Vec<u8>> = (0..20u8).map(|n| vec![n + 1; 100 + n as usize]).collect();
    let mut ids = store.put_blobs(held).await.unwrap();
    // One the store never had, one torn, and one asked for twice.
    let torn = ids[3];
    sqlx::query("UPDATE blob SET data = x'746f726e', codec = 0 WHERE id = ?")
        .bind(torn.0.0.as_slice())
        .execute(&store.content.writer)
        .await
        .unwrap();
    ids.extend([BlobId::of(b"never"), ids[5]]);

    let together = store.blobs(ids.clone()).await.unwrap();
    let mut alone = BTreeMap::new();
    for id in ids {
        if let Some(bytes) = store.blob(id).await.unwrap() {
            alone.insert(id, bytes);
        }
    }
    assert_eq!(together, alone);
    assert_eq!(together.len(), 19);
    assert!(!together.contains_key(&torn));
}

#[tokio::test]
async fn gc_keeps_referenced_content_and_drops_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let snaps = [21u8, 22, 23].map(|n| snapshot(&[("a", n)]));
    for s in &snaps {
        store
            .writes()
            .put_snapshot(s.clone(), Default::default(), T0)
            .await
            .unwrap();
    }
    store
        .put_blobs([21, 22, 23, 24].map(|n| vec![n]).to_vec())
        .await
        .unwrap();
    let (changeset, id) = review(&store, snaps[0].id, snaps[1].id).await;
    let now = T0;

    let report = gc(&store, now).await;
    assert_eq!(
        (
            report.changesets_deleted,
            report.snapshots_deleted,
            report.nodes_deleted,
            report.blobs_deleted
        ),
        (0, 1, 1, 2)
    );
    assert!(store.blob(BlobId::of(&[21])).await.unwrap().is_some());
    assert!(store.blob(BlobId::of(&[22])).await.unwrap().is_some());
    assert_eq!(store.blob(BlobId::of(&[23])).await.unwrap(), None);
    assert_eq!(store.blob(BlobId::of(&[24])).await.unwrap(), None);
    assert!(store.snapshot(snaps[2].id).await.unwrap().is_none());

    // A changeset is kept for the retention period after its review closed.
    store.writes().close_review(id, now, false).await.unwrap();
    assert_eq!(gc(&store, now + 29 * DAY).await, Default::default());
    assert!(store.review(id).await.unwrap().is_some());

    // An open thread keeps an expired changeset.
    let thread = add_thread(&store, changeset, Some(id), CommentTarget::Changeset, message("todo")).await;
    assert_eq!(gc(&store, now + 31 * DAY).await, Default::default());
    store
        .writes()
        .set_thread_status(thread, ThreadStatus::Resolved)
        .await
        .unwrap();

    // So does an open review, and the retention counts from the last closed one.
    let (_, next) = review(&store, snaps[0].id, snaps[1].id).await;
    assert_eq!(gc(&store, now + 31 * DAY).await, Default::default());
    store.writes().close_review(next, now + 31 * DAY, true).await.unwrap();
    assert_eq!(gc(&store, now + 32 * DAY).await, Default::default());

    let report = gc(&store, now + 62 * DAY).await;
    assert_eq!(
        (
            report.changesets_deleted,
            report.snapshots_deleted,
            report.nodes_deleted,
            report.blobs_deleted
        ),
        (1, 2, 2, 2)
    );
    assert!(store.review(id).await.unwrap().is_none());
    assert!(store.threads(changeset).await.unwrap().is_empty());
    assert!(store.versions(changeset).await.unwrap().is_empty());
    assert_eq!(
        store.changeset("/repo".into(), Source::WorkingTree).await.unwrap(),
        None
    );
    assert_eq!(store.blob(BlobId::of(&[21])).await.unwrap(), None);
}

#[tokio::test]
async fn gc_keeps_what_an_open_thread_of_a_closed_review_is_anchored_in() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let snaps = [31u8, 32, 33].map(|n| snapshot(&[("a", n)]));
    for s in &snaps {
        store
            .writes()
            .put_snapshot(s.clone(), Default::default(), T0)
            .await
            .unwrap();
    }
    let (changeset, id) = review(&store, snaps[0].id, snaps[1].id).await;
    let anchored = Anchor {
        snapshot: snaps[2].id,
        ..anchor(Side::Target, 0)
    };
    add_thread(
        &store,
        changeset,
        Some(id),
        CommentTarget::Range(anchored),
        message("open"),
    )
    .await;
    store.writes().close_review(id, T0, false).await.unwrap();

    let report = gc(&store, T0 + 100 * DAY).await;
    assert_eq!((report.changesets_deleted, report.snapshots_deleted), (0, 0));
    for s in &snaps {
        assert!(store.snapshot(s.id).await.unwrap().is_some());
    }
}

#[tokio::test]
async fn gc_grace_protects_fresh_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let retention = Retention::default();
    store.put_blobs(vec![vec![1]]).await.unwrap();
    store
        .writes()
        .put_snapshot(snapshot(&[("a", 1)]), Default::default(), T0)
        .await
        .unwrap();
    assert_eq!(collect(&store, T0, &retention).await.unwrap(), Default::default());
    assert!(store.blob(BlobId::of(&[1])).await.unwrap().is_some());
    assert!(store.snapshot(snapshot(&[("a", 1)]).id).await.unwrap().is_some());

    let later = collect(&store, T0 + 3600, &retention).await.unwrap();
    assert_eq!(later.snapshots_deleted, 1);
}

async fn node_count(store: &SqliteStore) -> u64 {
    tree_node::Entity::find().count(&store.reader.db).await.unwrap()
}

#[tokio::test]
async fn gc_drops_a_node_only_when_no_retained_root_reaches_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let kept = snapshot(&[("a/x", 1), ("b/y", 2)]);
    let other = snapshot(&[("a/x", 9), ("b/y", 2)]);
    for s in [&kept, &other] {
        store
            .writes()
            .put_snapshot(s.clone(), Default::default(), T0)
            .await
            .unwrap();
    }
    assert_eq!(node_count(&store).await, 5); // root, a, b, and the second snapshot's own root and `a`
    assert_eq!(store.snapshot(other.id).await.unwrap().unwrap().0, other);
    review(&store, kept.id, kept.id).await;

    let report = gc(&store, T0).await;
    assert_eq!((report.snapshots_deleted, report.nodes_deleted), (1, 2)); // other's root and `a`; `b` is shared
    assert_eq!(node_count(&store).await, 3);
    assert_eq!(store.snapshot(kept.id).await.unwrap().unwrap().0, kept);
}

#[tokio::test]
async fn a_long_run_of_versions_over_a_large_tree_stays_small() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let mut entries: BTreeMap<RepoPath, FileEntry> = (0..2000)
        .map(|i| (RepoPath(format!("d{:02}/f{i}", i % 100).into_bytes()), file_entry(0)))
        .collect();
    let (changeset, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    for i in 0..50u32 {
        let path = RepoPath(format!("d{:02}/f{}", (i * 7) % 100, (i * 7) % 100).into_bytes());
        entries.insert(
            path,
            FileEntry {
                blob: BlobId::of(&i.to_le_bytes()),
                ..file_entry(0)
            },
        );
        let snap = Snapshot {
            id: merkle::snapshot_id(&entries),
            entries: entries.clone(),
        };
        store
            .writes()
            .put_snapshot(snap.clone(), Default::default(), T0)
            .await
            .unwrap();
        store
            .writes()
            .append_version(changeset, version(i + 2, SnapshotId(h(1)), snap.id))
            .await
            .unwrap();
    }
    // 101 nodes for the first snapshot (which includes edit 0), then a root and one directory per later edit.
    assert_eq!(node_count(&store).await, 101 + 2 * 49);
    let rows = tree_child::Entity::find().count(&store.reader.db).await.unwrap();
    assert!(rows < 2000 + 100 + 50 * 125, "{rows} child rows");
    let last = store
        .versions(changeset)
        .await
        .unwrap()
        .last()
        .unwrap()
        .snapshots
        .unwrap()
        .target;
    assert_eq!(store.snapshot(last).await.unwrap().unwrap().0.entries, entries);
}

#[tokio::test]
async fn recorded_spans_survive_remapping_so_a_revert_restores_coverage() {
    let text = |line15: &str| -> String {
        (0..40)
            .map(|i| {
                if i == 15 {
                    format!("{line15}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect()
    };
    let (original, edited) = (text("line 15"), text("changed"));
    let (o, e) = (BlobId::of(original.as_bytes()), BlobId::of(edited.as_bytes()));
    let blobs = BTreeMap::from([(o, original.clone().into_bytes()), (e, edited.into_bytes())]);
    let path = || Some(RepoPath(b"a.rs".to_vec()));
    let input = |new| FileRemapInput {
        old_path: path(),
        new_path: path(),
        old_blob: Some(o),
        new_blob: Some(new),
    };
    let now = Changeset {
        base: SnapshotId(h(2)),
        target: SnapshotId(h(3)),
    };

    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (_, id) = review(&store, SnapshotId(h(2)), SnapshotId(h(3))).await;
    let recorded = Anchor::from_text(
        Side::Target,
        SnapshotId(h(1)),
        path().unwrap(),
        original.as_bytes(),
        LineRange { start: 10, end: 30 },
    )
    .unwrap();
    let span = ReviewedSpan {
        base: None,
        target: Some(recorded),
        state: SpanState::Seen,
    };
    let ids = store
        .writes()
        .edit_spans(id, vec![], vec![(span.clone(), SpanOrigin::Squash)])
        .await
        .unwrap();

    let remapped = async |new| {
        let review = store.review(id).await.unwrap().unwrap();
        let input = input(new);
        let target = PreparedFile::new(&input, &blobs);
        review
            .spans
            .iter()
            .flat_map(|r| remap_span(&r.span, now, None, Some(&target), RemapMode::SameUnit))
            .count()
    };
    assert_eq!(remapped(e).await, 2, "the edited line splits the span");
    assert_eq!(remapped(o).await, 1, "reverted: full coverage again");
    assert_eq!(
        store.review(id).await.unwrap().unwrap().spans[0].span,
        span,
        "nothing was overwritten"
    );

    store.writes().edit_spans(id, ids, vec![]).await.unwrap();
    assert_eq!(remapped(o).await, 0, "un-marked");
}

/// Far longer than any answer of an idle store takes, so that only a test that hangs runs into it.
const PROMPT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn a_mark_is_written_while_a_long_read_is_open_and_the_read_does_not_see_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (_, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;

    let reading = store.read().await.unwrap();
    assert!(span::Entity::find().all(&reading).await.unwrap().is_empty());

    let span = ReviewedSpan {
        base: None,
        target: Some(anchor(Side::Target, 2)),
        state: SpanState::Seen,
    };
    tokio::time::timeout(
        PROMPT,
        store.writes().edit_spans(id, vec![], vec![(span, SpanOrigin::Squash)]),
    )
    .await
    .expect("a write waits for a read")
    .unwrap();

    assert!(
        span::Entity::find().all(&reading).await.unwrap().is_empty(),
        "the read stays one state of the file"
    );
    drop(reading);
    assert_eq!(store.review(id).await.unwrap().unwrap().spans.len(), 1);
}

#[tokio::test]
async fn reads_are_answered_while_a_write_is_open_and_see_only_what_was_committed() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;

    let writing = store.writer.db.begin().await.unwrap();
    let other = insert_changeset(&writing, "/other".into(), &Source::WorkingTree)
        .await
        .unwrap();

    let all_reads = async {
        let found = store.changeset("/other".into(), Source::WorkingTree).await.unwrap();
        let versions = store.versions(changeset).await.unwrap();
        let threads = store.threads(changeset).await.unwrap();
        let snapshot = store.snapshot(SnapshotId(h(1))).await.unwrap();
        (found, versions.len(), threads.len(), snapshot)
    };
    let (found, versions, threads, snapshot) = tokio::time::timeout(PROMPT, all_reads)
        .await
        .expect("a read waits for a write");
    assert_eq!((found, versions, threads, snapshot), (None, 1, 0, None));

    writing.commit().await.unwrap();
    assert_eq!(
        store.changeset("/other".into(), Source::WorkingTree).await.unwrap(),
        Some(ChangesetId(other as u64))
    );
}

/// The details of a pull request with just a description, enough for the store's test.
fn discussion(description: &str) -> Discussion {
    Discussion {
        title: "PR".to_owned(),
        url: "https://github.com/acme/app/pull/7".to_owned(),
        description: description.to_owned(),
        draft: false,
        source_branch: "feature".to_owned(),
        target_branch: "main".to_owned(),
        state: PullRequestState::Open,
        author: "ada".to_owned(),
        author_name: "Ada".to_owned(),
        author_avatar: None,
        reviewers: Vec::new(),
        viewer_verdict: None,
        labels: Vec::new(),
        ci: None,
        decision: ReviewDecision::Undecided,
        blocked: false,
        conversation: Vec::new(),
        threads: Vec::new(),
        complete: true,
    }
}

fn app() -> RemoteKey {
    RemoteKey {
        host: "github.com".to_owned(),
        path: "acme/app".to_owned(),
    }
}

#[tokio::test]
async fn a_pull_requests_details_are_kept_an_older_copy_replaces_nothing_newer_and_the_oldest_read_goes_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let writes = store.writes();

    writes
        .set_discussion(app(), 7, "c1".into(), 100, T0, &discussion("first"))
        .await
        .unwrap();
    let kept = store.stored_discussion(app(), 7).await.unwrap().unwrap();
    assert_eq!((kept.updated_at, kept.discussion.description.as_str()), (100, "first"));

    // A copy older than what is kept replaces nothing.
    writes
        .set_discussion(app(), 7, "c1".into(), 50, T0 + 1, &discussion("older"))
        .await
        .unwrap();
    assert_eq!(
        store
            .stored_discussion(app(), 7)
            .await
            .unwrap()
            .unwrap()
            .discussion
            .description,
        "first"
    );

    // What the platform says now does.
    writes
        .set_discussion(app(), 7, "c2".into(), 200, T0 + 2, &discussion("newer"))
        .await
        .unwrap();
    assert_eq!(
        store
            .stored_discussion(app(), 7)
            .await
            .unwrap()
            .unwrap()
            .discussion
            .description,
        "newer"
    );

    // The table is bounded: only the 256 pull requests read most recently are kept.
    for n in 1..=300u64 {
        writes
            .set_discussion(app(), n, "c".into(), 1, T0 + 100 + n as i64, &discussion("x"))
            .await
            .unwrap();
    }
    assert!(
        store.stored_discussion(app(), 7).await.unwrap().is_none(),
        "the pull request read longest ago went first"
    );
    assert!(store.stored_discussion(app(), 300).await.unwrap().is_some());
    let kept = store.stored_discussion(app(), 250).await.unwrap();
    assert!(kept.is_some());
}
