//! Ports implemented by `vet-infra`. Only where there is a real second
//! implementation or a test seam. `LanguageServer`, `ServerLauncher` and
//! `Materializer` are owned by `vet-semantic`, which consumes them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use vet_core::content::{BlobId, RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::review::{
    CommentTarget, Message, MessageId, PendingVote, PlatformThread, ReviewedSpan, ThreadId, ThreadStatus,
};
use vet_semantic::ports::{BackendId, ServerConfig};

use crate::changeset::{BaseChoice, ChangesetId, CommitId, Lens, Source, Version};
use crate::platform::{Credential, PlatformKind, PullRequestHead};
use crate::review::{ChangesetThread, NonTextAck, Review, ReviewId, ReviewSummary, SpanId, SpanOrigin};
use crate::trust::PullRequestTrust;

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
    /// A branch ref was written: a commit on any branch, or a fetch.
    RefsMoved,
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

#[derive(Clone, Debug)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BranchInfo {
    /// A local branch's name, or `<remote>/<branch>` for a remote-tracking one.
    pub name: String,
    /// A remote-tracking branch, as of the last fetch.
    pub remote: bool,
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

/// A commit with what the by-commit lens shows beyond [`CommitInfo`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommitDetail {
    pub info: CommitInfo,
    /// The message after its subject, without the blank line between.
    pub body: String,
    /// In order: the first is the one a commit's own change is measured against.
    pub parents: Vec<CommitId>,
}

/// What a merge commit's two parents merge to by themselves.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Remerged {
    /// The merged tree; the conflicts git could not resolve stay in the files as markers.
    pub snapshot: Snapshot,
    /// The content of the merged files that git does not hold: the snapshot's blobs are of it.
    pub contents: Vec<Vec<u8>>,
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
    ) -> impl Future<Output = Result<TextDiff, GitError>> + Send;
}

/// What a snapshot's `.gitattributes` files say.
pub trait Attributes {
    /// Of `paths`, those that the `.gitattributes` files of `snapshot` give `linguist-generated`:
    /// `true` if it is set, `false` if it is unset. A path the attribute does not reach is absent.
    /// The files are read from `store` first, then as in [`BlobReader::read_blob`].
    fn linguist_generated<S: Store + Sync>(
        &self,
        store: &S,
        snapshot: &Snapshot,
        paths: Vec<RepoPath>,
    ) -> impl Future<Output = Result<BTreeMap<RepoPath, bool>, GitError>> + Send;
}

/// A diff of two snapshots and the content it compared.
#[derive(Clone, Debug, Default)]
pub struct TextDiff {
    pub diff: SnapshotDiff,
    /// The content of both sides of every file in `diff.files` that could be read.
    pub texts: BTreeMap<BlobId, Vec<u8>>,
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
    /// The merge base of `reference` (a branch or any revision) and `tip`.
    /// The error is fit to show to a user.
    fn merge_base(&self, reference: &str, tip: &CommitId) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// What a changeset of a branch is compared against by default: the remote's default
    /// branch (`origin/HEAD`) as a revision, else the default local branch; `None` if
    /// the repository has neither.
    fn default_reference(&self) -> impl Future<Output = Result<Option<String>, GitError>> + Send;
    /// The name of the repository's default branch (`main`), as a pull request names its target:
    /// the one `origin/HEAD` points at, else the local `main` or `master`.
    fn default_branch(&self) -> impl Future<Output = Result<Option<String>, GitError>> + Send;
    /// The commits reachable from `tip` but not from `base` along first parents only,
    /// oldest first.
    fn first_parent_range(
        &self,
        base: &CommitId,
        tip: &CommitId,
    ) -> impl Future<Output = Result<Vec<CommitId>, GitError>> + Send;
    /// The commit `spec` (a full or abbreviated id, or a revision) names.
    /// The error is fit to show to a user.
    fn resolve_commit(&self, spec: &str) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// The tip of the local branch `name`, else of the remote-tracking branch of that name.
    fn branch_tip(&self, name: &str) -> impl Future<Output = Result<CommitId, GitError>> + Send;
    /// The local branches by name, then the remote-tracking ones that no local branch
    /// follows at the same commit, by name.
    fn branches(&self) -> impl Future<Output = Result<Vec<BranchInfo>, GitError>> + Send;
    fn commit_info(&self, commit: &CommitId) -> impl Future<Output = Result<CommitInfo, GitError>> + Send;
    fn commit_detail(&self, commit: &CommitId) -> impl Future<Output = Result<CommitDetail, GitError>> + Send;
    /// The automatic merge of the two parents of `merge`, as `git show --remerge-diff` computes
    /// it, and `None` unless `merge` has exactly two parents. Parents with no common ancestor
    /// merge as if the ancestor were empty. Writes nothing to the repository.
    fn remerge(&self, merge: &CommitId) -> impl Future<Output = Result<Option<Remerged>, GitError>> + Send;
    /// How many commits are reachable from `tip` but not from `base`, counted up to `max`.
    fn commits_behind(
        &self,
        tip: &CommitId,
        base: &CommitId,
        max: usize,
    ) -> impl Future<Output = Result<usize, GitError>> + Send;
}

