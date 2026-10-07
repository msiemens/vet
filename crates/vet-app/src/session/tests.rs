use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use vet_core::classify::{changes, classify};
use vet_core::content::{
    BlobId, EntryKind, FileDiff, FileEntry, Hunk, LineRange, NonTextKind, RepoPath, Snapshot, SnapshotDiff, SnapshotId,
    SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::merkle;
use vet_core::review::{
    AnchorState, CommentTarget, Location, Message, ReviewedSpan, Side, StaleReason, Strategy, Thread, ThreadId,
    ThreadStatus,
};

use super::*;
use crate::pin::{BaseChoice, CommitId, Pin, PinId};
use crate::ports::{BlobRead, BlobReader, Checkout, Differ, Dirty, History, Scanner, SessionRefs};
use crate::review::{Session, Version};

const REPO: &str = "/repo";

fn path(s: &str) -> RepoPath {
    RepoPath::from(s)
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
pub(crate) struct FakeGit {
    blobs: Arc<Mutex<HashMap<BlobId, Vec<u8>>>>,
    base: Arc<Mutex<Option<Snapshot>>>,
    scans: Arc<Mutex<Vec<Snapshot>>>,
    /// `<name>` of each held `refs/vet/sessions/<id>/<name>` and the commit.
    refs: Arc<Mutex<Vec<(String, CommitId)>>>,
    /// What revisions (`merge-base <ref>` for merge bases) resolve to, with the commit's tree.
    commits: Arc<Mutex<HashMap<String, (CommitId, Snapshot)>>>,
    /// Largest blob `read_blob_capped` returns.
    cap: Arc<Mutex<Option<usize>>>,
    /// Renames `diff` reports, `old -> new`.
    renames: Arc<Mutex<Vec<(RepoPath, RepoPath)>>>,
    user: Arc<Mutex<Option<String>>>,
    branches: Arc<Mutex<Vec<BranchInfo>>>,
    /// First-parent history of a tip, the tip first.
    history: Arc<Mutex<HashMap<CommitId, Vec<CommitId>>>>,
}

fn clock() -> i64 {
    NOW.load(Ordering::Relaxed)
}

static NOW: AtomicI64 = AtomicI64::new(0);

impl FakeGit {
    /// A local branch at `tip` whose first-parent history is `history` after it.
    fn add_branch(&self, name: &str, current: bool, tip: &str, history: &[&str]) {
        let chain: Vec<CommitId> = [tip].iter().chain(history).map(|c| CommitId((*c).into())).collect();
        self.history.lock().unwrap().insert(chain[0].clone(), chain.clone());
        let mut branches = self.branches.lock().unwrap();
        branches.push(BranchInfo {
            name: name.into(),
            commit: chain[0].clone(),
            current,
            default: false,
        });
        branches.sort_by(|a, b| a.name.cmp(&b.name));
    }

    fn add_commit(&self, spec: &str, id: &str, files: &[(&str, &str)]) {
        let snapshot = self.snapshot(files);
        self.commits
            .lock()
            .unwrap()
            .insert(spec.into(), (CommitId(id.into()), snapshot));
    }

    /// Git drops the object, as it may once no ref reaches its commit.
    fn forget(&self, text: &str) {
        self.blobs.lock().unwrap().remove(&BlobId::of(text.as_bytes()));
    }

    fn snapshot(&self, files: &[(&str, &str)]) -> Snapshot {
        let mut blobs = self.blobs.lock().unwrap();
        for (_, text) in files {
            blobs.insert(BlobId::of(text.as_bytes()), text.as_bytes().to_vec());
        }
        snapshot(files)
    }
}

impl Scanner for FakeGit {
    async fn scan(&self, _: Dirty) -> std::result::Result<Scan, GitError> {
        let snapshot = self.scans.lock().unwrap().remove(0);
        Ok(Scan {
            snapshot,
            status: SnapshotStatus::default(),
        })
    }
}

impl Checkout for FakeGit {
    async fn checkout<St: Store + Sync>(
        &self,
        _: &St,
        _: &CommitId,
        _: &Snapshot,
        _: &Path,
    ) -> std::result::Result<(), crate::ports::CheckoutError> {
        Ok(())
    }
}

impl BlobReader for FakeGit {
    async fn read_blob(&self, _: &RepoPath, blob: BlobId) -> std::result::Result<Option<Vec<u8>>, GitError> {
        Ok(self.blobs.lock().unwrap().get(&blob).cloned())
    }
    async fn read_blob_capped(&self, _: &RepoPath, blob: BlobId) -> std::result::Result<BlobRead, GitError> {
        let cap = *self.cap.lock().unwrap();
        Ok(match self.blobs.lock().unwrap().get(&blob) {
            Some(bytes) if cap.is_some_and(|cap| bytes.len() > cap) => BlobRead::TooLarge,
            Some(bytes) => BlobRead::Found(bytes.clone()),
            None => BlobRead::Missing,
        })
    }
    /// The first byte of the blob id, in hex.
    async fn submodule_commit(&self, blob: BlobId) -> std::result::Result<Option<String>, GitError> {
        Ok(Some(format!("{:02x}", blob.0.0[0])))
    }
}

impl History for FakeGit {
    async fn head(&self) -> std::result::Result<CommitId, GitError> {
        Ok(CommitId("head".into()))
    }
    async fn user_name(&self) -> Option<String> {
        self.user.lock().unwrap().clone()
    }
    async fn commit_snapshot(&self, commit: &CommitId) -> std::result::Result<Snapshot, GitError> {
        let known = self.commits.lock().unwrap();
        match known.values().find(|(c, _)| c == commit) {
            Some((_, snapshot)) => Ok(snapshot.clone()),
            None => Ok(self.base.lock().unwrap().clone().unwrap()),
        }
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
    async fn branch_tip(&self, name: &str) -> std::result::Result<CommitId, GitError> {
        self.resolve_commit(&format!("refs/heads/{name}")).await
    }
    async fn base_candidates(&self, _: usize) -> std::result::Result<crate::ports::BaseCandidates, GitError> {
        unreachable!()
    }
    async fn branches(&self) -> std::result::Result<Vec<BranchInfo>, GitError> {
        Ok(self.branches.lock().unwrap().clone())
    }
    async fn commit_info(&self, commit: &CommitId) -> std::result::Result<CommitInfo, GitError> {
        Ok(CommitInfo {
            id: commit.clone(),
            subject: format!("subject of {}", commit.0),
            author: "Ann".into(),
            time: 100,
        })
    }
    async fn first_parent_distance(
        &self,
        tip: &CommitId,
        commit: &CommitId,
        max: usize,
    ) -> std::result::Result<Option<usize>, GitError> {
        let history = self.history.lock().unwrap();
        let chain = history.get(tip).map(Vec::as_slice).unwrap_or_default();
        Ok(chain.iter().position(|c| c == commit).filter(|&n| n <= max))
    }
    async fn commits_behind(&self, tip: &CommitId, base: &CommitId, _: usize) -> std::result::Result<usize, GitError> {
        let history = self.history.lock().unwrap();
        let chain = history.get(tip).map(Vec::as_slice).unwrap_or_default();
        Ok(chain.iter().position(|c| c == base).unwrap_or(chain.len()))
    }
}

impl SessionRefs for FakeGit {
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
}

impl Differ for FakeGit {
    async fn diff<St: Store + Sync>(
        &self,
        _: &St,
        old: &Snapshot,
        new: &Snapshot,
    ) -> std::result::Result<SnapshotDiff, GitError> {
        let blobs: BTreeMap<_, _> = self.blobs.lock().unwrap().clone().into_iter().collect();
        let mut out = SnapshotDiff::default();
        out.renames.0.extend(self.renames.lock().unwrap().iter().cloned());
        for change in changes(old, new) {
            let classified = classify(&change, &blobs);
            out.non_text.extend(classified.non_text);
            if let Some(text) = classified.text {
                let utf8 = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).unwrap();
                out.files.push(FileDiff {
                    old_path: text.old.map(|(p, _)| p.clone()),
                    new_path: text.new.map(|(p, _)| p.clone()),
                    old_blob: text.old.map(|(_, b)| b),
                    new_blob: text.new.map(|(_, b)| b),
                    hunks: hunks(&utf8(text.before), &utf8(text.after)),
                });
            }
        }
        Ok(out)
    }
}

#[derive(Default)]
struct Mem {
    blobs: HashMap<BlobId, Vec<u8>>,
    snapshots: HashMap<SnapshotId, (Snapshot, SnapshotStatus)>,
    timeline: Vec<Version>,
    spans: Vec<(SpanId, ReviewedSpan)>,
    next_span: u64,
    acks: Vec<NonTextAck>,
    base: Option<(SnapshotId, BaseChoice, CommitId)>,
    closed: bool,
    /// Whether the review was complete when it was closed.
    complete: Option<bool>,
    threads: Vec<Thread>,
    filter: Option<ReviewFilter>,
    pin: Option<Pin>,
    /// The repository and changeset the session was opened with.
    opened: Option<(String, Changeset)>,
}

impl Mem {
    fn thread(&mut self, id: ThreadId) -> &mut Thread {
        self.threads.iter_mut().find(|t| t.id == id).expect("a stored thread")
    }
}

#[derive(Clone, Default)]
pub(crate) struct MemStore(Arc<Mutex<Mem>>);

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
    async fn put_snapshot(&self, snapshot: Snapshot, status: SnapshotStatus, _: i64) -> StoreResult<()> {
        self.0.lock().unwrap().snapshots.insert(snapshot.id, (snapshot, status));
        Ok(())
    }
    async fn snapshot(&self, id: SnapshotId) -> StoreResult<Option<(Snapshot, SnapshotStatus)>> {
        Ok(self.0.lock().unwrap().snapshots.get(&id).cloned())
    }
    async fn append_timeline(&self, _: SessionId, snapshot: SnapshotId, at: i64) -> StoreResult<()> {
        self.0.lock().unwrap().timeline.push(Version { snapshot, at });
        Ok(())
    }
    async fn timeline(&self, _: SessionId) -> StoreResult<Vec<Version>> {
        Ok(self.0.lock().unwrap().timeline.clone())
    }
}

impl ReviewStore for MemStore {
    async fn put_pin(&self, head: CommitId, tree: SnapshotId) -> StoreResult<Pin> {
        let pin = Pin {
            id: PinId(1),
            head,
            tree,
        };
        self.0.lock().unwrap().pin = Some(pin.clone());
        Ok(pin)
    }
    async fn pin(&self, _: PinId) -> StoreResult<Option<Pin>> {
        Ok(self.0.lock().unwrap().pin.clone())
    }
    async fn open_session(
        &self,
        repo: String,
        _: PinId,
        changeset: Changeset,
        choice: BaseChoice,
        commit: CommitId,
    ) -> StoreResult<SessionId> {
        let mut mem = self.0.lock().unwrap();
        *mem = Mem {
            blobs: std::mem::take(&mut mem.blobs),
            snapshots: std::mem::take(&mut mem.snapshots),
            pin: mem.pin.take(),
            base: Some((changeset.base, choice, commit)),
            opened: Some((repo, changeset)),
            filter: mem.filter.take(),
            ..Default::default()
        };
        Ok(SessionId(1))
    }
    async fn close_session(&self, _: SessionId, _: i64, complete: bool) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        mem.closed = true;
        mem.complete = Some(complete);
        Ok(())
    }
    async fn open_sessions(&self, repo: String) -> StoreResult<Vec<SessionId>> {
        let mem = self.0.lock().unwrap();
        let open = mem.opened.as_ref().is_some_and(|(r, _)| *r == repo) && !mem.closed;
        Ok(open.then_some(SessionId(1)).into_iter().collect())
    }
    async fn session(&self, id: SessionId) -> StoreResult<Option<Session>> {
        let mem = self.0.lock().unwrap();
        let Some((_, changeset)) = &mem.opened else {
            return Ok(None);
        };
        let (base, choice, commit) = mem.base.clone().expect("set when the session was opened");
        Ok(Some(Session {
            id,
            pin: PinId(1),
            changeset: Changeset {
                base,
                target: changeset.target,
            },
            base: choice,
            base_commit: commit,
            complete: mem.complete,
            spans: mem
                .spans
                .iter()
                .map(|(id, span)| RecordedSpan {
                    id: *id,
                    span: span.clone(),
                })
                .collect(),
            acks: mem.acks.clone(),
            threads: mem.threads.clone(),
            closed_at: mem.closed.then_some(0),
        }))
    }
    async fn filter(&self, _: String) -> StoreResult<ReviewFilter> {
        Ok(self.0.lock().unwrap().filter.clone().unwrap_or_default())
    }
    async fn set_filter(&self, _: String, filter: ReviewFilter) -> StoreResult<()> {
        self.0.lock().unwrap().filter = Some(filter);
        Ok(())
    }
    async fn set_base(&self, _: SessionId, base: SnapshotId, choice: BaseChoice, commit: CommitId) -> StoreResult<()> {
        self.0.lock().unwrap().base = Some((base, choice, commit));
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
    async fn add_thread(&self, _: SessionId, target: CommentTarget, opener: Message) -> StoreResult<ThreadId> {
        let mut mem = self.0.lock().unwrap();
        let id = ThreadId(mem.threads.len() as u64 + 1);
        mem.threads.push(Thread {
            id,
            target,
            messages: vec![opener],
            status: ThreadStatus::Open,
        });
        Ok(id)
    }
    async fn add_message(&self, thread: ThreadId, message: Message) -> StoreResult<()> {
        self.0.lock().unwrap().thread(thread).messages.push(message);
        Ok(())
    }
    async fn set_thread_status(&self, thread: ThreadId, status: ThreadStatus) -> StoreResult<()> {
        self.0.lock().unwrap().thread(thread).status = status;
        Ok(())
    }
    async fn set_thread_target(&self, thread: ThreadId, target: CommentTarget) -> StoreResult<()> {
        self.0.lock().unwrap().thread(thread).target = target;
        Ok(())
    }
    async fn delete_thread(&self, thread: ThreadId) -> StoreResult<()> {
        self.0.lock().unwrap().threads.retain(|t| t.id != thread);
        Ok(())
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

pub(crate) struct Fixture {
    pub(crate) git: FakeGit,
    store: MemStore,
    pub(crate) session: LiveSession<FakeGit, MemStore>,
}

impl Fixture {
    pub(crate) async fn open(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Self {
        let git = FakeGit::default();
        *git.base.lock().unwrap() = Some(git.snapshot(base));
        let first = git.snapshot(dirty);
        git.scans.lock().unwrap().push(first);
        let store = MemStore::default();
        let session = LiveSession::open(git.clone(), store.clone(), REPO.into(), clock)
            .await
            .unwrap();
        Self { git, store, session }
    }

    /// Like `open`, with the review started.
    async fn reviewing(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Self {
        let fx = Self::open(base, dirty).await;
        fx.begin().await;
        fx
    }

    async fn begin(&self) {
        self.session.begin(&mut *self.session.state.lock().await).await.unwrap();
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
async fn browsing_keeps_nothing_and_settling_does_not_start_a_review() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);

    fx.settle(&[("a.txt", "three\n")]).await;
    let m = fx.session.manifest().await.unwrap();

    assert_eq!((m.revision, m.lifecycle), (1, Lifecycle::Browsing));
    assert_eq!(m.files.len(), 1);
    let mem = fx.store.0.lock().unwrap();
    assert!(mem.opened.is_none() && mem.snapshots.is_empty() && mem.timeline.is_empty());
    assert!(mem.blobs.is_empty());
    assert!(fx.git.refs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_first_mark_starts_the_review_at_version_1_and_retains_dirty_content() {
    let a = numbered(10, &[]);
    let b = numbered(10, &[3]);
    let fx = Fixture::open(&[("a.txt", &a)], &[("a.txt", &b), ("new.txt", "n\n")]).await;

    // Un-marking is not a review action.
    let unmarked = fx.session.set_seen(0, "a.txt", &[0], false).await.unwrap();
    assert_eq!(
        (fx.session.lifecycle().await, unmarked.started),
        (Lifecycle::Browsing, None)
    );

    let marked = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert!(matches!(marked.started, Some(Lifecycle::Open { version: 1, .. })));
    let again = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(
        again.started, None,
        "only the request that starts the review reports it"
    );

    assert!(matches!(
        fx.session.manifest().await.unwrap().lifecycle,
        Lifecycle::Open { version: 1, .. }
    ));
    let mem = fx.store.0.lock().unwrap();
    assert!(mem.blobs.contains_key(&BlobId::of(b.as_bytes())));
    assert!(mem.blobs.contains_key(&BlobId::of(b"n\n")));
    assert!(
        !mem.blobs.contains_key(&BlobId::of(a.as_bytes())),
        "HEAD content stays in git"
    );
    assert_eq!(mem.snapshots.len(), 2);
    assert_eq!(mem.timeline.len(), 1);
    assert_eq!(mem.spans.len(), 1);
    assert_eq!(fx.git.refs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn acknowledging_and_a_new_thread_also_start_the_review() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", "echo\n")]);
    let mut dirty = base.clone();
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();
    session
        .acknowledge(0, "run.sh", NonTextKind::Mode, false)
        .await
        .unwrap();
    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);

    session.acknowledge(0, "run.sh", NonTextKind::Mode, true).await.unwrap();
    assert!(matches!(session.lifecycle().await, Lifecycle::Open { version: 1, .. }));

    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session
        .create_thread(0, NewThread::Changeset, "why".into(), 1)
        .await
        .unwrap();
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 1, .. }
    ));
}

#[tokio::test]
async fn each_settled_snapshot_of_an_open_review_is_the_next_version() {
    // The only test that moves the clock.
    NOW.store(100, Ordering::Relaxed);
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert_eq!(
        fx.session.lifecycle().await,
        Lifecycle::Open {
            version: 1,
            updated_at: 100
        }
    );
    NOW.store(900, Ordering::Relaxed);

    fx.settle(&[("a.txt", "three\n")]).await;
    fx.settle(&[("a.txt", "four\n")]).await;

    assert_eq!(
        fx.session.lifecycle().await,
        Lifecycle::Open {
            version: 3,
            updated_at: 900
        }
    );
    // A base or filter change is not a version.
    fx.session.set_filter(ReviewFilter::default()).await.unwrap();
    fx.git.add_commit("c1", "c1-full", &[("a.txt", "zero\n")]);
    fx.session
        .set_base(BaseChoice::Commit {
            id: CommitId("c1".into()),
        })
        .await
        .unwrap();
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 3, .. }
    ));
    assert_eq!(fx.store.0.lock().unwrap().timeline.len(), 3);
}

#[tokio::test]
async fn finishing_returns_to_browsing_and_a_later_action_starts_a_new_review() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert!(matches!(
        fx.session.finish_review().await,
        Err(SessionError::Invalid(_))
    ));
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    fx.settle(&[("a.txt", "three\n")]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Unseen]);

    // Unseen changes may be left: the review is recorded as not complete.
    assert!(!fx.session.finish_review().await.unwrap());

    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
    assert!(fx.git.refs.lock().unwrap().is_empty());
    {
        let mem = fx.store.0.lock().unwrap();
        assert_eq!((mem.closed, mem.complete), (true, Some(false)));
    }
    let m = fx.session.manifest().await.unwrap();
    assert_eq!((m.revision, m.lifecycle), (2, Lifecycle::Browsing));
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    assert_eq!(m.completion.seen_lines, 0);

    // Editing is browsing again; the next review action starts review number two.
    fx.settle(&[("a.txt", "four\n")]).await;
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
    fx.session.set_seen(3, "a.txt", &[0], true).await.unwrap();
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 1, .. }
    ));
    let mem = fx.store.0.lock().unwrap();
    assert_eq!((mem.closed, mem.timeline.len(), mem.spans.len()), (false, 1, 1));
}

