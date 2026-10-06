//! Ports implemented by `vet-infra`. Only where there is a real second
//! implementation or a test seam. `LanguageServer`, `ServerLauncher` and
//! `Materializer` are owned by `vet-semantic`, which consumes them.

use std::path::Path;

use vet_core::content::{BlobId, Changeset, Hash, RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::platform::{PlatformThread, PlatformTraits};
use vet_core::review::{CommentTarget, Message, ReviewedSpan, ThreadId, ThreadStatus};

use crate::execution::targets::RunTarget;
use crate::pin::{BaseChoice, CommitId, Pin, PinId};
use crate::review::{NonTextAck, Session, SessionId, SpanId};

/// Monotonic milliseconds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Millis(pub u64);

/// Settling's debounce needs deadlines, not just `now`, or fake-clock tests are
/// fake in name only.
pub trait Clock {
    fn now(&self) -> Millis;
    fn sleep_until(&self, deadline: Millis) -> impl Future<Output = ()> + Send;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WatchEvent {
    Dirty(Vec<RepoPath>),
    /// Overflow, dropped events, or a poll tick: reconcile everything (§6.4).
    MustRescan,
    /// The watcher fell back to polling; implies `MustRescan`.
    Degraded(Degraded),
}

/// Why watching is polling-only; the session status surfaces it (§4.1, §6.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Degraded {
    /// WSL worktree under `/mnt/*`: events do not cross 9P.
    MountedWindowsDrive,
    /// inotify watch limit exhausted.
    WatchLimit,
}

/// A latency optimization, never authority (§6.2).
pub trait Watcher {
    fn degraded(&self) -> Option<Degraded>;
    fn next(&mut self) -> impl Future<Output = WatchEvent> + Send;
}

/// Git's ignore rules, so watcher events for ignored paths (`target/`,
/// `node_modules/`) never reach settling. Tracked files are never ignored.
pub trait IgnoreRules {
    /// `paths` without the ignored ones.
    fn unignored(&self, paths: Vec<RepoPath>) -> impl Future<Output = Result<Vec<RepoPath>, GitError>> + Send;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Dirty {
    All,
    Paths(Vec<RepoPath>),
}

/// The result of [`GitRepo::read_blob_capped`].
#[derive(Debug, PartialEq, Eq)]
pub enum BlobRead {
    Found(Vec<u8>),
    TooLarge,
    Missing,
}

pub struct Scan {
    pub snapshot: Snapshot,
    pub status: SnapshotStatus,
}

#[derive(Debug)]
pub struct GitError(pub String);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BranchInfo {
    pub name: String,
    pub commit: CommitId,
    /// The checked-out branch.
    pub current: bool,
    /// The remote's default branch (`origin/HEAD`), if the repository has one.
    pub default: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommitInfo {
    pub id: CommitId,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct BaseCandidates {
    pub branches: Vec<BranchInfo>,
    pub commits: Vec<CommitInfo>,
}

#[derive(Debug)]
pub enum CheckoutError {
    /// Neither git nor the content store has these blobs.
    MissingBlobs(Vec<BlobId>),
    Git(GitError),
}

impl From<GitError> for CheckoutError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// Everything that touches git or hashes the worktree. The persisted
/// stat-to-hash cache, racy-mtime handling and git blob hashing are private to
/// the adapter: callers get correct snapshots, not "uncertain" markers.
pub trait GitRepo {
    fn head(&self) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// Establish correct snapshot identity, metadata first (§6.2).
    fn scan(&self, dirty: Dirty) -> impl Future<Output = Result<Scan, GitError>> + Send;
    /// Write every entry of `snapshot` into the empty directory `root`: each
    /// blob from `store`, else from the object database (`commit` is where git
    /// is expected to have the rest). Content is written as stored: no hooks,
    /// filters or smudging. Every blob is attempted; if any is unavailable the
    /// result is `MissingBlobs` and `root` holds a partial tree the caller must
    /// discard (§6.1, §7.2).
    fn checkout<S: Store + Sync>(
        &self,
        store: &S,
        commit: &CommitId,
        snapshot: &Snapshot,
        root: &Path,
    ) -> impl Future<Output = Result<(), CheckoutError>> + Send;
    /// The snapshot of a commit's tree.
    fn commit_snapshot(&self, commit: &CommitId) -> impl Future<Output = Result<Snapshot, GitError>> + Send;
    /// Content of `blob` as git would store it: from the object database, else
    /// from the worktree file at `path` if it still hashes to `blob`.
    fn read_blob(
        &self,
        path: &RepoPath,
        blob: BlobId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, GitError>> + Send;
    /// Like [`Self::read_blob`], but content over the capture cap is not read.
    fn read_blob_capped(
        &self,
        path: &RepoPath,
        blob: BlobId,
    ) -> impl Future<Output = Result<BlobRead, GitError>> + Send;
    /// The commit a submodule entry with this `BlobId` is pinned at, for display.
    fn submodule_commit(&self, blob: BlobId) -> impl Future<Output = Result<Option<String>, GitError>> + Send;
    /// The merge base of `reference` (a branch or any revision) and `HEAD`.
    /// The error is fit to show to a user.
    fn merge_base(&self, reference: &str) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// The commit `spec` (a full or abbreviated id, or a revision) names.
    /// The error is fit to show to a user.
    fn resolve_commit(&self, spec: &str) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// Local branches and up to `max_commits` commits reachable from `HEAD`, newest first.
    fn base_candidates(&self, max_commits: usize) -> impl Future<Output = Result<BaseCandidates, GitError>> + Send;
    /// Points `refs/vet/sessions/<session>/<name>` at `commit` so git keeps it
    /// (§6.1).
    fn set_session_ref(
        &self,
        session: SessionId,
        name: &str,
        commit: &CommitId,
    ) -> impl Future<Output = Result<(), GitError>> + Send;
    /// Deletes every `refs/vet/sessions/<session>/*`.
    fn delete_session_refs(&self, session: SessionId) -> impl Future<Output = Result<(), GitError>> + Send;
    /// Content comes from `store` first, then as in `read_blob`.
    fn diff<S: Store + Sync>(
        &self,
        store: &S,
        old: &Snapshot,
        new: &Snapshot,
    ) -> impl Future<Output = Result<SnapshotDiff, GitError>> + Send;
}

#[derive(Debug)]
pub struct StoreError(pub String);

/// Lets adapters use `?` on their own error types; `StoreError` must never
/// implement `Error` itself, or this overlaps `From<T> for T`.
impl<E: std::error::Error> From<E> for StoreError {
    fn from(e: E) -> Self {
        Self(e.to_string())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GcReport {
    pub sessions_deleted: usize,
    pub snapshots_deleted: usize,
    pub nodes_deleted: usize,
    pub blobs_deleted: usize,
}

/// Content and snapshots (§8). One implementation backs this and
/// [`ReviewStore`]: GC checks references across sessions, spans and comments.
pub trait Store {
    /// Content-addressed by `BlobId::of`; durable (fsynced, renamed) on return.
    fn put_blob(&self, bytes: Vec<u8>) -> impl Future<Output = Result<BlobId, StoreError>> + Send;
    fn blob(&self, id: BlobId) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;
    /// Stored as shared Merkle nodes: only nodes not yet present are written.
    fn put_snapshot(
        &self,
        snapshot: Snapshot,
        status: SnapshotStatus,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn snapshot(
        &self,
        id: SnapshotId,
    ) -> impl Future<Output = Result<Option<(Snapshot, SnapshotStatus)>, StoreError>> + Send;
    /// Append a settled snapshot to the session's edit timeline (§3).
    fn append_timeline(
        &self,
        session: SessionId,
        snapshot: SnapshotId,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn timeline(&self, session: SessionId) -> impl Future<Output = Result<Vec<SnapshotId>, StoreError>> + Send;
    /// Delete expired closed sessions, then snapshots and content nothing
    /// retained refers to (§8). `now` is unix seconds.
    fn gc(&self, now: i64) -> impl Future<Output = Result<GcReport, StoreError>> + Send;
}

/// Review state (§5.2). Spans are replaced as a set because remapping is
/// a partition.
pub trait ReviewStore {
    fn put_pin(&self, head: CommitId, tree: SnapshotId) -> impl Future<Output = Result<Pin, StoreError>> + Send;
    fn open_session(
        &self,
        pin: PinId,
        changeset: Changeset,
        filter: ReviewFilter,
    ) -> impl Future<Output = Result<SessionId, StoreError>> + Send;
    /// `now` is unix seconds.
    fn close_session(&self, id: SessionId, now: i64) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn open_sessions(&self) -> impl Future<Output = Result<Vec<SessionId>, StoreError>> + Send;
    fn session(&self, id: SessionId) -> impl Future<Output = Result<Option<Session>, StoreError>> + Send;
    fn set_filter(&self, id: SessionId, filter: ReviewFilter) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Moves the session's changeset to the base snapshot `base`, chosen as `choice`.
    fn set_base(
        &self,
        id: SessionId,
        base: SnapshotId,
        choice: BaseChoice,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Recorded spans are the user's marks in the coordinates they were made
    /// in. They are never remapped in place: callers remap them per changeset
    /// from these (so a reverted edit regains coverage) and keep the result
    /// only in memory. Marking adds spans; un-marking removes the recorded
    /// spans that cover the lines and adds back the uncovered remainder.
    /// Atomic; returns the ids of the added spans.
    fn edit_spans(
        &self,
        id: SessionId,
        remove: Vec<SpanId>,
        add: Vec<ReviewedSpan>,
    ) -> impl Future<Output = Result<Vec<SpanId>, StoreError>> + Send;
    fn acknowledge(&self, id: SessionId, ack: NonTextAck) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn unacknowledge(&self, id: SessionId, ack: NonTextAck) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn add_thread(
        &self,
        id: SessionId,
        target: CommentTarget,
        opener: Message,
    ) -> impl Future<Output = Result<ThreadId, StoreError>> + Send;
    fn add_message(&self, thread: ThreadId, message: Message) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn set_thread_status(
        &self,
        thread: ThreadId,
        status: ThreadStatus,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Re-anchor a thread (attach, confirm a moved anchor).
    fn set_thread_target(
        &self,
        thread: ThreadId,
        target: CommentTarget,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn set_thread_platform(
        &self,
        thread: ThreadId,
        platform: Option<PlatformThread>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn delete_thread(&self, thread: ThreadId) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Per-repo execution state (§7.5, §10); `repo` identifies the repository.
pub trait RunStore {
    /// User-edited and user-added targets, in insertion order.
    fn run_targets(&self, repo: String) -> impl Future<Output = Result<Vec<RunTarget>, StoreError>> + Send;
    /// Inserts, or replaces the target with the same id.
    fn put_run_target(&self, repo: String, target: RunTarget) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn delete_run_target(&self, repo: String, id: String) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Approving records the executor-relevant hash for `repo` and the stored
    /// snapshot of those files, so the next prompt can show what changed. The
    /// store keeps that snapshot's content (§8).
    fn record_run_approval(
        &self,
        repo: String,
        hash: Hash,
        baseline: SnapshotId,
        at: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn last_run_approval(
        &self,
        repo: String,
    ) -> impl Future<Output = Result<Option<(Hash, SnapshotId)>, StoreError>> + Send;
}

pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProcessEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// The exit code, or `None` if the process was killed by a signal. Always
    /// the last event.
    Exit(Option<i32>),
}

pub trait ProcessRunner {
    type Process: RunningProcess;
    fn spawn(&self, command: &Command) -> Result<Self::Process, String>;
}

pub trait RunningProcess {
    /// Output as it arrives, then `Exit`; after that it yields `Exit` again.
    fn next(&mut self) -> impl Future<Output = ProcessEvent> + Send;
    /// Asks the process to die; `Exit` follows.
    fn kill(&self);
}

/// Implemented by `vet-infra::platform` (an enum over GitHub and ADO). Core
/// branches on the returned traits, never on which platform it is.
pub trait Platform {
    fn traits(&self) -> PlatformTraits;
}
