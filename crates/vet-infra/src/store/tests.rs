use std::collections::BTreeMap;
use std::time::Duration;

use tempfile::TempDir;
use vet_app::ports::{ReviewStore, Store};
use vet_app::review::NonTextAck;
use vet_core::content::{BlobId, Changeset, EntryKind, FileEntry, Hash, LineRange, RepoPath, ReviewFilter, Snapshot, SnapshotId};
use vet_core::platform::PlatformThread;
use vet_core::review::{Anchor, CommentTarget, Message, ReviewedSpan, Side, SpanState, ThreadStatus};

use super::*;

const DAY: i64 = 24 * 3600;

fn h(n: u8) -> Hash {
    Hash([n; 32])
}

async fn open(dir: &TempDir) -> SqliteStore {
    let config = StoreConfig { gc_grace: Duration::ZERO, ..Default::default() };
    SqliteStore::open(dir.path(), config).await.unwrap()
}

fn file_entry(blob: u8) -> FileEntry {
    FileEntry { blob: BlobId::of(&[blob]), mode: 0o100644, kind: EntryKind::File, unfiltered: false }
}

fn snapshot(id: u8, files: &[(&str, u8)]) -> Snapshot {
    let entries = files.iter().map(|(p, b)| (RepoPath(p.as_bytes().to_vec()), file_entry(*b))).collect::<BTreeMap<_, _>>();
    Snapshot { id: SnapshotId(h(id)), entries }
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
    Message { author: "me".into(), body: body.into(), at: 1_700_000_000 }
}

async fn session(store: &SqliteStore, base: u8, target: u8) -> SessionId {
    let pin = store.put_pin(CommitId("abc123".into()), SnapshotId(h(base))).await.unwrap();
    store.open_session(pin.id, Changeset { base: SnapshotId(h(base)), target: SnapshotId(h(target)) }, ReviewFilter::default()).await.unwrap()
}

#[tokio::test]
async fn database_is_in_wal_mode() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    store.put_snapshot(snapshot(1, &[])).await.unwrap();
    assert!(dir.path().join("vet.sqlite-wal").exists());
}

#[tokio::test]
async fn spans_and_threads_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let id = session(&store, 1, 2).await;

    let spans = vec![
        ReviewedSpan { base: Some(anchor(Side::Base, 1)), target: Some(anchor(Side::Target, 2)), state: SpanState::Seen },
        ReviewedSpan { base: None, target: Some(anchor(Side::Target, 2)), state: SpanState::SeenAgainstOldBase },
        ReviewedSpan { base: Some(anchor(Side::Base, 1)), target: None, state: SpanState::Seen },
    ];
    let ids = store.edit_spans(id, vec![], spans.clone()).await.unwrap();
    assert_eq!(ids.len(), 3);

    let range = store.add_thread(id, CommentTarget::Range(anchor(Side::Target, 2)), message("why?")).await.unwrap();
    let file = store.add_thread(id, CommentTarget::File { side: Side::Base, path: RepoPath(b"a/b".to_vec()) }, message("file")).await.unwrap();
    let whole = store.add_thread(id, CommentTarget::Changeset, message("overall")).await.unwrap();
    store.add_message(range, message("because")).await.unwrap();
    store.set_thread_status(file, ThreadStatus::Resolved).await.unwrap();
    let platform = PlatformThread { remote_id: "r1".into(), remote_status: Some("Won't fix".into()), resolvable: true, blocks_merge: false };
    store.set_thread_platform(whole, Some(platform.clone())).await.unwrap();

    let ack = NonTextAck { before: Some((RepoPath(b"x".to_vec()), file_entry(5))), after: None };
    store.acknowledge(id, ack.clone()).await.unwrap();
    store.acknowledge(id, ack.clone()).await.unwrap();

    let filter = ReviewFilter { roots: vec![RepoPath(b"crates".to_vec())], include: vec!["*.rs".into()], exclude: vec!["*.lock".into()], hide_binary: true };
    store.set_filter(id, filter.clone()).await.unwrap();

    let loaded = store.session(id).await.unwrap().unwrap();
    assert_eq!(loaded.spans.iter().map(|r| r.span.clone()).collect::<Vec<_>>(), spans);
    assert_eq!(loaded.acks, vec![ack.clone()]);
    assert_eq!(loaded.filter, filter);
    assert_eq!(loaded.threads.len(), 3);
    assert_eq!(loaded.threads[0].target, CommentTarget::Range(anchor(Side::Target, 2)));
    assert_eq!(loaded.threads[0].messages, vec![message("why?"), message("because")]);
    assert_eq!(loaded.threads[1].status, ThreadStatus::Resolved);
    assert_eq!(loaded.threads[1].target, CommentTarget::File { side: Side::Base, path: RepoPath(b"a/b".to_vec()) });
    assert_eq!(loaded.threads[2].target, CommentTarget::Changeset);
    assert_eq!(loaded.threads[2].platform, Some(platform));

    let moved = CommentTarget::Range(Anchor { range: LineRange { start: 20, end: 25 }, ..anchor(Side::Target, 2) });
    store.set_thread_target(range, moved.clone()).await.unwrap();
    store.delete_thread(file).await.unwrap();
    store.unacknowledge(id, ack).await.unwrap();
    store.edit_spans(id, ids, vec![]).await.unwrap();
    let loaded = store.session(id).await.unwrap().unwrap();
    assert_eq!(loaded.threads.iter().map(|t| &t.target).collect::<Vec<_>>(), vec![&moved, &CommentTarget::Changeset]);
    assert!(loaded.spans.is_empty() && loaded.acks.is_empty());
}

