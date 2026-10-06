use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use vet_core::content::{
    BlobId, EntryKind, FileDiff, FileEntry, Hunk, LineRange, NonTextChange, NonTextKind, RepoPath, Snapshot,
    SnapshotDiff, SnapshotId, SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::merkle;
use vet_core::platform::PlatformThread;
use vet_core::review::Side;
use vet_core::review::{CommentTarget, Message, ReviewedSpan, ThreadId, ThreadStatus};

use super::*;
use crate::pin::{BaseChoice, CommitId, Pin, PinId};
use crate::ports::{Dirty, GcReport};
use crate::review::Session;

fn path(s: &str) -> RepoPath {
    RepoPath(s.as_bytes().to_vec())
}

fn entry(text: &str, mode: u32) -> FileEntry {
    FileEntry {
        blob: BlobId::of(text.as_bytes()),
        mode,
        kind: EntryKind::File,
        unfiltered: false,
    }
}

fn snapshot(files: &[(&str, &str)]) -> Snapshot {
    let entries = files.iter().map(|(p, t)| (path(p), entry(t, 0o100644))).collect();
    Snapshot {
        id: merkle::snapshot_id(&entries),
        entries,
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// With equal line counts, one hunk per run of differing lines; otherwise one
/// hunk from the first to the last differing line.
fn hunks(old: &str, new: &str) -> Vec<Hunk> {
    let (o, n) = (lines(old), lines(new));
    if o.len() != n.len() {
        return hunk(old, new).into_iter().collect();
    }
    let mut out: Vec<Hunk> = Vec::new();
    for (i, (a, b)) in o.iter().zip(&n).enumerate() {
        let i = i as u32;
        match out.last_mut() {
            _ if a == b => {}
            Some(h) if h.old.end == i => {
                h.old.end += 1;
                h.new.end += 1;
            }
            _ => {
                let r = LineRange { start: i, end: i + 1 };
                out.push(Hunk { old: r, new: r });
            }
        }
    }
    out
}

fn hunk(old: &str, new: &str) -> Option<Hunk> {
    let (o, n) = (lines(old), lines(new));
    let prefix = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
    let suffix = o[prefix..]
        .iter()
        .rev()
        .zip(n[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let range = |len: usize| LineRange {
        start: prefix as u32,
        end: (len - suffix) as u32,
    };
    (old != new).then(|| Hunk {
        old: range(o.len()),
        new: range(n.len()),
    })
}

/// Git and worktree in one: every blob any snapshot refers to.
#[derive(Clone, Default)]
struct FakeGit {
    blobs: Arc<Mutex<HashMap<BlobId, Vec<u8>>>>,
    base: Arc<Mutex<Option<Snapshot>>>,
    scans: Arc<Mutex<Vec<Snapshot>>>,
    /// `<name>` of each held `refs/vet/sessions/<id>/<name>` and the commit.
    refs: Arc<Mutex<Vec<(String, CommitId)>>>,
    /// What revisions (`merge-base <ref>` for merge bases) resolve to, with the commit's tree.
    commits: Arc<Mutex<HashMap<String, (CommitId, Snapshot)>>>,
}

impl FakeGit {
    fn add_commit(&self, spec: &str, id: &str, files: &[(&str, &str)]) {
        let snapshot = self.snapshot(files);
        self.commits
            .lock()
            .unwrap()
            .insert(spec.into(), (CommitId(id.into()), snapshot));
    }

    fn snapshot(&self, files: &[(&str, &str)]) -> Snapshot {
        let mut blobs = self.blobs.lock().unwrap();
        for (_, text) in files {
            blobs.insert(BlobId::of(text.as_bytes()), text.as_bytes().to_vec());
        }
        snapshot(files)
    }
}

impl GitRepo for FakeGit {
    async fn head(&self) -> std::result::Result<CommitId, GitError> {
        Ok(CommitId("head".into()))
    }
    async fn scan(&self, _: Dirty) -> std::result::Result<Scan, GitError> {
        let snapshot = self.scans.lock().unwrap().remove(0);
        Ok(Scan {
            snapshot,
            status: SnapshotStatus::default(),
        })
    }
    async fn checkout<St: Store + Sync>(
        &self,
        _: &St,
        _: &CommitId,
        _: &Snapshot,
        _: &Path,
    ) -> std::result::Result<(), crate::ports::CheckoutError> {
        unreachable!()
    }
    async fn commit_snapshot(&self, commit: &CommitId) -> std::result::Result<Snapshot, GitError> {
        let known = self.commits.lock().unwrap();
        match known.values().find(|(c, _)| c == commit) {
            Some((_, snapshot)) => Ok(snapshot.clone()),
            None => Ok(self.base.lock().unwrap().clone().unwrap()),
        }
    }
    async fn read_blob(&self, _: &RepoPath, blob: BlobId) -> std::result::Result<Option<Vec<u8>>, GitError> {
        Ok(self.blobs.lock().unwrap().get(&blob).cloned())
    }
    /// The first byte of the blob id, in hex.
    async fn submodule_commit(&self, blob: BlobId) -> std::result::Result<Option<String>, GitError> {
        Ok(Some(format!("{:02x}", blob.0.0[0])))
    }
    async fn merge_base(&self, reference: &str) -> std::result::Result<CommitId, GitError> {
        self.commits
            .lock()
            .unwrap()
            .get(&format!("merge-base {reference}"))
            .map(|(c, _)| c.clone())
            .ok_or_else(|| GitError(format!("no common ancestor with {reference}")))
    }
    async fn resolve_commit(&self, spec: &str) -> std::result::Result<CommitId, GitError> {
        self.commits
            .lock()
            .unwrap()
            .get(spec)
            .map(|(c, _)| c.clone())
            .ok_or_else(|| GitError(format!("unknown commit {spec}")))
    }
    async fn base_candidates(&self, _: usize) -> std::result::Result<crate::ports::BaseCandidates, GitError> {
        unreachable!()
    }
    async fn set_session_ref(&self, _: SessionId, name: &str, commit: &CommitId) -> std::result::Result<(), GitError> {
        let mut refs = self.refs.lock().unwrap();
        refs.retain(|(n, _)| n != name);
        refs.push((name.into(), commit.clone()));
        Ok(())
    }
    async fn delete_session_refs(&self, _: SessionId) -> std::result::Result<(), GitError> {
        self.refs.lock().unwrap().clear();
        Ok(())
    }
    async fn diff<St: Store + Sync>(
        &self,
        _: &St,
        old: &Snapshot,
        new: &Snapshot,
    ) -> std::result::Result<SnapshotDiff, GitError> {
        let blobs = self.blobs.lock().unwrap();
        let text = |e: Option<&FileEntry>| {
            e.map(|e| String::from_utf8(blobs[&e.blob].clone()).unwrap())
                .unwrap_or_default()
        };
        let paths: BTreeSet<_> = old.entries.keys().chain(new.entries.keys()).collect();
        let mut out = SnapshotDiff::default();
        for p in paths {
            let (o, n) = (old.entries.get(p), new.entries.get(p));
            if o == n {
                continue;
            }
            let item = |kind| NonTextChange {
                path: p.clone(),
                from: None,
                kind,
                before: o.cloned(),
                after: n.cloned(),
            };
            if [o, n].iter().flatten().any(|e| e.kind == EntryKind::Submodule) {
                out.non_text.push(item(NonTextKind::Submodule));
                continue;
            }
            let mode_changed = matches!((o, n), (Some(a), Some(b)) if a.mode != b.mode);
            if mode_changed {
                out.non_text.push(item(NonTextKind::Mode));
            }
            if text(o).contains('\0') || text(n).contains('\0') {
                out.non_text.push(item(NonTextKind::Binary));
                continue;
            }
            if o.map(|a| a.blob) == n.map(|b| b.blob) {
                continue;
            }
            out.files.push(FileDiff {
                old_path: o.map(|_| p.clone()),
                new_path: n.map(|_| p.clone()),
                old_blob: o.map(|e| e.blob),
                new_blob: n.map(|e| e.blob),
                hunks: hunks(&text(o), &text(n)),
            });
        }
        Ok(out)
    }
}

#[derive(Default)]
struct Mem {
    blobs: HashMap<BlobId, Vec<u8>>,
    snapshots: HashMap<SnapshotId, (Snapshot, SnapshotStatus)>,
    timeline: Vec<SnapshotId>,
    spans: Vec<(SpanId, ReviewedSpan)>,
    next_span: u64,
    acks: Vec<NonTextAck>,
    base: Option<(SnapshotId, BaseChoice)>,
    closed: bool,
}

#[derive(Clone, Default)]
struct MemStore(Arc<Mutex<Mem>>);

type StoreResult<T> = std::result::Result<T, StoreError>;

impl Store for MemStore {
    async fn put_blob(&self, bytes: Vec<u8>) -> StoreResult<BlobId> {
        let id = BlobId::of(&bytes);
        self.0.lock().unwrap().blobs.insert(id, bytes);
        Ok(id)
    }
    async fn blob(&self, id: BlobId) -> StoreResult<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().blobs.get(&id).cloned())
    }
    async fn put_snapshot(&self, snapshot: Snapshot, status: SnapshotStatus) -> StoreResult<()> {
        self.0.lock().unwrap().snapshots.insert(snapshot.id, (snapshot, status));
        Ok(())
    }
    async fn snapshot(&self, id: SnapshotId) -> StoreResult<Option<(Snapshot, SnapshotStatus)>> {
        Ok(self.0.lock().unwrap().snapshots.get(&id).cloned())
    }
    async fn append_timeline(&self, _: SessionId, snapshot: SnapshotId) -> StoreResult<()> {
        self.0.lock().unwrap().timeline.push(snapshot);
        Ok(())
    }
    async fn timeline(&self, _: SessionId) -> StoreResult<Vec<SnapshotId>> {
        Ok(self.0.lock().unwrap().timeline.clone())
    }
    async fn gc(&self, _: i64) -> StoreResult<GcReport> {
        unimplemented!()
    }
}

impl ReviewStore for MemStore {
    async fn put_pin(&self, head: CommitId, tree: SnapshotId) -> StoreResult<Pin> {
        Ok(Pin {
            id: PinId(1),
            head,
            tree,
        })
    }
    async fn open_session(&self, _: PinId, _: Changeset, _: ReviewFilter) -> StoreResult<SessionId> {
        Ok(SessionId(1))
    }
    async fn close_session(&self, _: SessionId, _: i64) -> StoreResult<()> {
        self.0.lock().unwrap().closed = true;
        Ok(())
    }
    async fn open_sessions(&self) -> StoreResult<Vec<SessionId>> {
        unimplemented!()
    }
    async fn session(&self, _: SessionId) -> StoreResult<Option<Session>> {
        unimplemented!()
    }
    async fn set_filter(&self, _: SessionId, _: ReviewFilter) -> StoreResult<()> {
        unimplemented!()
    }
    async fn set_base(&self, _: SessionId, base: SnapshotId, choice: BaseChoice) -> StoreResult<()> {
        self.0.lock().unwrap().base = Some((base, choice));
        Ok(())
    }
    async fn edit_spans(&self, _: SessionId, remove: Vec<SpanId>, add: Vec<ReviewedSpan>) -> StoreResult<Vec<SpanId>> {
        let mut mem = self.0.lock().unwrap();
        mem.spans.retain(|(id, _)| !remove.contains(id));
        let mut ids = Vec::new();
        for span in add {
            mem.next_span += 1;
            let id = SpanId(mem.next_span);
            mem.spans.push((id, span));
            ids.push(id);
        }
        Ok(ids)
    }
    async fn acknowledge(&self, _: SessionId, ack: NonTextAck) -> StoreResult<()> {
        self.0.lock().unwrap().acks.push(ack);
        Ok(())
    }
    async fn unacknowledge(&self, _: SessionId, ack: NonTextAck) -> StoreResult<()> {
        self.0.lock().unwrap().acks.retain(|a| *a != ack);
        Ok(())
    }
    async fn add_thread(&self, _: SessionId, _: CommentTarget, _: Message) -> StoreResult<ThreadId> {
        unimplemented!()
    }
    async fn add_message(&self, _: ThreadId, _: Message) -> StoreResult<()> {
        unimplemented!()
    }
    async fn set_thread_status(&self, _: ThreadId, _: ThreadStatus) -> StoreResult<()> {
        unimplemented!()
    }
    async fn set_thread_target(&self, _: ThreadId, _: CommentTarget) -> StoreResult<()> {
        unimplemented!()
    }
    async fn set_thread_platform(&self, _: ThreadId, _: Option<PlatformThread>) -> StoreResult<()> {
        unimplemented!()
    }
    async fn delete_thread(&self, _: ThreadId) -> StoreResult<()> {
        unimplemented!()
    }
}

fn numbered(n: usize, edited: &[usize]) -> String {
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

struct Fixture {
    git: FakeGit,
    store: MemStore,
    session: LiveSession<FakeGit, MemStore>,
}

impl Fixture {
    async fn open(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Self {
        let git = FakeGit::default();
        *git.base.lock().unwrap() = Some(git.snapshot(base));
        let first = git.snapshot(dirty);
        git.scans.lock().unwrap().push(first);
        let store = MemStore::default();
        let session = LiveSession::open(git.clone(), store.clone(), "test".into())
            .await
            .unwrap();
        Self { git, store, session }
    }

    /// The tree settles at `files`.
    async fn settle(&self, files: &[(&str, &str)]) -> u32 {
        let snapshot = self.git.snapshot(files);
        let scan = Scan {
            snapshot,
            status: SnapshotStatus::default(),
        };
        self.session.settled(scan).await.unwrap().0
    }

    async fn hunks(&self, file: &str) -> Vec<HunkState> {
        let manifest = self.session.manifest().await.unwrap();
        let f = manifest.files.iter().find(|f| f.path == path(file)).unwrap();
        f.review.hunks.clone()
    }
}

#[tokio::test]
async fn open_stores_the_pin_and_retains_dirty_content() {
    let a = numbered(10, &[]);
    let b = numbered(10, &[3]);
    let fx = Fixture::open(&[("a.txt", &a)], &[("a.txt", &b), ("new.txt", "n\n")]).await;

    let mem = fx.store.0.lock().unwrap();
    assert!(mem.blobs.contains_key(&BlobId::of(b.as_bytes())));
    assert!(mem.blobs.contains_key(&BlobId::of(b"n\n")));
    assert!(
        !mem.blobs.contains_key(&BlobId::of(a.as_bytes())),
        "HEAD content stays in git"
    );
    assert_eq!(mem.snapshots.len(), 2);
    assert_eq!(mem.timeline.len(), 1);
}

#[tokio::test]
async fn manifest_lists_changes_with_unseen_hunks() {
    let a = numbered(10, &[]);
    let fx = Fixture::open(
        &[("a.txt", &a), ("gone.txt", "x\ny\n")],
        &[("a.txt", &numbered(10, &[3])), ("new.txt", "n\n")],
    )
    .await;

    let m = fx.session.manifest().await.unwrap();

    assert_eq!(m.revision, 0);
    let paths: Vec<_> = m.files.iter().map(|f| path_str(&f.path)).collect();
    assert_eq!(paths, ["a.txt", "gone.txt", "new.txt"]);
    let status: Vec<_> = m.files.iter().map(|f| f.status).collect();
    assert_eq!(status, [FileStatus::Modified, FileStatus::Deleted, FileStatus::Added]);
    assert_eq!((m.files[0].base_lines, m.files[0].target_lines), (10, 10));
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    // a.txt: 1 + 1 lines, gone.txt: 2, new.txt: 1
    assert_eq!((m.completion.lines, m.completion.seen_lines), (5, 0));
    assert!(!m.completion.is_complete());
}

#[tokio::test]
async fn marking_survives_edits_elsewhere_and_a_revert() {
    let base = numbered(30, &[]);
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;

    let update = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    assert_eq!((update.completion.lines, update.completion.seen_lines), (2, 2));
    assert!(update.changeset.is_complete());

    // An edit far away adds an unseen hunk and leaves the first one seen.
    let more = numbered(30, &[3, 25]);
    assert_eq!(fx.settle(&[("a.txt", &more)]).await, 1);
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Unseen]);

    // Reverting it restores the settled state of the first mark.
    fx.settle(&[("a.txt", &dirty)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);

    // Editing the marked line itself uncovers it; reverting covers it again.
    let edited = numbered(30, &[3]).replace("edited 3", "edited three");
    fx.settle(&[("a.txt", &edited)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Unseen]);
    fx.settle(&[("a.txt", &dirty)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
}

#[tokio::test]
async fn a_seen_region_with_an_edit_in_the_middle_is_split() {
    let base = numbered(30, &[]);
    let region: Vec<usize> = (5..25).collect();
    let dirty = numbered(30, &region);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    let middle = dirty
        .replace("edited 14", "mid a")
        .replace("edited 15", "mid b")
        .replace("edited 16", "mid c");
    fx.settle(&[("a.txt", &middle)]).await;

    let m = fx.session.manifest().await.unwrap();
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    // 20 base lines are still seen; 17 of 20 target lines are.
    assert_eq!((m.completion.lines, m.completion.seen_lines), (40, 37));
}

#[tokio::test]
async fn unmarking_removes_the_hunks_spans() {
    let base = numbered(30, &[]);
    let dirty = numbered(30, &[3, 20]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0, 1], true).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Seen]);

    let update = fx.session.set_seen(0, "a.txt", &[0], false).await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Unseen, HunkState::Seen]);
    assert_eq!(update.completion.seen_lines, 2);

    let update = fx.session.set_seen(0, "a.txt", &[1], false).await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Unseen, HunkState::Unseen]);
    assert!(fx.store.0.lock().unwrap().spans.is_empty());
}

