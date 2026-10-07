//! Ports implemented by `vet-infra`. Only where there is a real second
//! implementation or a test seam. `LanguageServer`, `ServerLauncher` and
//! `Materializer` are owned by `vet-semantic`, which consumes them.

use std::path::{Path, PathBuf};

use vet_core::content::{BlobId, Changeset, RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::review::{CommentTarget, Message, ReviewedSpan, ThreadId, ThreadStatus};
use vet_semantic::ports::{BackendId, ServerConfig};

use crate::gc::{GcFacts, GcPlan};
use crate::pin::{BaseChoice, CommitId, Pin, PinId};
use crate::review::{NonTextAck, Session, SessionId, SpanId, Version};

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

/// The result of [`BlobReader::read_blob_capped`].
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

/// Hashes the worktree. The persisted stat-to-hash cache, racy-mtime handling
/// and git blob hashing are private to the adapter: callers get correct
/// snapshots, not "uncertain" markers.
pub trait Scanner {
    /// Establish correct snapshot identity, metadata first (§6.2).
    fn scan(&self, dirty: Dirty) -> impl Future<Output = Result<Scan, GitError>> + Send;
}

/// Writes a snapshot out as a directory.
pub trait Checkout {
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
}

/// The line diff of two snapshots.
pub trait Differ {
    /// Content comes from `store` first, then as in [`BlobReader::read_blob`].
    fn diff<S: Store + Sync>(
        &self,
        store: &S,
        old: &Snapshot,
        new: &Snapshot,
    ) -> impl Future<Output = Result<SnapshotDiff, GitError>> + Send;
}

/// The content of blobs git has.
pub trait BlobReader {
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
}

/// Commits, branches and the history between them.
pub trait History {
    fn head(&self) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// The configured `user.name`, if any.
    fn user_name(&self) -> impl Future<Output = Option<String>> + Send;
    /// The snapshot of a commit's tree.
    fn commit_snapshot(&self, commit: &CommitId) -> impl Future<Output = Result<Snapshot, GitError>> + Send;
    /// The merge base of `reference` (a branch or any revision) and `HEAD`.
    /// The error is fit to show to a user.
    fn merge_base(&self, reference: &str) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// The commit `spec` (a full or abbreviated id, or a revision) names.
    /// The error is fit to show to a user.
    fn resolve_commit(&self, spec: &str) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// The tip of the local branch `name`.
    fn branch_tip(&self, name: &str) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// Local branches and up to `max_commits` commits reachable from `HEAD`, newest first.
    fn base_candidates(&self, max_commits: usize) -> impl Future<Output = Result<BaseCandidates, GitError>> + Send;
    /// Local branches, by name.
    fn branches(&self) -> impl Future<Output = Result<Vec<BranchInfo>, GitError>> + Send;
    fn commit_info(&self, commit: &CommitId) -> impl Future<Output = Result<CommitInfo, GitError>> + Send;
    /// How many first-parent steps lead from `tip` back to `commit`; `None` if
    /// it is not on that history within `max` steps.
    fn first_parent_distance(
        &self,
        tip: &CommitId,
        commit: &CommitId,
        max: usize,
    ) -> impl Future<Output = Result<Option<usize>, GitError>> + Send;
    /// How many commits are reachable from `tip` but not from `base`, counted up to `max`.
    fn commits_behind(
        &self,
        tip: &CommitId,
        base: &CommitId,
        max: usize,
    ) -> impl Future<Output = Result<usize, GitError>> + Send;
}

/// The refs that keep a session's commits alive.
pub trait SessionRefs {
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
}

/// Everything a live session needs of git.
pub trait GitRepo: Scanner + Checkout + Differ + BlobReader + History + SessionRefs {}

impl<T: Scanner + Checkout + Differ + BlobReader + History + SessionRefs> GitRepo for T {}

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

/// Content and snapshots (§8). One implementation backs this, [`ReviewStore`]
/// and [`GcStore`]: GC checks references across sessions, spans and comments.
pub trait Store {
    /// Content-addressed by `BlobId::of`; durable (fsynced, renamed) on return.
    fn put_blob(&self, bytes: Vec<u8>) -> impl Future<Output = Result<BlobId, StoreError>> + Send;
    fn blob(&self, id: BlobId) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;
    /// Stored as shared Merkle nodes: only nodes not yet present are written.
    /// `at` is unix seconds; GC spares snapshots younger than its grace period.
    fn put_snapshot(
        &self,
        snapshot: Snapshot,
        status: SnapshotStatus,
        at: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn snapshot(
        &self,
        id: SnapshotId,
    ) -> impl Future<Output = Result<Option<(Snapshot, SnapshotStatus)>, StoreError>> + Send;
    /// Append a settled snapshot to the session's edit timeline (§3); `at` is
    /// unix seconds. The timeline's length is the review's version.
    fn append_timeline(
        &self,
        session: SessionId,
        snapshot: SnapshotId,
        at: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn timeline(&self, session: SessionId) -> impl Future<Output = Result<Vec<Version>, StoreError>> + Send;
}

/// The store's side of garbage collection (§8): it knows how to read what
/// refers to what and how to delete, `crate::gc` decides what to keep.
pub trait GcStore {
    /// Reads the facts, deletes what `decide` makes of them in the same
    /// transaction, then the tree nodes no snapshot reaches and the content no
    /// tree refers to. `now` is unix seconds.
    fn gc(
        &self,
        now: i64,
        decide: &(dyn Fn(&GcFacts) -> GcPlan + Send + Sync),
    ) -> impl Future<Output = Result<GcReport, StoreError>> + Send;
}

/// Review state (§5.2). Spans are replaced as a set because remapping is
/// a partition.
pub trait ReviewStore {
    fn put_pin(&self, head: CommitId, tree: SnapshotId) -> impl Future<Output = Result<Pin, StoreError>> + Send;
    fn pin(&self, id: PinId) -> impl Future<Output = Result<Option<Pin>, StoreError>> + Send;
    /// `repo` is the repository root the session reviews, which is how a later
    /// open finds it again. `base` is how `changeset.base` was chosen and
    /// `base_commit` the commit it is the tree of.
    fn open_session(
        &self,
        repo: String,
        pin: PinId,
        changeset: Changeset,
        base: BaseChoice,
        base_commit: CommitId,
    ) -> impl Future<Output = Result<SessionId, StoreError>> + Send;
    /// `now` is unix seconds; `complete` is whether the review was complete.
    fn close_session(
        &self,
        id: SessionId,
        now: i64,
        complete: bool,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// The open sessions of `repo`, oldest first.
    fn open_sessions(&self, repo: String) -> impl Future<Output = Result<Vec<SessionId>, StoreError>> + Send;
    fn session(&self, id: SessionId) -> impl Future<Output = Result<Option<Session>, StoreError>> + Send;
    /// The review filter of the repository at `repo`, a project setting that
    /// applies to every changeset of it; the default if none was stored.
    fn filter(&self, repo: String) -> impl Future<Output = Result<ReviewFilter, StoreError>> + Send;
    fn set_filter(&self, repo: String, filter: ReviewFilter) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Moves the session's changeset to the base snapshot `base`, the tree of
    /// `commit`, chosen as `choice`.
    fn set_base(
        &self,
        id: SessionId,
        base: SnapshotId,
        choice: BaseChoice,
        commit: CommitId,
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
    fn delete_thread(&self, thread: ThreadId) -> impl Future<Output = Result<(), StoreError>> + Send;
}

pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
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

/// How to start a language server, as found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ServerCommand {
    /// A path, or a name the OS finds. The arguments are the backend's own.
    pub program: String,
    /// The compiler the server must be told about because it brings none (the
    /// project's `tsserver.js` for TypeScript).
    pub compiler: Option<PathBuf>,
}

/// Where a language server was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServerOrigin {
    /// The path the user set.
    Custom,
    /// Part of the project: its Rust toolchain.
    Project,
    /// Installed into Vet's data directory.
    Installed,
    /// On `PATH`.
    Path,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FoundServer {
    pub command: ServerCommand,
    pub origin: ServerOrigin,
    /// If it is cheap to learn.
    pub version: Option<String>,
    /// Facts that shape its answers, e.g. which `tsserver` it uses.
    pub notes: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ServerState {
    Found(FoundServer),
    Missing {
        /// Fit to show: what was looked for, where, and what to do.
        reason: String,
    },
}

/// Finds and installs language servers (§7.1). Implemented by `vet-infra`.
pub trait ServerResolver {
    /// The server of `language` for the project at `project`; `custom` is the
    /// user's path for it, which wins over everything else.
    fn resolve(
        &self,
        language: &BackendId,
        project: &Path,
        custom: Option<&str>,
    ) -> impl Future<Output = ServerState> + Send;
    /// The server's name as a person knows it, e.g. "Pyright".
    fn display_name(&self, language: &BackendId) -> String;
    /// The version `install` puts in place; `None` if Vet cannot install the language's server.
    fn pinned(&self, language: &BackendId) -> Option<String>;
    /// Why `install` cannot run now (a tool it needs is missing), if so.
    fn install_blocker(&self, language: &BackendId) -> Option<String>;
    /// Installs the pinned server into Vet's data directory. `progress` gets the
    /// tool's output lines.
    fn install(
        &self,
        language: &BackendId,
        progress: &(dyn Fn(&str) + Send + Sync),
    ) -> impl Future<Output = Result<(), String>> + Send;
}

/// What the language server launcher asks for on every start.
pub trait ServerLocator {
    /// The server for `config`'s language; the error is the reason to show.
    fn locate(
        &self,
        config: &ServerConfig,
        project: &Path,
    ) -> impl Future<Output = Result<ServerCommand, String>> + Send;
}

/// The recently opened repositories, kept across launches.
pub trait RecentStore {
    /// The paths as recorded, most recent first.
    fn recent_projects(&self) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;
    /// Replaces the whole list; `paths` is most recent first.
    fn set_recent_projects(&self, paths: Vec<String>) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// The custom server path per language, kept across launches.
pub trait ServerSettings {
    fn custom_server_path(&self, language: String) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;
    /// `None` removes the setting.
    fn set_custom_server_path(
        &self,
        language: String,
        path: Option<String>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
