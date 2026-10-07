use std::collections::BTreeMap;
use std::time::Duration;

use sea_orm::PaginatorTrait;
use tempfile::TempDir;
use vet_app::ports::{ReviewStore, Store};
use vet_app::review::{NonTextAck, Version};
use vet_core::content::{
    BlobId, Changeset, EntryKind, FileEntry, Hash, LineRange, NonTextKind, RepoPath, Snapshot, SnapshotId,
    SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::platform::PlatformThread;
use vet_core::review::{Anchor, CommentTarget, Message, ReviewedSpan, Side, SpanState, ThreadStatus};

use super::*;

const DAY: i64 = 24 * 3600;
/// A fixed "now", so retention and grace are deterministic.
const T0: i64 = 1_700_000_000;

fn h(n: u8) -> Hash {
    Hash([n; 32])
}

async fn open(dir: &TempDir) -> SqliteStore {
    let config = StoreConfig {
        gc_grace: Duration::ZERO,
        ..Default::default()
    };
    SqliteStore::open(dir.path(), config).await.unwrap()
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
    Message {
        author: "me".into(),
        body: body.into(),
        at: 1_700_000_000,
    }
}

async fn session(store: &SqliteStore, base: SnapshotId, target: SnapshotId) -> SessionId {
    let pin = store.put_pin(CommitId("abc123".into()), base).await.unwrap();
    let head = CommitId("abc123".into());
    store
        .open_session(
            "/repo".into(),
            pin.id,
            Changeset { base, target },
            BaseChoice::Commit { id: head.clone() },
            head,
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
    }
}

#[tokio::test]
async fn the_filter_is_stored_per_repository_and_replaced_as_a_whole() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    assert_eq!(store.filter("/a".into()).await.unwrap(), ReviewFilter::default());

    store.set_filter("/a".into(), sample_filter()).await.unwrap();
    let other = ReviewFilter {
        exclude: vec!["b".into()],
        ..Default::default()
    };
    store.set_filter("/b".into(), other.clone()).await.unwrap();
    let narrower = ReviewFilter {
        exclude: vec!["x".into()],
        ..Default::default()
    };
    store.set_filter("/a".into(), narrower.clone()).await.unwrap();
    store.set_filter("/b".into(), other.clone()).await.unwrap();

    drop(store);
    let store = open(&dir).await;
    assert_eq!(store.filter("/a".into()).await.unwrap(), narrower);
    assert_eq!(store.filter("/b".into()).await.unwrap(), other);
    assert_eq!(store.filter("/c".into()).await.unwrap(), ReviewFilter::default());

    store.set_filter("/a".into(), sample_filter()).await.unwrap();
    assert_eq!(store.filter("/a".into()).await.unwrap(), sample_filter());
}

#[tokio::test]
async fn a_database_of_another_version_is_recreated_and_one_of_this_version_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let db = connect(dir.path()).await.unwrap();
    // What an earlier version left behind: its migrations and a differently shaped session table.
    for sql in [
        "CREATE TABLE seaql_migrations (version VARCHAR NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)",
        "INSERT INTO seaql_migrations VALUES ('m0001_initial', 1), ('m0002_run_state', 2)",
        "CREATE TABLE session (id INTEGER PRIMARY KEY, repo TEXT)",
        "INSERT INTO session VALUES (1, '/old')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    db.close().await.unwrap();

    let store = open(&dir).await;
    assert!(store.open_sessions("/old".into()).await.unwrap().is_empty());
    let id = session(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    drop(store);

    let store = open(&dir).await;
    assert_eq!(store.open_sessions("/repo".into()).await.unwrap(), vec![id]);
}

#[tokio::test]
async fn the_base_choice_and_snapshot_are_stored_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let id = session(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let loaded = |store: &SqliteStore| {
        let store = store.clone();
        async move { store.session(id).await.unwrap().unwrap() }
    };
    let s = loaded(&store).await;
    assert_eq!(
        (s.base, s.base_commit),
        (
            BaseChoice::Commit {
                id: CommitId("abc123".into())
            },
            CommitId("abc123".into())
        )
    );

    let merge_base = BaseChoice::MergeBase {
        reference: "main".into(),
    };
    let at = CommitId("m1".into());
    store
        .set_base(id, SnapshotId(h(3)), merge_base.clone(), at.clone())
        .await
        .unwrap();
    let s = loaded(&store).await;
    assert_eq!(
        (s.base, s.base_commit, s.changeset.base),
        (merge_base, at, SnapshotId(h(3)))
    );

    let commit = BaseChoice::Commit {
        id: CommitId("abc".into()),
    };
    store
        .set_base(id, SnapshotId(h(4)), commit.clone(), CommitId("abc".into()))
        .await
        .unwrap();
    drop(store);
    let store = open(&dir).await;
    let s = loaded(&store).await;
    assert_eq!((s.base, s.changeset.base), (commit, SnapshotId(h(4))));

    let branch = BaseChoice::Branch { name: "main".into() };
    let tip = CommitId("t1".into());
    store
        .set_base(id, SnapshotId(h(1)), branch.clone(), tip.clone())
        .await
        .unwrap();
    let s = loaded(&store).await;
    assert_eq!((s.base, s.base_commit), (branch, tip));
}

#[tokio::test]
async fn database_is_in_wal_mode() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    store.put_snapshot(snapshot(&[]), Default::default(), T0).await.unwrap();
    assert!(dir.path().join("vet.sqlite-wal").exists());
}