#[tokio::test]
async fn unmarking_cuts_a_hunk_out_of_a_larger_span() {
    let base = numbered(40, &[]);
    let region: Vec<usize> = (5..25).collect();
    let dirty = numbered(40, &region);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    // Reverting the lower part shrinks the hunk to 5..10; its base side lies
    // inside the old span's 5..25.
    let shorter = numbered(40, &(5..10).collect::<Vec<_>>());
    let rev = fx.settle(&[("a.txt", &shorter)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
    let update = fx.session.set_seen(rev, "a.txt", &[0], false).await.unwrap();

    assert_eq!(update.review.hunks, [HunkState::Unseen]);
    assert_eq!(update.completion.seen_lines, 0);
    let mem = fx.store.0.lock().unwrap();
    let left: Vec<_> = mem
        .spans
        .iter()
        .flat_map(|(_, s)| s.base.iter().chain(&s.target))
        .map(|a| (a.side == Side::Base, a.range.start, a.range.end))
        .collect();
    assert_eq!(left, [(true, 10, 25), (false, 10, 25)], "the rest stays as recorded");
}

#[tokio::test]
async fn lines_lost_to_an_edit_come_back_after_unmarking_elsewhere_and_a_revert() {
    let region: Vec<usize> = (5..25).collect();
    let dirty = numbered(40, &region);
    let fx = Fixture::open(&[("a.txt", &numbered(40, &[]))], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    // Three lines in the middle go back to their base text: two hunks around them.
    let edited = dirty
        .replace("edited 14", "line 14")
        .replace("edited 15", "line 15")
        .replace("edited 16", "line 16");
    let rev = fx.settle(&[("a.txt", &edited)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Seen]);
    fx.session.set_seen(rev, "a.txt", &[0], false).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Unseen, HunkState::Seen]);

    // Reverting the edit: the second part is seen again, including the three lines.
    fx.settle(&[("a.txt", &dirty)]).await;
    let m = fx.session.manifest().await.unwrap();
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    // Base and target lines 14..25 are seen.
    assert_eq!((m.completion.lines, m.completion.seen_lines), (40, 22));
}

#[tokio::test]
async fn requests_for_an_old_revision_are_rejected() {
    let fx = Fixture::open(&[("a.txt", &numbered(5, &[]))], &[("a.txt", &numbered(5, &[1]))]).await;
    fx.settle(&[("a.txt", &numbered(5, &[2]))]).await;

    let err = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap_err();
    assert!(matches!(err, SessionError::Stale { current: 1 }));
    let err = fx.session.set_seen(1, "a.txt", &[5], true).await.unwrap_err();
    assert!(matches!(err, SessionError::NotFound(_)));
    assert!(fx.store.0.lock().unwrap().spans.is_empty());
}

#[tokio::test]
async fn a_mode_change_needs_acknowledging() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", "echo\n")]);
    let mut dirty = base.clone();
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let store = MemStore::default();
    let session = LiveSession::open(git, store.clone(), "t".into()).await.unwrap();

    let m = session.manifest().await.unwrap();
    assert_eq!((m.completion.items, m.completion.acknowledged_items), (1, 0));
    assert!(m.files[0].hunks.is_empty());

    let update = session.acknowledge(0, "run.sh", NonTextKind::Mode, true).await.unwrap();
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);
    assert!(update.changeset.is_complete());
    assert_eq!(store.0.lock().unwrap().acks.len(), 1);

    let update = session
        .acknowledge(0, "run.sh", NonTextKind::Mode, false)
        .await
        .unwrap();
    assert!(update.review.acknowledged.is_empty() && !update.changeset.is_complete());
    assert!(store.0.lock().unwrap().acks.is_empty());
}