#[tokio::test]
async fn snapshots_timeline_and_approvals_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    let snap = snapshot(1, &[("a", 1), ("b", 2)]);
    {
        let store = open(&dir).await;
        id = session(&store, 1, 2).await;
        store.put_snapshot(snap.clone()).await.unwrap();
        store.put_snapshot(snap.clone()).await.unwrap();
        for s in [3, 1, 2] {
            store.append_timeline(id, SnapshotId(h(s))).await.unwrap();
        }
        store.record_run_approval("/repo".into(), h(1), 10).await.unwrap();
        store.record_run_approval("/repo".into(), h(2), 20).await.unwrap();
    }
    let store = open(&dir).await;
    assert_eq!(store.snapshot(snap.id).await.unwrap(), Some(snap));
    assert_eq!(store.snapshot(SnapshotId(h(99))).await.unwrap(), None);
    assert_eq!(store.timeline(id).await.unwrap(), vec![SnapshotId(h(3)), SnapshotId(h(1)), SnapshotId(h(2))]);
    assert_eq!(store.last_run_approval("/repo".into()).await.unwrap(), Some(h(2)));
    assert_eq!(store.last_run_approval("/other".into()).await.unwrap(), None);
    assert_eq!(store.open_sessions().await.unwrap(), vec![id]);
}

#[tokio::test]
async fn gc_keeps_referenced_content_and_drops_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    for (n, files) in [(1u8, [("a", 21u8)]), (2, [("a", 22)]), (3, [("a", 23)])] {
        store.put_snapshot(snapshot(n, &files)).await.unwrap();
    }
    for n in [21, 22, 23, 24] {
        store.put_blob(vec![n]).await.unwrap();
    }
    let id = session(&store, 1, 2).await;
    let now = unix_now();

    let report = store.gc(now).await.unwrap();
    assert_eq!((report.sessions_deleted, report.snapshots_deleted, report.blobs_deleted), (0, 1, 2));
    assert!(store.blob(BlobId::of(&[21])).await.unwrap().is_some());
    assert!(store.blob(BlobId::of(&[22])).await.unwrap().is_some());
    assert_eq!(store.blob(BlobId::of(&[23])).await.unwrap(), None);
    assert_eq!(store.blob(BlobId::of(&[24])).await.unwrap(), None);
    assert!(store.snapshot(SnapshotId(h(3))).await.unwrap().is_none());

    // Closed sessions are kept for the retention period.
    store.close_session(id, now).await.unwrap();
    assert_eq!(store.gc(now + 29 * DAY).await.unwrap(), Default::default());
    assert!(store.session(id).await.unwrap().is_some());

    // An open thread keeps an expired session.
    let thread = store.add_thread(id, CommentTarget::Changeset, message("todo")).await.unwrap();
    assert_eq!(store.gc(now + 31 * DAY).await.unwrap(), Default::default());
    store.set_thread_status(thread, ThreadStatus::Resolved).await.unwrap();

    let report = store.gc(now + 31 * DAY).await.unwrap();
    assert_eq!((report.sessions_deleted, report.snapshots_deleted, report.blobs_deleted), (1, 2, 2));
    assert!(store.session(id).await.unwrap().is_none());
    assert_eq!(store.blob(BlobId::of(&[21])).await.unwrap(), None);
}

#[tokio::test]
async fn gc_grace_protects_fresh_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path(), StoreConfig::default()).await.unwrap();
    store.put_blob(vec![1]).await.unwrap();
    store.put_snapshot(snapshot(1, &[("a", 1)])).await.unwrap();
    assert_eq!(store.gc(unix_now()).await.unwrap(), Default::default());
    assert!(store.blob(BlobId::of(&[1])).await.unwrap().is_some());
}

#[tokio::test]
async fn recorded_spans_survive_remapping_so_a_revert_restores_coverage() {
    use vet_core::prepared::PreparedFile;
    use vet_core::remap::remap_span;
    use vet_core::review::FileRemapInput;

    let text = |line15: &str| -> String { (0..40).map(|i| if i == 15 { format!("{line15}\n") } else { format!("line {i}\n") }).collect() };
    let (original, edited) = (text("line 15"), text("changed"));
    let (o, e) = (BlobId::of(original.as_bytes()), BlobId::of(edited.as_bytes()));
    let blobs = BTreeMap::from([(o, original.clone().into_bytes()), (e, edited.into_bytes())]);
    let path = || Some(RepoPath(b"a.rs".to_vec()));
    let input = |new| FileRemapInput { old_path: path(), new_path: path(), old_blob: Some(o), new_blob: Some(new) };
    let now = Changeset { base: SnapshotId(h(2)), target: SnapshotId(h(3)) };

    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let id = session(&store, 2, 3).await;
    let recorded = Anchor::from_text(Side::Target, SnapshotId(h(1)), path().unwrap(), original.as_bytes(), LineRange { start: 10, end: 30 }).unwrap();
    let span = ReviewedSpan { base: None, target: Some(recorded), state: SpanState::Seen };
    let ids = store.edit_spans(id, vec![], vec![span.clone()]).await.unwrap();

    let remapped = async |new| {
        let session = store.session(id).await.unwrap().unwrap();
        let input = input(new);
        let target = PreparedFile::new(&input, &blobs);
        session.spans.iter().flat_map(|r| remap_span(&r.span, now, None, Some(&target))).count()
    };
    assert_eq!(remapped(e).await, 2, "the edited line splits the span");
    assert_eq!(remapped(o).await, 1, "reverted: full coverage again");
    assert_eq!(store.session(id).await.unwrap().unwrap().spans[0].span, span, "nothing was overwritten");

    store.edit_spans(id, ids, vec![]).await.unwrap();
    assert_eq!(remapped(o).await, 0, "un-marked");
}