#[tokio::test]
async fn spans_and_threads_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let id = session(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;

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
            state: SpanState::Seen,
        },
    ];
    let ids = store.edit_spans(id, vec![], spans.clone()).await.unwrap();
    assert_eq!(ids.len(), 3);

    let range = store
        .add_thread(id, CommentTarget::Range(anchor(Side::Target, 2)), message("why?"))
        .await
        .unwrap();
    let file = store
        .add_thread(
            id,
            CommentTarget::File {
                side: Side::Base,
                path: RepoPath(b"a/b".to_vec()),
            },
            message("file"),
        )
        .await
        .unwrap();
    let whole = store
        .add_thread(id, CommentTarget::Changeset, message("overall"))
        .await
        .unwrap();
    store.add_message(range, message("because")).await.unwrap();
    store.set_thread_status(file, ThreadStatus::Resolved).await.unwrap();
    let platform = PlatformThread {
        remote_id: "r1".into(),
        remote_status: Some("Won't fix".into()),
        resolvable: true,
        blocks_merge: false,
    };
    store.set_thread_platform(whole, Some(platform.clone())).await.unwrap();

    let ack = NonTextAck {
        kind: NonTextKind::Binary,
        before: Some((RepoPath(b"x".to_vec()), file_entry(5))),
        after: None,
    };
    store.acknowledge(id, ack.clone()).await.unwrap();
    store.acknowledge(id, ack.clone()).await.unwrap();
    // Same entries, another kind: a separate item.
    let other = NonTextAck {
        kind: NonTextKind::Mode,
        ..ack.clone()
    };
    store.acknowledge(id, other.clone()).await.unwrap();

    let loaded = store.session(id).await.unwrap().unwrap();
    assert_eq!(loaded.spans.iter().map(|r| r.span.clone()).collect::<Vec<_>>(), spans);
    assert_eq!(loaded.acks, vec![ack.clone(), other.clone()]);
    assert_eq!(loaded.threads.len(), 3);
    assert_eq!(loaded.threads[0].target, CommentTarget::Range(anchor(Side::Target, 2)));
    assert_eq!(loaded.threads[0].messages, vec![message("why?"), message("because")]);
    assert_eq!(loaded.threads[1].status, ThreadStatus::Resolved);
    assert_eq!(
        loaded.threads[1].target,
        CommentTarget::File {
            side: Side::Base,
            path: RepoPath(b"a/b".to_vec())
        }
    );
    assert_eq!(loaded.threads[2].target, CommentTarget::Changeset);
    assert_eq!(loaded.threads[2].platform, Some(platform));

    let moved = CommentTarget::Range(Anchor {
        range: LineRange { start: 20, end: 25 },
        ..anchor(Side::Target, 2)
    });
    store.set_thread_target(range, moved.clone()).await.unwrap();
    store.delete_thread(file).await.unwrap();
    store.unacknowledge(id, ack).await.unwrap();
    assert_eq!(store.session(id).await.unwrap().unwrap().acks, vec![other.clone()]);
    store.unacknowledge(id, other).await.unwrap();
    store.edit_spans(id, ids, vec![]).await.unwrap();
    let loaded = store.session(id).await.unwrap().unwrap();
    assert_eq!(
        loaded.threads.iter().map(|t| &t.target).collect::<Vec<_>>(),
        vec![&moved, &CommentTarget::Changeset]
    );
    assert!(loaded.spans.is_empty() && loaded.acks.is_empty());
}