#[tokio::test]
async fn items_of_one_file_are_acknowledged_separately() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("x.bin", "a\0\n")]);
    let mut dirty = git.snapshot(&[("x.bin", "b\0\n")]);
    dirty.entries.get_mut(&path("x.bin")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), "t".into()).await.unwrap();

    let m = session.manifest().await.unwrap();
    assert_eq!(m.completion.items, 2);
    let update = session
        .acknowledge(0, "x.bin", NonTextKind::Binary, true)
        .await
        .unwrap();

    assert_eq!(update.review.acknowledged, [NonTextKind::Binary]);
    assert_eq!((update.changeset.items, update.changeset.acknowledged_items), (2, 1));
}

#[tokio::test]
async fn submodule_changes_carry_both_commits() {
    let git = FakeGit::default();
    let sub = |commit: &[u8]| FileEntry {
        blob: BlobId::of(commit),
        mode: 0o160000,
        kind: EntryKind::Submodule,
        unfiltered: false,
    };
    let tree = |commit: &[u8]| {
        let entries = [(path("lib"), sub(commit))].into();
        Snapshot {
            id: merkle::snapshot_id(&entries),
            entries,
        }
    };
    *git.base.lock().unwrap() = Some(tree(b"one"));
    git.scans.lock().unwrap().push(tree(b"two"));
    let session = LiveSession::open(git, MemStore::default(), "t".into()).await.unwrap();

    let m = session.manifest().await.unwrap();

    let item = &m.files[0].non_text[0];
    assert_eq!(item.kind, NonTextKind::Submodule);
    let commit = |blob: &[u8]| Some(format!("{:02x}", BlobId::of(blob).0.0[0]));
    assert_eq!(item.before.as_ref().unwrap().target, commit(b"one"));
    assert_eq!(item.after.as_ref().unwrap().target, commit(b"two"));
}