/// A remote a repository fetches from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RemoteInfo {
    pub name: String,
    /// Fit to show.
    pub url: String,
    /// `host[:port]`, lower case, when the remote is fetched over HTTPS: what an account is held under.
    pub https_host: Option<String>,
}

/// The credential to present to the host of an HTTPS remote.
#[derive(Clone, Debug)]
pub struct FetchAuth {
    pub host: String,
    pub kind: PlatformKind,
    pub credential: Credential,
}

/// A ref of the repository and the commit it points to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RefTip {
    /// Full name.
    pub name: String,
    /// The hex id of the commit.
    pub commit: String,
}

/// A ref of a remote and the ref of the repository it is kept under.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RefUpdate {
    /// Full name on the remote, e.g. `refs/pull/7/head`.
    pub source: String,
    /// Full name here; below `refs/vet/`.
    pub destination: String,
}

/// Brings in what remotes have. Read-only: it writes objects and remote-tracking
/// refs and touches no other ref, the index or the worktree; of what it fetched below
/// `refs/vet/pr/`, it deletes what is no longer needed.
pub trait Fetcher {
    /// The remotes that have a fetch URL.
    fn remotes(&self) -> impl Future<Output = Result<Vec<RemoteInfo>, GitError>> + Send;
    /// Fetches `remote` as its refspecs say, authenticating with `auth` where the host matches;
    /// otherwise with git's credential helpers, else anonymously. Never prompts. Answers with how
    /// many refs were updated. The error is fit to show.
    fn fetch(&self, remote: &str, auth: Option<FetchAuth>) -> impl Future<Output = Result<usize, GitError>> + Send;
    /// Fetches exactly the refs of `updates` from `remote` and moves the refs below `refs/vet/`
    /// they name, even when that is not a fast-forward (a force-pushed branch). Authenticates as
    /// `fetch` does. Fails, fetching nothing, if a destination is not below `refs/vet/`. A ref the
    /// remote lacks fails the fetch. The error is fit to show.
    fn fetch_refs(
        &self,
        remote: &str,
        updates: Vec<RefUpdate>,
        auth: Option<FetchAuth>,
    ) -> impl Future<Output = Result<(), GitError>> + Send;
    /// The refs whose names start with `prefix`, each with the commit it points to.
    fn refs_below(&self, prefix: &str) -> impl Future<Output = Result<Vec<RefTip>, GitError>> + Send;
    /// Deletes the refs `names`, which must all be below `refs/vet/pr/`, where the refs of fetched
    /// pull requests are kept. Fails, deleting nothing, if one is not; a ref that is not there is
    /// left out.
    fn delete_refs(&self, names: Vec<String>) -> impl Future<Output = Result<(), GitError>> + Send;
}

/// The refs that keep a changeset's commits alive.
pub trait ChangesetRefs {
    /// Points `refs/vet/changesets/<changeset>/<version>/<name>` at `commit` so
    /// git keeps it (§6.1).
    fn set_changeset_ref(
        &self,
        changeset: ChangesetId,
        version: u32,
        name: &str,
        commit: &CommitId,
    ) -> impl Future<Output = Result<(), GitError>> + Send;
    /// Deletes every `refs/vet/changesets/<changeset>/*`.
    fn delete_changeset_refs(&self, changeset: ChangesetId) -> impl Future<Output = Result<(), GitError>> + Send;
}

/// Everything a live session needs of git.
pub trait GitRepo: Scanner + Checkout + Differ + Attributes + BlobReader + History + ChangesetRefs {}

impl<T: Scanner + Checkout + Differ + Attributes + BlobReader + History + ChangesetRefs> GitRepo for T {}