#[tokio::test]
async fn snapshots_timeline_and_approvals_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    let snap = snapshot(&[("a", 1), ("b/c/d", 2), ("b/e", 3)]);
    let status = SnapshotStatus {
        path_collisions: vec![RepoPath(b"A".to_vec())],
        content_unavailable: vec![RepoPath(b"big".to_vec())],
    };
    {
        let store = open(&dir).await;
        id = session(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
        store.put_snapshot(snap.clone(), status.clone(), T0).await.unwrap();
        store.put_snapshot(snap.clone(), status.clone(), T0).await.unwrap();
        for (at, s) in [(10, 3), (20, 1), (30, 2)] {
            store.append_timeline(id, SnapshotId(h(s)), at).await.unwrap();
        }
    }
    let store = open(&dir).await;
    assert_eq!(store.snapshot(snap.id).await.unwrap(), Some((snap, status)));
    assert_eq!(store.snapshot(SnapshotId(h(99))).await.unwrap(), None);
    let version = |s: u8, at| Version {
        snapshot: SnapshotId(h(s)),
        at,
    };
    assert_eq!(
        store.timeline(id).await.unwrap(),
        vec![version(3, 10), version(1, 20), version(2, 30)]
    );
    assert_eq!(store.open_sessions("/repo".into()).await.unwrap(), vec![id]);
    assert!(store.open_sessions("/other".into()).await.unwrap().is_empty());
    let stored = store.session(id).await.unwrap().unwrap();
    assert_eq!(stored.complete, None);
    assert_eq!(
        store.pin(stored.pin).await.unwrap().unwrap().head,
        CommitId("abc123".into())
    );
    store.close_session(id, 1, true).await.unwrap();
    assert!(store.open_sessions("/repo".into()).await.unwrap().is_empty());
    let closed = store.session(id).await.unwrap().unwrap();
    assert_eq!((closed.closed_at, closed.complete), (Some(1), Some(true)));
}

#[tokio::test]
async fn gc_keeps_referenced_content_and_drops_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let snaps = [21u8, 22, 23].map(|n| snapshot(&[("a", n)]));
    for s in &snaps {
        store.put_snapshot(s.clone(), Default::default(), T0).await.unwrap();
    }
    for n in [21, 22, 23, 24] {
        store.put_blob(vec![n]).await.unwrap();
    }
    let id = session(&store, snaps[0].id, snaps[1].id).await;
    let now = T0;

    let report = store.gc(now).await.unwrap();
    assert_eq!(
        (
            report.sessions_deleted,
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

    // Closed sessions are kept for the retention period.
    store.close_session(id, now, false).await.unwrap();
    assert_eq!(store.gc(now + 29 * DAY).await.unwrap(), Default::default());
    assert!(store.session(id).await.unwrap().is_some());

    // An open thread keeps an expired session.
    let thread = store
        .add_thread(id, CommentTarget::Changeset, message("todo"))
        .await
        .unwrap();
    assert_eq!(store.gc(now + 31 * DAY).await.unwrap(), Default::default());
    store.set_thread_status(thread, ThreadStatus::Resolved).await.unwrap();

    let report = store.gc(now + 31 * DAY).await.unwrap();
    assert_eq!(
        (
            report.sessions_deleted,
            report.snapshots_deleted,
            report.nodes_deleted,
            report.blobs_deleted
        ),
        (1, 2, 2, 2)
    );
    assert!(store.session(id).await.unwrap().is_none());
    assert_eq!(store.blob(BlobId::of(&[21])).await.unwrap(), None);
}

#[tokio::test]
async fn gc_grace_protects_fresh_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path(), StoreConfig::default()).await.unwrap();
    store.put_blob(vec![1]).await.unwrap();
    store
        .put_snapshot(snapshot(&[("a", 1)]), Default::default(), T0)
        .await
        .unwrap();
    assert_eq!(store.gc(T0).await.unwrap(), Default::default());
    assert!(store.blob(BlobId::of(&[1])).await.unwrap().is_some());
    assert!(store.snapshot(snapshot(&[("a", 1)]).id).await.unwrap().is_some());

    assert_eq!(store.gc(T0 + 3600).await.unwrap().snapshots_deleted, 1);
}