#[tokio::test]
async fn the_pinned_commit_is_held_while_the_session_is_open() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let held = |fx: &Fixture| fx.git.refs.lock().unwrap().clone();
    assert_eq!(
        held(&fx),
        [
            ("base".to_string(), CommitId("head".into())),
            ("target".to_string(), CommitId("head".into()))
        ]
    );

    fx.session.close(0).await.unwrap();

    assert!(held(&fx).is_empty());
    assert!(fx.store.0.lock().unwrap().closed);
}

fn base_of(m: &ManifestView) -> (&BaseChoice, &str, &str) {
    (&m.base.choice, &m.base.commit.0, &m.base.label)
}

#[tokio::test]
async fn setting_the_base_moves_the_ref_and_bumps_the_revision() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    let m = fx.session.manifest().await.unwrap();
    assert_eq!(base_of(&m), (&BaseChoice::Pin, "head", "pin"));
    assert_eq!(m.files[0].hunks.len(), 1);

    // The chosen commit already has the edit: nothing is left to review.
    fx.git.add_commit("c1", "c1-full", &[("a.txt", &numbered(10, &[3]))]);
    let (view, revision) = fx
        .session
        .set_base(BaseChoice::Commit {
            id: CommitId("c1".into()),
        })
        .await
        .unwrap();
    assert_eq!(revision, 1);
    let full = BaseChoice::Commit {
        id: CommitId("c1-full".into()),
    };
    assert_eq!(
        (&view.choice, view.commit.0.as_str(), view.label.as_str()),
        (&full, "c1-full", "commit")
    );
    let m = fx.session.manifest().await.unwrap();
    assert_eq!((m.revision, base_of(&m)), (1, (&full, "c1-full", "commit")));
    assert!(m.files.is_empty());
    let held = fx.git.refs.lock().unwrap().clone();
    assert!(held.contains(&("base".to_string(), CommitId("c1-full".into()))));
    assert!(held.contains(&("target".to_string(), CommitId("head".into()))));
    assert_eq!(fx.store.0.lock().unwrap().base.as_ref().unwrap().1, full);

    // Back to the pin; asking again changes nothing.
    let (view, revision) = fx.session.set_base(BaseChoice::Pin).await.unwrap();
    assert_eq!(
        (revision, view.commit.0.as_str(), view.label.as_str()),
        (2, "head", "pin")
    );
    assert_eq!(fx.session.set_base(BaseChoice::Pin).await.unwrap().1, 2);
    assert_eq!(fx.session.manifest().await.unwrap().files[0].hunks.len(), 1);
    assert_eq!(fx.store.0.lock().unwrap().base.as_ref().unwrap().1, BaseChoice::Pin);
    assert!(
        fx.git
            .refs
            .lock()
            .unwrap()
            .contains(&("base".to_string(), CommitId("head".into())))
    );
}