#[tokio::test]
async fn a_complete_review_is_recorded_as_complete_when_finished() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    assert!(fx.session.finish_review().await.unwrap());
    assert_eq!(fx.store.0.lock().unwrap().complete, Some(true));
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
    let paths: Vec<_> = m.files.iter().map(|f| f.path.to_string_lossy()).collect();
    assert_eq!(paths, ["a.txt", "gone.txt", "new.txt"]);
    let status: Vec<_> = m.files.iter().map(|f| f.status).collect();
    assert_eq!(status, [FileStatus::Modified, FileStatus::Deleted, FileStatus::Added]);
    // Ten lines and a final line break: the editor shows an eleventh, empty one.
    assert_eq!((m.files[0].base_lines, m.files[0].target_lines), (11, 11));
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
async fn completion_counts_the_seen_hunks_of_a_file_and_of_the_changeset() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(
        &[("a.txt", &base), ("b.txt", &base)],
        &[("a.txt", &numbered(30, &[3, 20])), ("b.txt", &numbered(30, &[5]))],
    )
    .await;

    let m = fx.session.manifest().await.unwrap();
    assert_eq!((m.files[0].completion.hunks, m.files[0].completion.seen_hunks), (2, 0));
    assert_eq!((m.completion.hunks, m.completion.seen_hunks), (3, 0));

    let update = fx.session.set_seen(0, "a.txt", &[1], true).await.unwrap();
    assert_eq!((update.completion.hunks, update.completion.seen_hunks), (2, 1));
    assert!(!update.completion.is_complete());
    assert_eq!((update.changeset.hunks, update.changeset.seen_hunks), (3, 1));

    let update = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!((update.completion.hunks, update.completion.seen_hunks), (2, 2));
    assert!(update.completion.is_complete() && !update.changeset.is_complete());
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
    let session = LiveSession::open(git, store.clone(), REPO.into(), clock).await.unwrap();

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
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest().await.unwrap();
    assert_eq!(m.completion.items, 2);
    let update = session
        .acknowledge(0, "x.bin", NonTextKind::Binary, true)
        .await
        .unwrap();

    assert_eq!(update.review.acknowledged, [NonTextKind::Binary]);
    assert_eq!((update.changeset.items, update.changeset.acknowledged_items), (2, 1));
}

