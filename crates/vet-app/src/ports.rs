//! Ports implemented by `vet-infra`. Only where there is a real second
//! implementation or a test seam. `LanguageServer`, `ServerLauncher` and
//! `Materializer` are owned by `vet-semantic`, which consumes them.

use std::path::Path;

use vet_core::content::{BlobId, FileDiff, RenameMap, RepoPath, Snapshot, SnapshotStatus};
use vet_core::platform::PlatformTraits;

use crate::acquisition::CommitId;

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

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Dirty {
    All,
    Paths(Vec<RepoPath>),
}

pub struct Scan {
    pub snapshot: Snapshot,
    pub status: SnapshotStatus,
}

#[derive(Debug)]
pub struct GitError(pub String);

/// Everything that touches git or hashes the worktree. The persisted
/// stat-to-hash cache, racy-mtime handling and git blob hashing are private to
/// the adapter: callers get correct snapshots, not "uncertain" markers.
pub trait GitRepo {
    fn head(&self) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// Establish correct snapshot identity, metadata first (§6.2).
    fn scan(&self, dirty: Dirty) -> impl Future<Output = Result<Scan, GitError>> + Send;
    /// Check out `commit` into `root` and overlay content-store blobs for
    /// everything git cannot reconstruct (§7.2).
    fn checkout(&self, commit: &CommitId, snapshot: &Snapshot, root: &Path) -> impl Future<Output = Result<(), GitError>> + Send;
    fn diff(&self, old: &Snapshot, new: &Snapshot) -> impl Future<Output = Result<(Vec<FileDiff>, RenameMap), GitError>> + Send;
}

#[derive(Debug)]
pub struct StoreError(pub String);

/// One store for content and review data: they share a SQLite file, and GC must
/// check references across sessions, spans and comments together. Review tables
/// join this trait when Review is implemented.
pub trait Store {
    fn put_blob(&self, bytes: &[u8]) -> impl Future<Output = Result<BlobId, StoreError>> + Send;
    fn blob(&self, id: BlobId) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;
    /// Merkle directory nodes are content-addressed here, so timeline snapshots
    /// share structure instead of storing full entry maps.
    fn put_tree_node(&self, bytes: &[u8]) -> impl Future<Output = Result<vet_core::content::Hash, StoreError>> + Send;
    fn tree_node(&self, id: vet_core::content::Hash) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;
}

pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: std::path::PathBuf,
}

pub trait ProcessRunner {
    fn run(&self, command: &Command) -> impl Future<Output = Result<i32, String>> + Send;
}

/// Implemented by `vet-infra::platform` (an enum over GitHub and ADO). Core
/// branches on the returned traits, never on which platform it is.
pub trait Platform {
    fn traits(&self) -> PlatformTraits;
}