#[tokio::test]
async fn a_changed_base_makes_marks_seen_against_an_old_base_until_it_is_chosen_again() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3, 4]))]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    // The merge base lacks the second base line of the marked hunk.
    let other = base.replace("line 4\n", "");
    fx.git.add_commit("merge-base main", "mb", &[("a.txt", &other)]);
    let merge_base = BaseChoice::MergeBase {
        reference: "main".into(),
    };

    fx.session.set_base(merge_base.clone()).await.unwrap();
    // The target lines are as marked, the base lines are not all.
    assert_eq!(fx.hunks("a.txt").await, [HunkState::SeenAgainstOldBase]);
    let m = fx.session.manifest().await.unwrap();
    assert_eq!((m.completion.seen_lines, m.completion.old_base_lines), (1, 2));
    assert!(!m.completion.is_complete());

    // Choosing the pin again restores the coverage, and so does coming back.
    fx.session.set_base(BaseChoice::Pin).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
    fx.session.set_base(merge_base).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::SeenAgainstOldBase]);

    // Re-confirming marks the hunk against the new base.
    fx.session.set_seen(3, "a.txt", &[0], true).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
}

#[tokio::test]
async fn a_base_that_does_not_resolve_is_refused_and_changes_nothing() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;

    let err = fx
        .session
        .set_base(BaseChoice::Commit {
            id: CommitId("nope".into()),
        })
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "unknown commit nope");
    let err = fx
        .session
        .set_base(BaseChoice::MergeBase {
            reference: "dev".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "no common ancestor with dev");

    assert_eq!(fx.session.revision().await, 0);
    assert_eq!(fx.session.manifest().await.unwrap().base.choice, BaseChoice::Pin);
    assert!(fx.store.0.lock().unwrap().base.is_none());
}