/// `run.sh` is edited and made executable: a hunk and a mode change.
async fn edited_script() -> (LiveSession<FakeGit, MemStore>, MemStore) {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", &numbered(10, &[]))]);
    let mut dirty = git.snapshot(&[("run.sh", &numbered(10, &[2, 7]))]);
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let store = MemStore::default();
    let session = LiveSession::open(git, store.clone(), REPO.into(), clock).await.unwrap();
    (session, store)
}

#[tokio::test]
async fn toggling_a_file_marks_all_of_it_and_toggling_again_takes_it_back() {
    let (session, store) = edited_script().await;

    // Partly seen is not seen: everything gets marked.
    session.set_seen(0, "run.sh", &[0], true).await.unwrap();
    let update = session.toggle_file_seen(0, "run.sh").await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Seen, HunkState::Seen]);
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);
    assert!(update.completion.is_complete() && update.changeset.is_complete());
    assert_eq!(store.0.lock().unwrap().acks.len(), 1);

    let update = session.toggle_file_seen(0, "run.sh").await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Unseen, HunkState::Unseen]);
    assert!(update.review.acknowledged.is_empty());
    assert!(!update.completion.is_complete());
    let mem = store.0.lock().unwrap();
    assert!(mem.spans.is_empty() && mem.acks.is_empty());
}