async fn node_count(store: &SqliteStore) -> u64 {
    tree_node::Entity::find().count(&store.db).await.unwrap()
}

#[tokio::test]
async fn snapshots_that_differ_in_one_file_share_all_other_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let before = snapshot(&[("a/x", 1), ("b/y", 2), ("c/d/z", 3)]);
    let after = snapshot(&[("a/x", 9), ("b/y", 2), ("c/d/z", 3)]);
    store.put_snapshot(before.clone(), Default::default(), T0).await.unwrap();
    assert_eq!(node_count(&store).await, 5); // root, a, b, c, c/d
    store.put_snapshot(after.clone(), Default::default(), T0).await.unwrap();
    assert_eq!(node_count(&store).await, 7); // plus a new root and a new `a`
    assert_eq!(store.snapshot(before.id).await.unwrap().unwrap().0, before);
    assert_eq!(store.snapshot(after.id).await.unwrap().unwrap().0, after);
}

#[tokio::test]
async fn gc_drops_a_node_only_when_no_retained_root_reaches_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let kept = snapshot(&[("a/x", 1), ("b/y", 2)]);
    let other = snapshot(&[("a/x", 9), ("b/y", 2)]);
    for s in [&kept, &other] {
        store.put_snapshot(s.clone(), Default::default(), T0).await.unwrap();
    }
    assert_eq!(node_count(&store).await, 5);
    session(&store, kept.id, kept.id).await;

    let report = store.gc(T0).await.unwrap();
    assert_eq!((report.snapshots_deleted, report.nodes_deleted), (1, 2)); // other's root and `a`; `b` is shared
    assert_eq!(node_count(&store).await, 3);
    assert_eq!(store.snapshot(kept.id).await.unwrap().unwrap().0, kept);
}

#[tokio::test]
async fn a_long_timeline_of_a_large_tree_stays_small() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let mut entries: BTreeMap<RepoPath, FileEntry> = (0..2000)
        .map(|i| (RepoPath(format!("d{:02}/f{i}", i % 100).into_bytes()), file_entry(0)))
        .collect();
    let id = session(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
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
        store.put_snapshot(snap.clone(), Default::default(), T0).await.unwrap();
        store.append_timeline(id, snap.id, 0).await.unwrap();
    }
    // 101 nodes for the first snapshot (which includes edit 0), then a root and one directory per later edit.
    assert_eq!(node_count(&store).await, 101 + 2 * 49);
    let rows = tree_child::Entity::find().count(&store.db).await.unwrap();
    assert!(rows < 2000 + 100 + 50 * 125, "{rows} child rows");
    let last = store.timeline(id).await.unwrap().last().unwrap().snapshot;
    assert_eq!(store.snapshot(last).await.unwrap().unwrap().0.entries, entries);
}

#[tokio::test]
async fn recorded_spans_survive_remapping_so_a_revert_restores_coverage() {
    use vet_core::prepared::PreparedFile;
    use vet_core::remap::remap_span;
    use vet_core::review::FileRemapInput;

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
    let id = session(&store, SnapshotId(h(2)), SnapshotId(h(3))).await;
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
    let ids = store.edit_spans(id, vec![], vec![span.clone()]).await.unwrap();

    let remapped = async |new| {
        let session = store.session(id).await.unwrap().unwrap();
        let input = input(new);
        let target = PreparedFile::new(&input, &blobs);
        session
            .spans
            .iter()
            .flat_map(|r| remap_span(&r.span, now, None, Some(&target)))
            .count()
    };
    assert_eq!(remapped(e).await, 2, "the edited line splits the span");
    assert_eq!(remapped(o).await, 1, "reverted: full coverage again");
    assert_eq!(
        store.session(id).await.unwrap().unwrap().spans[0].span,
        span,
        "nothing was overwritten"
    );

    store.edit_spans(id, ids, vec![]).await.unwrap();
    assert_eq!(remapped(o).await, 0, "un-marked");
}