#[tokio::test]
async fn file_text_returns_both_sides() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n"), ("new.txt", "n\n")]).await;

    let text = fx.session.file_text("a.txt").await.unwrap();
    assert_eq!(
        (text.base.as_deref(), text.target.as_deref()),
        (Some("one\n"), Some("two\n"))
    );
    let added = fx.session.file_text("new.txt").await.unwrap();
    assert_eq!((added.base, added.target.as_deref()), (None, Some("n\n")));
    assert!(matches!(
        fx.session.file_text("nope").await,
        Err(SessionError::NotFound(_))
    ));
}

fn listing(l: &DirListing) -> Vec<(&str, DirEntryKind, bool, bool)> {
    l.entries
        .iter()
        .map(|e| (e.name.as_str(), e.kind, e.changed, e.excluded))
        .collect()
}

#[tokio::test]
async fn directories_list_first_in_byte_order_with_changed_below() {
    use DirEntryKind::{Dir, File};
    let base = [
        ("src/a/deep/x.rs", "x\n"),
        ("src/a-b/y.rs", "y\n"),
        ("src/b.rs", "b\n"),
        ("Z.md", "z\n"),
        ("a.md", "a\n"),
    ];
    let mut dirty = base;
    dirty[0].1 = "changed\n";
    let fx = Fixture::open(&base, &dirty).await;

    let root = fx.session.list_directory("").await.unwrap();
    assert_eq!(
        listing(&root),
        [
            ("src", Dir, true, false),
            ("Z.md", File, false, false),
            ("a.md", File, false, false)
        ]
    );
    let src = fx.session.list_directory("src").await.unwrap();
    assert_eq!(
        listing(&src),
        [
            ("a", Dir, true, false),
            ("a-b", Dir, false, false),
            ("b.rs", File, false, false)
        ]
    );
    let deep = fx.session.list_directory("src/a/deep").await.unwrap();
    assert_eq!(listing(&deep), [("x.rs", File, true, false)]);
    assert_eq!(deep.revision, 0);
    assert!(matches!(
        fx.session.list_directory("src/nope").await,
        Err(SessionError::NotFound(_))
    ));
    assert!(matches!(
        fx.session.list_directory("a.md").await,
        Err(SessionError::NotFound(_))
    ));
}