#[tokio::test]
async fn toggling_a_file_with_only_a_mode_change_acknowledges_it_and_starts_the_review() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", "echo\n")]);
    let mut dirty = base.clone();
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let update = session.toggle_file_seen(0, "run.sh").await.unwrap();

    assert!(matches!(update.started, Some(Lifecycle::Open { version: 1, .. })));
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);
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
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest().await.unwrap();

    let item = &m.files[0].non_text[0];
    assert_eq!(item.kind, NonTextKind::Submodule);
    let commit = |blob: &[u8]| Some(format!("{:02x}", BlobId::of(blob).0.0[0]));
    assert_eq!(item.before.as_ref().unwrap().target, commit(b"one"));
    assert_eq!(item.after.as_ref().unwrap().target, commit(b"two"));
}

#[tokio::test]
async fn the_commits_are_held_while_the_review_is_open() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let held = |fx: &Fixture| fx.git.refs.lock().unwrap().clone();
    assert!(held(&fx).is_empty());

    fx.begin().await;
    assert_eq!(
        held(&fx),
        [
            ("base".to_string(), CommitId("head".into())),
            ("target".to_string(), CommitId("head".into()))
        ]
    );

    fx.session.finish_review().await.unwrap();

    assert!(held(&fx).is_empty());
    assert!(fx.store.0.lock().unwrap().closed);
}

impl Fixture {
    /// Vet restarts: a new session on the same git and store, with the tree as
    /// `dirty` now.
    async fn reopen(&self, dirty: &[(&str, &str)]) -> LiveSession<FakeGit, MemStore> {
        let scan = self.git.snapshot(dirty);
        self.git.scans.lock().unwrap().push(scan);
        LiveSession::open(self.git.clone(), self.store.clone(), REPO.into(), clock)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn reopening_resumes_the_open_review_and_remaps_edits_made_while_closed() {
    let base = numbered(30, &[]);
    let dirty = numbered(30, &[3, 20]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    let range = LineRange { start: 20, end: 21 };
    let thread = NewThread::Range {
        side: Side::Target,
        path: path("a.txt"),
        range,
    };
    fx.session.create_thread(0, thread, "why".into(), 1).await.unwrap();
    let before = fx.session.manifest().await.unwrap();

    assert_eq!(before.files[0].review.hunks, [HunkState::Seen, HunkState::Unseen]);

    // Another hunk appeared while Vet was closed.
    let edited = dirty.replace("line 25", "changed 25");
    let session = fx.reopen(&[("a.txt", &edited)]).await;

    let after = session.manifest().await.unwrap();
    assert_eq!(after.revision, 1, "the tree on disk is a new settled snapshot");
    assert!(matches!(after.lifecycle, Lifecycle::Open { version: 2, .. }));
    assert_eq!(
        after.files[0].review.hunks,
        [HunkState::Seen, HunkState::Unseen, HunkState::Unseen]
    );
    let threads = session.threads().await.unwrap().threads;
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].messages[0].body, "why");
    assert_eq!(threads[0].location.as_ref().map(|l| l.range), Some(range));
    assert_eq!(fx.store.0.lock().unwrap().timeline.len(), 2);
}

#[tokio::test]
async fn resuming_an_unchanged_tree_does_not_settle_again() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    assert_eq!(session.revision().await, 0);
    assert!(matches!(session.lifecycle().await, Lifecycle::Open { version: 1, .. }));
    assert_eq!(fx.store.0.lock().unwrap().timeline.len(), 1);
}

#[tokio::test]
async fn reopening_without_a_review_browses_again() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.settle(&[("a.txt", "three\n")]).await;

    let session = fx.reopen(&[("a.txt", "three\n")]).await;

    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);
    assert_eq!(session.revision().await, 0);
}

#[tokio::test]
async fn a_resumed_session_uses_the_repository_filter_and_keeps_its_base_and_refs() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let filter = ReviewFilter {
        hide_binary: true,
        ..Default::default()
    };
    fx.session.set_filter(filter.clone()).await.unwrap();

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    let m = session.manifest().await.unwrap();
    assert_eq!((&m.filter, base_of(&m).0), (&filter, &detached_head()));
    assert_eq!(base_of(&m).1, "head");
    assert_eq!(fx.git.refs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_session_opens_with_the_repository_filter_and_keeps_it_across_reviews() {
    let git = FakeGit::default();
    *git.base.lock().unwrap() = Some(git.snapshot(&[("a.txt", "1\n"), ("docs/b.md", "1\n")]));
    let dirty = git.snapshot(&[("a.txt", "2\n"), ("docs/b.md", "2\n")]);
    git.scans.lock().unwrap().push(dirty);
    let filter = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };
    let store = MemStore::default();
    store.0.lock().unwrap().filter = Some(filter.clone());

    let session = LiveSession::open(git, store.clone(), REPO.into(), clock).await.unwrap();

    let browsing = session.manifest().await.unwrap();
    assert_eq!((&browsing.filter, browsing.files.len()), (&filter, 1));
    session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    session.finish_review().await.unwrap();
    assert_eq!(session.manifest().await.unwrap().filter, filter);
    assert_eq!(store.0.lock().unwrap().filter, Some(filter));
}