#[derive(Clone, Debug)]
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
    pub changesets_deleted: usize,
    pub snapshots_deleted: usize,
    pub nodes_deleted: usize,
    pub blobs_deleted: usize,
}

/// A changeset and everything it keeps alive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangesetFacts {
    pub id: ChangesetId,
    pub has_open_review: bool,
    /// Unix seconds the latest closed review ended; `None` if none has.
    pub last_closed_at: Option<i64>,
    pub has_open_thread: bool,
    /// A vote of the viewer's waits to be sent.
    pub has_pending_vote: bool,
    /// The bases and targets of its versions, and the snapshots its anchors name.
    pub snapshots: Vec<SnapshotId>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GcFacts {
    pub changesets: Vec<ChangesetFacts>,
    /// Every stored snapshot and when it was stored (unix seconds).
    pub snapshots: Vec<(SnapshotId, i64)>,
}

/// What to delete. Tree nodes and content follow from what remains.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GcPlan {
    pub changesets: Vec<ChangesetId>,
    pub snapshots: Vec<SnapshotId>,
    /// Unreferenced content younger than this stays.
    pub content_grace: Duration,
}

/// Content and snapshots (§8), as a reader has them. One implementation backs this,
/// [`ChangesetStore`], [`ReviewStore`], [`ThreadStore`] (which only read), [`ReviewWrites`] and
/// [`GcStore`]: GC checks references across changesets, spans and comments. Content is
/// addressed by its hash, so putting it changes no review state.
pub trait Store {
    /// Content-addressed by `BlobId::of`, answered with the ids in the order of `blobs`;
    /// all durable (fsynced, renamed) on return. Blobs the store holds are not written again.
    fn put_blobs(&self, blobs: Vec<Vec<u8>>) -> impl Future<Output = Result<Vec<BlobId>, StoreError>> + Send;
    /// Of `ids`, those the store does not hold durably, without reading any.
    fn missing_blobs(&self, ids: Vec<BlobId>) -> impl Future<Output = Result<Vec<BlobId>, StoreError>> + Send;
    fn blob(&self, id: BlobId) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;
    /// The content of each of `ids` the store holds, read in one go; the others are left out.
    fn blobs(&self, ids: Vec<BlobId>) -> impl Future<Output = Result<BTreeMap<BlobId, Vec<u8>>, StoreError>> + Send;
    fn snapshot(
        &self,
        id: SnapshotId,
    ) -> impl Future<Output = Result<Option<(Snapshot, SnapshotStatus)>, StoreError>> + Send;
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

/// Changesets and their versions (§3). A changeset is found by the repository
/// root and its source.
pub trait ChangesetStore {
    /// The changeset of `source` in the repository at `repo`, if it was stored.
    fn changeset(
        &self,
        repo: String,
        source: Source,
    ) -> impl Future<Output = Result<Option<ChangesetId>, StoreError>> + Send;
    /// The lens the changeset of `source` was last put in; `None` if it never was.
    fn lens(&self, repo: String, source: Source) -> impl Future<Output = Result<Option<Lens>, StoreError>> + Send;
    /// How the platform last named the pull request of `source`; `None` if it was never stored.
    fn pull_request_head(
        &self,
        repo: String,
        source: Source,
    ) -> impl Future<Output = Result<Option<PullRequestHead>, StoreError>> + Send;
    /// Where the pull request of `source` is from and the head the viewer trusted it at; `None` if
    /// the platform's data on it was never stored.
    fn pull_request_trust(
        &self,
        repo: String,
        source: Source,
    ) -> impl Future<Output = Result<Option<PullRequestTrust>, StoreError>> + Send;
    /// The commit the working tree is compared against while no review of it is open, pinned when
    /// the session started browsing; `None` if none is.
    fn pinned_base(
        &self,
        repo: String,
        source: Source,
    ) -> impl Future<Output = Result<Option<CommitId>, StoreError>> + Send;
    /// The vote of the viewer's on the pull request of `source` that waits to be sent; `None` if
    /// there is none.
    fn pending_vote(
        &self,
        repo: String,
        source: Source,
    ) -> impl Future<Output = Result<Option<PendingVote>, StoreError>> + Send;
    /// The changesets a review of which is open, in any repository.
    fn open_changesets(&self) -> impl Future<Output = Result<Vec<ChangesetId>, StoreError>> + Send;
    /// The versions in order of their numbers.
    fn versions(&self, changeset: ChangesetId) -> impl Future<Output = Result<Vec<Version>, StoreError>> + Send;
}

/// Review state (§5.2), as a reader has it. Spans are replaced as a set because remapping is
/// a partition.
pub trait ReviewStore {
    /// The review of `changeset` that is open, if any; the newest if several are.
    fn open_review(&self, changeset: ChangesetId) -> impl Future<Output = Result<Option<ReviewId>, StoreError>> + Send;
    /// The newest review of `changeset`, open or finished.
    fn latest_review(
        &self,
        changeset: ChangesetId,
    ) -> impl Future<Output = Result<Option<ReviewId>, StoreError>> + Send;
    fn review(&self, id: ReviewId) -> impl Future<Output = Result<Option<Review>, StoreError>> + Send;
    /// The review filter of the repository at `repo`, a project setting that
    /// applies to every changeset of it; the default if none was stored.
    fn filter(&self, repo: String) -> impl Future<Output = Result<ReviewFilter, StoreError>> + Send;
    /// Whether the repository at `repo` is marked trusted (§7.5), a project setting; it is not
    /// unless it was.
    fn project_trusted(&self, repo: String) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

/// The threads of a changeset, which outlive any review of it, as a reader has them.
pub trait ThreadStore {
    /// Every thread of `changeset`, oldest first.
    fn threads(&self, changeset: ChangesetId) -> impl Future<Output = Result<Vec<ChangesetThread>, StoreError>> + Send;
}

/// Every change to the review database: the one write connection of the store. Writes queue in the
/// order they are asked for. Only a review command has it (the write token of a session), so that
/// what reads the store cannot change it.
pub trait ReviewWrites {
    /// Stored as shared Merkle nodes: only nodes not yet present are written.
    /// `at` is unix seconds; GC spares snapshots younger than its grace period.
    fn put_snapshot(
        &self,
        snapshot: Snapshot,
        status: SnapshotStatus,
        at: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Remembers the lens of the changeset of `source`, storing the changeset if need be.
    fn set_lens(&self, repo: String, source: Source, lens: Lens)
    -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Remembers how the platform names the pull request of `source`, storing the changeset if
    /// need be.
    fn set_pull_request_head(
        &self,
        repo: String,
        source: Source,
        head: PullRequestHead,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Pins the commit the working tree of `source` is compared against while no review of it is
    /// open, or with `None` lets the next session pin its own. The changeset is stored if need be.
    fn set_pinned_base(
        &self,
        repo: String,
        source: Source,
        base: Option<CommitId>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Replaces the vote of the viewer's that waits to be sent on the pull request of `source`;
    /// `None` clears it. The changeset is stored if need be.
    fn set_pending_vote(
        &self,
        repo: String,
        source: Source,
        vote: Option<PendingVote>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// The changeset of `source` in the repository at `repo`, stored if it was not: threads
    /// that arrive from a platform belong to it before any review of it started.
    fn ensure_changeset(
        &self,
        repo: String,
        source: Source,
    ) -> impl Future<Output = Result<ChangesetId, StoreError>> + Send;
    /// Adds the next version, numbered after the last.
    fn append_version(
        &self,
        changeset: ChangesetId,
        version: Version,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Opens a review of the changeset of `source` in the repository at `repo`,
    /// storing the changeset if need be, with `base` as the chosen base rule.
    /// `first` replaces the changeset's versions: a working tree is versioned
    /// only while a review of it is open, so a new review starts over. `now` is
    /// unix seconds.
    fn start_review(
        &self,
        repo: String,
        source: Source,
        base: BaseChoice,
        first: Version,
        now: i64,
    ) -> impl Future<Output = Result<(ChangesetId, ReviewId), StoreError>> + Send;
    /// `now` is unix seconds; `complete` is whether the review was complete.
    fn close_review(
        &self,
        id: ReviewId,
        now: i64,
        complete: bool,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn set_filter(&self, repo: String, filter: ReviewFilter) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn set_project_trusted(&self, repo: String, trusted: bool) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Records the head commit the pull request of `source` is trusted at, or with `None` that it
    /// is trusted at none. The changeset is stored if need be.
    fn set_pull_request_trusted_head(
        &self,
        repo: String,
        source: Source,
        head: Option<CommitId>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Records the number of the version the viewer last marked something in.
    fn set_last_reviewed(&self, id: ReviewId, version: u32) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Records how far the review is, for a list of changesets to show without remapping its marks.
    fn set_review_summary(
        &self,
        id: ReviewId,
        summary: ReviewSummary,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Recorded spans are the user's marks in the coordinates they were made
    /// in. They are never remapped in place: callers remap them per changeset
    /// from these (so a reverted edit regains coverage) and keep the result
    /// only in memory. Marking adds spans; un-marking removes the recorded
    /// spans that cover the lines and adds back the uncovered remainder.
    /// Atomic; returns the ids of the added spans.
    fn edit_spans(
        &self,
        id: ReviewId,
        remove: Vec<SpanId>,
        add: Vec<(ReviewedSpan, SpanOrigin)>,
    ) -> impl Future<Output = Result<Vec<SpanId>, StoreError>> + Send;
    fn acknowledge(&self, id: ReviewId, ack: NonTextAck) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn unacknowledge(&self, id: ReviewId, ack: NonTextAck) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Opens a thread with its first message (whose `id` is ignored); `review` is the review it
    /// is written in, if any. Answers with the ids of the thread and of the message.
    fn add_thread(
        &self,
        changeset: ChangesetId,
        review: Option<ReviewId>,
        target: CommentTarget,
        opener: Message,
        platform: Option<PlatformThread>,
        status: ThreadStatus,
    ) -> impl Future<Output = Result<(ThreadId, MessageId), StoreError>> + Send;
    /// Adds a message after the thread's others; its `id` is ignored.
    fn add_message(
        &self,
        thread: ThreadId,
        message: Message,
    ) -> impl Future<Output = Result<MessageId, StoreError>> + Send;
    /// Replaces everything of the stored message `message.id` but its place in the thread.
    fn replace_message(&self, message: Message) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn delete_message(&self, message: MessageId) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn set_thread_status(
        &self,
        thread: ThreadId,
        status: ThreadStatus,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Replaces what the thread is on its platform.
    fn set_thread_platform(
        &self,
        thread: ThreadId,
        platform: PlatformThread,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Re-anchor a thread (attach, confirm a moved anchor).
    fn set_thread_target(
        &self,
        thread: ThreadId,
        target: CommentTarget,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn delete_thread(&self, thread: ThreadId) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Everything a live session reads of the store, and the means to write what it changes. The
/// session keeps the writes to the commands that hold its write token.
pub trait LiveStore: Store + ChangesetStore + ReviewStore + ThreadStore {
    type Writes: ReviewWrites + Clone + Send + Sync + 'static;

    /// The store's write connection; its clones share the one connection.
    fn writes(&self) -> Self::Writes;
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
    /// Learned by [`ServerResolver::version`]; `resolve` leaves it empty.
    pub version: Option<String>,
    /// The `tsserver` it answers through; only a TypeScript server has one.
    pub tsserver: Option<Tsserver>,
}

/// Which `tsserver` a TypeScript language server answers through.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Tsserver {
    Using {
        /// Which `typescript` it is, e.g. "the project's typescript 5.9.2". Fit to show.
        whose: String,
        /// Its `lib/tsserver.js`.
        path: PathBuf,
    },
    Missing {
        /// Why there is none. Fit to show.
        reason: String,
    },
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
    /// The version of a server `resolve` found. Separate because learning it
    /// can start a process, which a server start does not wait for.
    fn version(&self, language: &BackendId, found: &FoundServer) -> impl Future<Output = Option<String>> + Send;
    /// The language's name as a person knows it, e.g. "C#".
    fn title(&self, language: &BackendId) -> String;
    /// The server's name as a person knows it, e.g. "Pyright".
    fn display_name(&self, language: &BackendId) -> String;
    /// The file extensions (without the dot) the language's server serves.
    fn extensions(&self, language: &BackendId) -> Vec<String>;
    /// The names of files without an extension that the language's server serves, each also
    /// followed by a dot and anything (`Dockerfile`, `Dockerfile.dev`).
    fn file_names(&self, language: &BackendId) -> Vec<String>;
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

/// A recently opened repository: the path it was recorded with and when.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecentProject {
    pub path: String,
    /// Unix seconds.
    pub opened_at: i64,
}

/// The recently opened repositories, kept across launches.
pub trait RecentStore {
    /// The projects as recorded, most recent first.
    fn recent_projects(&self) -> impl Future<Output = Result<Vec<RecentProject>, StoreError>> + Send;
    /// Replaces the whole list; `projects` is most recent first.
    fn set_recent_projects(&self, projects: Vec<RecentProject>) -> impl Future<Output = Result<(), StoreError>> + Send;
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