#[tokio::test]
async fn deleted_and_added_files_flag_their_directories() {
    let fx = Fixture::open(
        &[("gone/f.txt", "f\n"), ("keep/k.txt", "k\n")],
        &[("keep/k.txt", "k\n"), ("new/n.txt", "n\n")],
    )
    .await;

    let root = fx.session.list_directory("").await.unwrap();
    // `gone` has no files left in the target, so it is not listed.
    assert_eq!(
        listing(&root),
        [
            ("keep", DirEntryKind::Dir, false, false),
            ("new", DirEntryKind::Dir, true, false)
        ]
    );
}

#[tokio::test]
async fn the_filter_excludes_files_and_directories_without_a_passing_file() {
    use DirEntryKind::{Dir, File};
    let files = [
        ("docs/a.md", "a\n"),
        ("mixed/b.md", "b\n"),
        ("mixed/c.rs", "c\n"),
        ("d.rs", "d\n"),
    ];
    let fx = Fixture::open(&files, &files).await;
    fx.session.state.lock().await.filter.include = vec!["*.rs".into()];

    let root = fx.session.list_directory("").await.unwrap();
    assert_eq!(
        listing(&root),
        [
            ("docs", Dir, false, true),
            ("mixed", Dir, false, false),
            ("d.rs", File, false, false)
        ]
    );
    let mixed = fx.session.list_directory("mixed").await.unwrap();
    assert_eq!(
        listing(&mixed),
        [("b.md", File, false, true), ("c.rs", File, false, false)]
    );
}