#[tokio::test]
async fn a_filter_set_while_browsing_is_stored_and_applies_to_the_review() {
    let fx = Fixture::open(&[("a.txt", "1\n")], &[("a.txt", "2\n")]).await;
    let filter = ReviewFilter {
        hide_binary: true,
        ..Default::default()
    };
    fx.session.set_filter(filter.clone()).await.unwrap();
    assert_eq!(fx.store.0.lock().unwrap().filter, Some(filter.clone()));

    fx.session.set_seen(1, "a.txt", &[0], true).await.unwrap();

    assert_eq!(fx.session.manifest().await.unwrap().filter, filter);
    assert_eq!(fx.store.0.lock().unwrap().filter, Some(filter));
}

#[tokio::test]
async fn a_resumed_review_keeps_its_base_commit_however_its_branch_moved() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.add_commit("refs/heads/main", "m1", &[("a.txt", "one\n")]);
    let choice = BaseChoice::Branch { name: "main".into() };
    fx.session.set_base(choice.clone()).await.unwrap();
    fx.begin().await;

    fx.git.add_commit("refs/heads/main", "m2", &[("a.txt", "one\n")]);
    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    let m = session.manifest().await.unwrap();
    assert_eq!((base_of(&m).0, base_of(&m).1), (&choice, "m1"));
}

#[tokio::test]
async fn a_review_whose_snapshots_are_gone_is_closed_and_browsing_starts() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.store.0.lock().unwrap().snapshots.clear();

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);
    assert!(fx.store.0.lock().unwrap().closed);
}

#[tokio::test]
async fn a_finished_review_is_not_resumed() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    fx.session.finish_review().await.unwrap();

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);
    assert_eq!(
        session.manifest().await.unwrap().files[0].review.hunks,
        [HunkState::Unseen]
    );
}

/// With no branches, the default base is `HEAD` itself.
fn detached_head() -> BaseChoice {
    BaseChoice::Commit {
        id: CommitId("head".into()),
    }
}

fn base_of(m: &ManifestView) -> (&BaseChoice, &str, &str) {
    (&m.base.choice, &m.base.commit.id.0, &m.base.name)
}

#[tokio::test]
async fn the_default_base_is_the_current_branchs_tip() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert_eq!(fx.session.manifest().await.unwrap().base.name, "head");

    let git = FakeGit::default();
    *git.base.lock().unwrap() = Some(git.snapshot(&[("a.txt", "one\n")]));
    let scan = git.snapshot(&[("a.txt", "two\n")]);
    git.scans.lock().unwrap().push(scan);
    git.add_branch("feature", true, "f3", &["f2", "f1"]);
    git.add_branch("main", false, "m1", &[]);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest().await.unwrap();
    assert_eq!(
        base_of(&m),
        (&BaseChoice::Branch { name: "feature".into() }, "f3", "feature")
    );
    assert_eq!(m.base.behind, None);
    assert_eq!(m.target.name, "Working tree");
    assert_eq!(m.target.branch.as_deref(), Some("feature"));
    assert_eq!(m.base.commit.subject, "subject of f3");
}

#[tokio::test]
async fn a_base_is_named_by_its_branch_tip_then_branch_relative_then_by_hash() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.add_branch("feature", true, "f2", &["f1", "base"]);
    fx.git.add_branch("main", false, "m2", &["base", "root"]);
    for (commit, name) in [
        ("m2", "main"),
        ("f1", "feature~1"),
        // On both histories: the current branch wins.
        ("base", "feature~2"),
        ("root", "main~2"),
        ("0123456789abcdef", "0123456"),
    ] {
        fx.git.add_commit(commit, commit, &[("a.txt", "one\n")]);
        fx.session
            .set_base(BaseChoice::Commit {
                id: CommitId(commit.into()),
            })
            .await
            .unwrap();
        assert_eq!(fx.session.manifest().await.unwrap().base.name, name, "{commit}");
    }
}

#[tokio::test]
async fn a_branch_base_counts_how_far_the_branch_moved_on_and_latest_catches_up() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.add_branch("main", true, "m1", &["m0"]);
    fx.git.add_commit("refs/heads/main", "m1", &[("a.txt", "one\n")]);
    let main = BaseChoice::Branch { name: "main".into() };
    fx.session.set_base(main.clone()).await.unwrap();
    assert_eq!(fx.session.manifest().await.unwrap().base.behind, None);

    // Two commits on main since: the base stays where it was.
    fx.git.add_branch("main", true, "m3", &["m2", "m1", "m0"]);
    fx.git.branches.lock().unwrap().retain(|b| b.commit.0 != "m1");
    let m = fx.session.manifest().await.unwrap();
    assert_eq!((base_of(&m), m.base.behind), ((&main, "m1", "main~2"), Some(2)));

    fx.git.add_commit("refs/heads/main", "m3", &[("a.txt", "one\n")]);
    let (view, _) = fx.session.set_base(main).await.unwrap();
    assert_eq!(
        (view.commit.id.0.as_str(), view.name.as_str(), view.behind),
        ("m3", "main", None)
    );
}

#[tokio::test]
async fn setting_the_base_moves_the_ref_and_bumps_the_revision() {
    let base = numbered(10, &[]);
    let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    let m = fx.session.manifest().await.unwrap();
    assert_eq!(base_of(&m), (&detached_head(), "head", "head"));
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
        (&view.choice, view.commit.id.0.as_str(), view.name.as_str()),
        (&full, "c1-full", "c1-full")
    );
    let m = fx.session.manifest().await.unwrap();
    assert_eq!((m.revision, base_of(&m)), (1, (&full, "c1-full", "c1-full")));
    assert!(m.files.is_empty());
    let held = fx.git.refs.lock().unwrap().clone();
    assert!(held.contains(&("base".to_string(), CommitId("c1-full".into()))));
    assert!(held.contains(&("target".to_string(), CommitId("head".into()))));
    assert_eq!(fx.store.0.lock().unwrap().base.as_ref().unwrap().1, full);

    // Back to where it was; asking again changes nothing.
    fx.git.add_commit("head", "head", &[("a.txt", &base)]);
    let back = || BaseChoice::Commit {
        id: CommitId("head".into()),
    };
    let (view, revision) = fx.session.set_base(back()).await.unwrap();
    assert_eq!((revision, view.commit.id.0.as_str()), (2, "head"));
    assert_eq!(fx.session.set_base(back()).await.unwrap().1, 2);
    assert_eq!(fx.session.manifest().await.unwrap().files[0].hunks.len(), 1);
    assert_eq!(fx.store.0.lock().unwrap().base.as_ref().unwrap().1, back());
    assert!(
        fx.git
            .refs
            .lock()
            .unwrap()
            .contains(&("base".to_string(), CommitId("head".into())))
    );
}

#[tokio::test]
async fn a_base_chosen_while_browsing_is_carried_into_the_review() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.add_commit("c1", "c1-full", &[("a.txt", "zero\n")]);
    let c1 = BaseChoice::Commit {
        id: CommitId("c1-full".into()),
    };
    fx.session
        .set_base(BaseChoice::Commit {
            id: CommitId("c1".into()),
        })
        .await
        .unwrap();
    assert!(
        fx.store.0.lock().unwrap().base.is_none(),
        "nothing is stored while browsing"
    );

    fx.session.set_seen(1, "a.txt", &[0], true).await.unwrap();

    let mem = fx.store.0.lock().unwrap();
    assert_eq!(
        mem.base.as_ref().map(|b| (b.1.clone(), b.2.clone())),
        Some((c1, CommitId("c1-full".into())))
    );
    assert!(
        fx.git
            .refs
            .lock()
            .unwrap()
            .contains(&("base".to_string(), CommitId("c1-full".into())))
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

    // Choosing the first base again restores the coverage, and so does coming back.
    fx.git.add_commit("head", "head", &[("a.txt", &base)]);
    fx.session.set_base(detached_head()).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
    fx.session.set_base(merge_base).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::SeenAgainstOldBase]);

    // Re-confirming marks the hunk against the new base.
    fx.session.set_seen(3, "a.txt", &[0], true).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
}

#[tokio::test]
async fn coverage_survives_git_dropping_the_old_base() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3, 4]))]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    let held = |fx: &Fixture| {
        fx.store
            .0
            .lock()
            .unwrap()
            .blobs
            .contains_key(&BlobId::of(base.as_bytes()))
    };
    assert!(!held(&fx), "git holds the base");

    fx.git
        .add_commit("c1", "c1-full", &[("a.txt", &base.replace("line 4\n", ""))]);
    let c1 = BaseChoice::Commit {
        id: CommitId("c1".into()),
    };
    fx.session.set_base(c1).await.unwrap();
    assert!(held(&fx), "copied when the base changes");

    fx.git.forget(&base);
    assert_eq!(fx.hunks("a.txt").await, [HunkState::SeenAgainstOldBase]);
}

#[tokio::test]
async fn the_content_of_base_side_threads_is_copied_when_the_base_changes() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    let thread = NewThread::Range {
        side: Side::Base,
        path: path("a.txt"),
        range: LineRange { start: 5, end: 7 },
    };
    fx.session.create_thread(0, thread, "why".into(), 1).await.unwrap();

    fx.git
        .add_commit("c1", "c1-full", &[("a.txt", &base.replace("line 0\n", ""))]);
    let c1 = BaseChoice::Commit {
        id: CommitId("c1".into()),
    };
    fx.session.set_base(c1).await.unwrap();

    let mem = fx.store.0.lock().unwrap();
    assert!(mem.blobs.contains_key(&BlobId::of(base.as_bytes())));
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

    let err = fx
        .session
        .set_base(BaseChoice::Branch { name: "gone".into() })
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "unknown branch gone");

    assert_eq!(fx.session.revision().await, 0);
    assert_eq!(fx.session.manifest().await.unwrap().base.choice, detached_head());
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
async fn setting_the_filter_persists_it_and_moves_the_counts() {
    let base = [("a.txt", "1\n"), ("docs/b.md", "1\n")];
    let dirty = [("a.txt", "2\n"), ("docs/b.md", "2\n")];
    let fx = Fixture::reviewing(&base, &dirty).await;
    let m = fx.session.manifest().await.unwrap();
    assert_eq!(
        (m.files.len(), m.completion.lines, m.completion.excluded_lines),
        (2, 4, 0)
    );

    let filter = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };
    assert_eq!(fx.session.set_filter(filter.clone()).await.unwrap(), 1);
    let m = fx.session.manifest().await.unwrap();
    assert_eq!((m.revision, m.filter.clone()), (1, filter.clone()));
    assert_eq!(
        (m.files.len(), m.completion.lines, m.completion.excluded_lines),
        (1, 2, 2)
    );
    assert_eq!(fx.store.0.lock().unwrap().filter, Some(filter.clone()));

    // The same filter again changes nothing.
    assert_eq!(fx.session.set_filter(filter).await.unwrap(), 1);

    // An invalid glob is refused and changes nothing.
    let bad = ReviewFilter {
        include: vec!["[".into()],
        ..Default::default()
    };
    let err = fx.session.set_filter(bad).await.unwrap_err();
    assert!(err.to_string().starts_with("invalid pattern \"[\""), "{err}");
    assert_eq!(fx.session.revision().await, 1);
}

#[tokio::test]
async fn filter_roots_are_taken_as_directories_without_slashes() {
    let files = [("a.txt", "1\n"), ("docs/b.md", "1\n")];
    let fx = Fixture::open(&files, &[("a.txt", "2\n"), ("docs/b.md", "2\n")]).await;
    let filter = ReviewFilter {
        roots: vec![path("/docs/"), path("/")],
        ..Default::default()
    };
    fx.session.set_filter(filter).await.unwrap();
    let m = fx.session.manifest().await.unwrap();
    assert_eq!(m.filter.roots, [path("docs")]);
    assert_eq!(m.files.len(), 1);
    assert_eq!(
        fx.store.0.lock().unwrap().filter.as_ref().unwrap().roots,
        [path("docs")]
    );
}

#[test]
fn only_common_image_formats_have_a_media_type() {
    let media = |p: &str| image_media_type(&path(p));
    assert_eq!(media("a/b.PNG"), Some("image/png"));
    assert_eq!(media("x.jpeg"), Some("image/jpeg"));
    assert_eq!(media("x.svg"), Some("image/svg+xml"));
    assert_eq!(media("x.txt"), None);
    assert_eq!(media("png"), None);
    assert_eq!(media("dir.png/file"), None);
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
    // Over the cap, and not retained by the store (as the scan would do).
    let big = "x".repeat(100);
    fx.git.snapshot(&[("big.txt", &big)]);
    let mut st = fx.session.state.lock().await;
    st.target.entries.insert(path("big.txt"), entry(&big, 0o100644));
    drop(st);
    *fx.git.cap.lock().unwrap() = Some(10);
    assert_eq!(
        fx.session.target_text("big.txt").await.unwrap().content,
        Err(NoText::Unavailable)
    );
    *fx.git.cap.lock().unwrap() = None;
    assert_eq!(fx.session.target_text("big.txt").await.unwrap().content, Ok(big));
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
async fn settled_snapshots_of_an_open_review_are_stored_and_bump_the_revision() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;

    assert_eq!(fx.settle(&[("a.txt", "three\n")]).await, 1);

    assert_eq!(fx.session.revision().await, 1);
    let mem = fx.store.0.lock().unwrap();
    assert_eq!(mem.timeline.len(), 2);
    assert!(mem.blobs.contains_key(&BlobId::of(b"three\n")));
}

fn on(file: &str, start: u32, end: u32) -> NewThread {
    NewThread::Range {
        side: Side::Target,
        path: path(file),
        range: LineRange { start, end },
    }
}

fn at(file: &str, start: u32, end: u32) -> Option<Location> {
    Some(Location {
        file: path(file),
        range: LineRange { start, end },
    })
}

impl Fixture {
    async fn thread(&self, target: NewThread) -> ThreadId {
        let revision = self.session.revision().await;
        let view = self
            .session
            .create_thread(revision, target, "why?".into(), 100)
            .await
            .unwrap();
        view.id
    }

    async fn view(&self, id: ThreadId) -> ThreadView {
        let all = self.session.threads().await.unwrap();
        all.threads.into_iter().find(|t| t.id == id).unwrap()
    }
}