#[tokio::test]
async fn target_text_reads_unchanged_files_and_explains_missing_text() {
    let fx = Fixture::open(
        &[("a.txt", "one\n"), ("bin", "x\0y"), ("link", "a.txt")],
        &[("a.txt", "one\n"), ("bin", "x\0y"), ("link", "a.txt")],
    )
    .await;
    // The target is a plain file; make `link` a symlink entry.
    let mut st = fx.session.state.lock().await;
    st.target.entries.get_mut(&path("link")).unwrap().kind = EntryKind::Symlink;
    drop(st);

    let text = fx.session.target_text("a.txt").await.unwrap();
    assert_eq!((text.revision, text.content), (0, Ok("one\n".into())));
    assert_eq!(
        fx.session.target_text("bin").await.unwrap().content,
        Err(NoText::Binary)
    );
    assert_eq!(
        fx.session.target_text("link").await.unwrap().content,
        Err(NoText::NotAFile)
    );
    // Content no source has any more.
    fx.git.blobs.lock().unwrap().clear();
    fx.store.0.lock().unwrap().blobs.clear();
    assert_eq!(
        fx.session.target_text("a.txt").await.unwrap().content,
        Err(NoText::Unavailable)
    );
    assert!(matches!(
        fx.session.target_text("nope").await,
        Err(SessionError::NotFound(_))
    ));
}

#[tokio::test]
async fn settling_moves_the_listing_to_the_new_revision() {
    let fx = Fixture::open(&[("a/x.txt", "x\n")], &[("a/x.txt", "x\n")]).await;
    assert!(!fx.session.list_directory("a").await.unwrap().entries[0].changed);

    fx.settle(&[("a/x.txt", "y\n")]).await;

    let after = fx.session.list_directory("").await.unwrap();
    assert_eq!(after.revision, 1);
    assert!(after.entries[0].changed);
}

#[tokio::test]
async fn settled_snapshots_are_stored_and_bump_the_revision() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;

    assert_eq!(fx.settle(&[("a.txt", "three\n")]).await, 1);

    assert_eq!(fx.session.revision().await, 1);
    let mem = fx.store.0.lock().unwrap();
    assert_eq!(mem.timeline.len(), 2);
    assert!(mem.blobs.contains_key(&BlobId::of(b"three\n")));
}