const EXACT: AnchorState = AnchorState::Exact {
    whitespace_changed: false,
};

#[tokio::test]
async fn a_thread_follows_an_edit_above_it() {
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(30, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 10, 13)).await;
    let view = fx.view(id).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("a.txt", 10, 13)));

    fx.settle(&[("a.txt", &format!("new 1\nnew 2\n{dirty}"))]).await;

    let view = fx.view(id).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("a.txt", 12, 15)));
    assert_eq!(view.path, Some(path("a.txt")));
    let CommentTarget::Range(recorded) = view.target else {
        panic!()
    };
    assert_eq!(recorded.range, LineRange { start: 10, end: 13 }, "never rewritten");
}

fn on_base(file: &str, start: u32, end: u32) -> NewThread {
    NewThread::Range {
        side: Side::Base,
        path: path(file),
        range: LineRange { start, end },
    }
}

impl Fixture {
    /// The target line the card of a new thread is shown below.
    async fn card_line(&self, target: NewThread) -> Option<u32> {
        let id = self.thread(target).await;
        self.view(id).await.target_line
    }
}

#[tokio::test]
async fn a_cards_line_is_the_end_of_the_range_in_the_target_text() {
    // Base lines 3 and 4 became one line, at 3.
    let base = numbered(10, &[]);
    let dirty = base.replace("line 3\nline 4\n", "changed\n");
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;

    assert_eq!(fx.card_line(on("a.txt", 5, 8)).await, Some(7));
    assert_eq!(fx.card_line(on("a.txt", 3, 4)).await, Some(3));
    // Before the hunk, in it, and after it, from the base side.
    assert_eq!(fx.card_line(on_base("a.txt", 0, 2)).await, Some(1));
    assert_eq!(fx.card_line(on_base("a.txt", 4, 5)).await, Some(3));
    assert_eq!(fx.card_line(on_base("a.txt", 3, 5)).await, Some(3));
    assert_eq!(fx.card_line(on_base("a.txt", 7, 9)).await, Some(7));
}

#[tokio::test]
async fn a_line_without_a_thread_is_placed_in_the_target_like_a_cards() {
    let base = numbered(10, &[]);
    let dirty = base.replace("line 3\nline 4\n", "changed\n");
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    let revision = fx.session.revision().await;
    let at = |side, line| fx.session.line_in_target(revision, side, "a.txt", line);

    assert_eq!(at(Side::Target, 7).await.unwrap(), 7);
    assert_eq!(at(Side::Base, 0).await.unwrap(), 0);
    assert_eq!(at(Side::Base, 4).await.unwrap(), 3);
    assert_eq!(at(Side::Base, 8).await.unwrap(), 7);
    // Not past the last line with text.
    assert_eq!(at(Side::Base, 9).await.unwrap(), 8);
    assert_eq!(at(Side::Target, 40).await.unwrap(), 8);

    let stale = fx.session.line_in_target(revision + 1, Side::Base, "a.txt", 4).await;
    assert!(matches!(stale, Err(SessionError::Stale { .. })));
}

#[tokio::test]
async fn a_card_on_deleted_lines_sits_where_they_were() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", "line 0\nline 1\nline 2\n")]).await;
    // The deleted lines face the last line that is left, not one past it.
    assert_eq!(fx.card_line(on_base("a.txt", 4, 6)).await, Some(2));
    assert_eq!(fx.card_line(on_base("a.txt", 9, 10)).await, Some(2));
}

#[tokio::test]
async fn a_card_in_a_deleted_file_sits_on_the_first_line() {
    let fx = Fixture::open(
        &[("a.txt", &numbered(10, &[])), ("keep.txt", "x\n")],
        &[("keep.txt", "x\n")],
    )
    .await;
    assert_eq!(fx.card_line(on_base("a.txt", 6, 8)).await, Some(0));
}

#[tokio::test]
async fn a_card_in_an_unchanged_file_sits_below_its_own_line() {
    let fx = Fixture::open(
        &[("a.txt", &numbered(10, &[])), ("same.txt", &numbered(5, &[]))],
        &[("a.txt", &numbered(10, &[2])), ("same.txt", &numbered(5, &[]))],
    )
    .await;
    assert_eq!(fx.card_line(on("same.txt", 1, 4)).await, Some(3));
    assert_eq!(fx.card_line(on_base("same.txt", 1, 4)).await, Some(3));
    // A replaced line is one line either way.
    assert_eq!(fx.card_line(on_base("a.txt", 2, 3)).await, Some(2));
}

#[tokio::test]
async fn file_and_changeset_threads_have_no_card_line() {
    let fx = Fixture::open(&[("a.txt", "x\n")], &[("a.txt", "y\n")]).await;
    for target in [
        NewThread::File {
            side: Side::Target,
            path: path("a.txt"),
        },
        NewThread::Changeset,
    ] {
        assert_eq!(fx.card_line(target).await, None);
    }
}

#[tokio::test]
async fn a_line_ending_change_keeps_the_anchor_exact() {
    let dirty = numbered(10, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(10, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 2, 5)).await;

    fx.settle(&[("a.txt", &dirty.replace('\n', "\r\n"))]).await;

    let view = fx.view(id).await;
    assert_eq!(
        view.state,
        Some(AnchorState::Exact {
            whitespace_changed: true
        })
    );
    assert_eq!(view.location, at("a.txt", 2, 5));
}

#[tokio::test]
async fn an_edit_inside_the_range_is_inferred_until_confirmed() {
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(30, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 10, 13)).await;

    let rev = fx.settle(&[("a.txt", &numbered(30, &[3, 11]))]).await;

    let view = fx.view(id).await;
    let Some(AnchorState::Inferred { strategy, .. }) = view.state else {
        panic!("{:?}", view.state)
    };
    assert_eq!(
        (strategy, view.location),
        (Strategy::ContextWindow, at("a.txt", 10, 13))
    );

    // Confirming from an older revision is refused.
    let err = fx.session.confirm_anchor(rev - 1, id).await.unwrap_err();
    assert!(matches!(err, SessionError::Stale { .. }));
    let confirmed = fx.session.confirm_anchor(rev, id).await.unwrap();
    assert_eq!(
        (confirmed.state, confirmed.location),
        (Some(EXACT), at("a.txt", 10, 13))
    );
    let stored = fx.store.0.lock().unwrap().threads[0].target.clone();
    let CommentTarget::Range(anchor) = stored else { panic!() };
    assert_eq!(anchor.snapshot, fx.session.state.lock().await.target.id);
    // An exact anchor cannot be confirmed again.
    let err = fx.session.confirm_anchor(rev, id).await.unwrap_err();
    assert!(matches!(err, SessionError::Invalid(_)));
}

#[tokio::test]
async fn resolution_starts_from_the_recorded_anchor() {
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(30, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 10, 13)).await;

    fx.settle(&[("a.txt", &numbered(30, &[3, 11]))]).await;
    assert!(matches!(fx.view(id).await.state, Some(AnchorState::Inferred { .. })));
    // The edit is reverted: the recorded anchor matches again, exactly.
    fx.settle(&[("a.txt", &dirty)]).await;

    assert_eq!(fx.view(id).await.state, Some(EXACT));
}

#[tokio::test]
async fn duplicated_content_is_ambiguous_until_one_is_attached() {
    let block = "dup A\ndup B\ndup C\n";
    let before = format!("p1\np2\np3\n{block}q1\nq2\nq3\n");
    let fx = Fixture::open(&[("a.txt", "x\n")], &[("a.txt", &before)]).await;
    let id = fx.thread(on("a.txt", 3, 6)).await;

    let after = format!("x1\nx2\nx3\n{block}x4\nx5\nx6\nx7\nx8\n{block}y1\ny2\ny3\n");
    let rev = fx.settle(&[("a.txt", &after)]).await;

    let view = fx.view(id).await;
    assert_eq!(view.location, None);
    assert_eq!(
        view.state,
        Some(AnchorState::Ambiguous {
            candidates: vec![at("a.txt", 3, 6).unwrap(), at("a.txt", 11, 14).unwrap()]
        })
    );
    let err = fx.session.attach_anchor(rev, id, 2).await.unwrap_err();
    assert!(matches!(err, SessionError::NotFound(_)));

    let attached = fx.session.attach_anchor(rev, id, 1).await.unwrap();
    assert_eq!((attached.state, attached.location), (Some(EXACT), at("a.txt", 11, 14)));
}

#[tokio::test]
async fn moved_content_in_another_changed_file_is_inferred() {
    let a = numbered(10, &[]);
    let b: String = (0..10).map(|i| format!("b {i}\n")).collect();
    let block = "m1\nm2\nm3\n";
    let with_block = format!(
        "{}{block}{}",
        &a[..a.find("line 5").unwrap()],
        &a[a.find("line 5").unwrap()..]
    );
    let fx = Fixture::open(
        &[("a.txt", &a), ("b.txt", &b)],
        &[("a.txt", &with_block), ("b.txt", &b)],
    )
    .await;
    let id = fx.thread(on("a.txt", 5, 8)).await;

    let moved = format!(
        "{}{block}{}",
        &b[..b.find("b 7").unwrap()],
        &b[b.find("b 7").unwrap()..]
    );
    fx.settle(&[("a.txt", &a), ("b.txt", &moved)]).await;

    let view = fx.view(id).await;
    assert!(
        matches!(
            view.state,
            Some(AnchorState::Inferred {
                strategy: Strategy::CrossFileMove,
                ..
            })
        ),
        "{:?}",
        view.state
    );
    assert_eq!((view.path, view.location), (Some(path("b.txt")), at("b.txt", 7, 10)));
}

#[tokio::test]
async fn deleting_the_file_detaches_range_and_file_threads_but_not_the_changeset_thread() {
    let fx = Fixture::open(&[("a.txt", &numbered(10, &[]))], &[("a.txt", &numbered(10, &[1]))]).await;
    let range = fx.thread(on("a.txt", 2, 5)).await;
    let file = fx
        .thread(NewThread::File {
            side: Side::Target,
            path: path("a.txt"),
        })
        .await;
    let whole = fx.thread(NewThread::Changeset).await;

    fx.settle(&[("other.txt", "o\n")]).await;

    let gone = Some(AnchorState::Stale {
        reason: StaleReason::FileDeleted,
    });
    let view = fx.view(range).await;
    assert_eq!((view.state, view.location), (gone.clone(), None));
    assert_eq!(fx.view(file).await.state, gone);
    let whole = fx.view(whole).await;
    assert_eq!((whole.state, whole.path, whole.location), (None, None, None));
}

#[tokio::test]
async fn range_and_file_threads_follow_a_rename() {
    let text = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &text)], &[("a.txt", &text)]).await;
    let range = fx.thread(on("a.txt", 2, 5)).await;
    let file = fx
        .thread(NewThread::File {
            side: Side::Target,
            path: path("a.txt"),
        })
        .await;
    fx.git.renames.lock().unwrap().push((path("a.txt"), path("b.txt")));

    fx.settle(&[("b.txt", &text)]).await;

    let view = fx.view(range).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("b.txt", 2, 5)));
    let view = fx.view(file).await;
    assert_eq!((view.state, view.path), (Some(EXACT), Some(path("b.txt"))));
}

#[tokio::test]
async fn replies_status_and_deletion_are_persisted() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    *fx.git.user.lock().unwrap() = Some("Ada".into());
    let id = fx.thread(on("a.txt", 0, 1)).await;
    let other = fx.thread(NewThread::Changeset).await;

    let view = fx.session.reply(id, "because".into(), 200).await.unwrap();
    assert_eq!(view.messages.len(), 2);
    assert_eq!(view.messages[1].at, 200);
    let view = fx.session.set_thread_status(id, ThreadStatus::Resolved).await.unwrap();
    assert_eq!(view.status, ThreadStatus::Resolved);
    fx.session.delete_thread(other).await.unwrap();

    let stored = fx.store.0.lock().unwrap().threads.clone();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].status, ThreadStatus::Resolved);
    assert_eq!(
        stored[0].messages.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        ["why?", "because"]
    );
    assert_eq!(fx.session.threads().await.unwrap().threads.len(), 1);
    assert!(matches!(
        fx.session.reply(other, "x".into(), 1).await,
        Err(SessionError::NotFound(_))
    ));
    assert!(matches!(
        fx.session.reply(id, "  ".into(), 1).await,
        Err(SessionError::Invalid(_))
    ));
}

#[tokio::test]
async fn a_session_is_named_after_its_repository_directory() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert_eq!(fx.session.manifest().await.unwrap().name, "repo");
    assert_eq!(name_of_repo("/"), "/");
}

#[tokio::test]
async fn the_author_is_the_git_user_or_you() {
    let git = FakeGit::default();
    *git.base.lock().unwrap() = Some(git.snapshot(&[("a.txt", "one\n")]));
    let first = git.snapshot(&[("a.txt", "two\n")]);
    git.scans.lock().unwrap().push(first);
    *git.user.lock().unwrap() = Some("Ada".into());
    let named = LiveSession::open(git.clone(), MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();
    let anonymous = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;

    let view = named
        .create_thread(0, NewThread::Changeset, "hi".into(), 1)
        .await
        .unwrap();
    assert_eq!(view.messages[0].author, "Ada");
    let view = anonymous
        .session
        .create_thread(0, NewThread::Changeset, "hi".into(), 1)
        .await
        .unwrap();
    assert_eq!(view.messages[0].author, "You");
}

#[tokio::test]
async fn threads_need_text_inside_the_current_revision() {
    let fx = Fixture::open(&[("a.txt", "one\ntwo\n")], &[("a.txt", "one\ntwo\n"), ("bin", "x\0y")]).await;
    let new = |target| fx.session.create_thread(0, target, "x".into(), 1);

    assert!(matches!(new(on("a.txt", 1, 3)).await, Err(SessionError::Invalid(_))));
    assert!(matches!(new(on("bin", 0, 1)).await, Err(SessionError::NotText(_))));
    assert!(matches!(new(on("nope", 0, 1)).await, Err(SessionError::NotFound(_))));
    fx.settle(&[("a.txt", "one\n")]).await;
    assert!(matches!(new(on("a.txt", 0, 1)).await, Err(SessionError::Stale { .. })));
    assert!(fx.store.0.lock().unwrap().threads.is_empty());
}
