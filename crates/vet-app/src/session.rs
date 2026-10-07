//! A live session of a dirty tree (§5, §6.1, §6.2).
//!
//! **Lifecycle.** A session starts out browsing: the live diff against the
//! base, with nothing stored. The first review action (marking seen,
//! acknowledging, a new thread) starts the review: it records the base, `HEAD`
//! and the current target as version 1, and from then on every settled snapshot
//! is the next version. Finishing releases the review's refs and returns to
//! browsing (see SPEC "Session lifecycle").
//!
//! **Changeset.** What it displays is `base → the latest settled snapshot`,
//! the base being the one chosen with `set_base` (§6.5), by default the current
//! branch's tip. The review is of everything the agent has done to the tree,
//! so reviewing the live part (`previous version → now`) alone would hide
//! reviewed work behind a changeset that has no review state of its own. Marks
//! recorded against earlier targets carry over by remapping (§5.2), so the live
//! part is exactly what shows up as unseen. Showing it separately is an
//! interdiff, which belongs to the timeline (M1).
//!
//! **Revision.** Starts at 0 and increases by one with each settled snapshot,
//! base or filter change, and finished review.
//!
//! **Review state.** Recorded spans stay as they were marked and are never
//! remapped in place. Remapping starts from them whenever the target changes or
//! a mark is edited, and the result lives only in memory. Un-marking a hunk
//! cuts the lines that map into it out of the recorded spans, in their recorded
//! coordinates: lines such a span lost to earlier edits stay in it and come back
//! when the edit is reverted.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::{fmt, slice};

use tokio::sync::Mutex;
use vet_core::completion::{Completion, Coverage, HunkState};
use vet_core::content::{
    BlobId, Changeset, EntryKind, FileDiff, FileEntry, Hunk, LineDelta, LineRange, NonTextChange, NonTextKind,
    RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus, decode_text, is_binary,
};
use vet_core::filter::ReviewFilter;
use vet_core::inline::TextChange;
use vet_core::marking::{hunk_regions, hunk_span};
use vet_core::prepared::PreparedFile;
use vet_core::remap::{cut_recorded, remap_span};
use vet_core::resolve::{Resolution, resolve_anchor};
use vet_core::review::{
    Anchor, AnchorState, CommentTarget, FileRemapInput, Location, Message, ReviewedSpan, Side, StaleReason, Thread,
    ThreadId, ThreadStatus, target_line,
};

use crate::acquisition::{Acquisition, BlobError};
use crate::base;
use crate::editor::{self, last_text_line, line_count};
use crate::execution::Execution;
use crate::execution::check::RunCheckConfig;
use crate::pin::{BaseChoice, CommitId};
use crate::ports::{
    BaseCandidates, BlobRead, BranchInfo, Clock, CommitInfo, Degraded, Dirty, GitError, GitRepo, ReviewStore, Scan,
    Store, StoreError, Watcher,
};
use crate::review::{NonTextAck, RecordedSpan, SessionId, SpanId};
use crate::settle::Settling;

#[derive(Debug)]
pub enum SessionError {
    Git(GitError),
    Store(StoreError),
    /// The request was made against an older revision.
    Stale {
        current: u32,
    },
    NotFound(String),
    /// The content cannot be shown or marked as text.
    NotText(String),
    /// The chosen base does not resolve; the message is fit to show.
    Base(String),
    /// The request does not apply to the thread or the text; the message is fit to show.
    Invalid(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => write!(f, "git: {}", e.0),
            Self::Store(e) => write!(f, "store: {}", e.0),
            Self::Stale { current } => write!(f, "stale revision; the changeset is at revision {current}"),
            Self::NotFound(what) => write!(f, "no such {what}"),
            Self::NotText(path) => write!(f, "{path} has no text to show"),
            Self::Base(message) | Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl From<GitError> for SessionError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

impl From<StoreError> for SessionError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<BlobError> for SessionError {
    fn from(e: BlobError) -> Self {
        match e {
            BlobError::Git(e) => Self::Git(e),
            BlobError::Store(e) => Self::Store(e),
        }
    }
}

type Result<T> = std::result::Result<T, SessionError>;

/// How many recent commits a base can be picked from.
const MAX_BASE_COMMITS: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReviewView {
    /// One per hunk of the file.
    pub hunks: Vec<HunkState>,
    pub acknowledged: Vec<NonTextKind>,
}

/// One side of a non-text change, with what a card needs beyond the entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EntryInfo {
    pub entry: FileEntry,
    /// Where a symlink points, or the commit a submodule is pinned at.
    pub target: Option<String>,
    /// Content size of a binary, renamed or mode-changed file.
    pub size: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ItemView {
    pub kind: NonTextKind,
    pub before: Option<EntryInfo>,
    pub after: Option<EntryInfo>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileView {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub status: FileStatus,
    pub base_lines: u32,
    pub target_lines: u32,
    pub hunks: Vec<Hunk>,
    pub delta: LineDelta,
    pub non_text: Vec<ItemView>,
    pub review: ReviewView,
    pub completion: Completion,
}

/// The session's base as chosen and as resolved, with what to show of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BaseView {
    pub choice: BaseChoice,
    pub commit: CommitInfo,
    /// What a person calls the commit: a branch, `branch~N` or the short hash.
    pub name: String,
    /// How many commits a chosen branch has gained since; `None` unless it moved.
    pub behind: Option<u32>,
}

/// What the changes lead to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetView {
    /// "Working tree".
    pub name: String,
    /// The checked-out branch, if any.
    pub branch: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lifecycle {
    /// No review has started; the diff is live and nothing is kept.
    Browsing,
    Open {
        /// Of the current snapshot: 1 is the review's start.
        version: u32,
        /// Unix seconds the version settled.
        updated_at: i64,
    },
}

/// The changeset through the review filter; hidden files are only counted in
/// `completion`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ManifestView {
    pub revision: u32,
    pub name: String,
    pub lifecycle: Lifecycle,
    pub base: BaseView,
    pub target: TargetView,
    pub filter: ReviewFilter,
    pub files: Vec<FileView>,
    /// The lines the shown files add and remove.
    pub delta: LineDelta,
    pub completion: Completion,
    /// Why the tree is polled instead of watched, if it is.
    pub degraded: Option<Degraded>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReviewUpdateView {
    pub revision: u32,
    pub path: RepoPath,
    pub review: ReviewView,
    pub completion: Completion,
    pub changeset: Completion,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileText {
    pub revision: u32,
    pub base: Option<String>,
    pub target: Option<String>,
    /// The diff's changed ranges within the texts.
    pub changes: Vec<TextChange>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirEntryKind {
    File,
    Dir,
    Symlink,
    Submodule,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DirItem {
    pub name: String,
    pub kind: DirEntryKind,
    /// The path, or anything below a directory, is in the changeset.
    pub changed: bool,
    /// The review filter excludes the path, or everything below a directory.
    pub excluded: bool,
}

/// Directories first, then the others, each by name in byte order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DirListing {
    pub revision: u32,
    pub entries: Vec<DirItem>,
}

/// Why a file has no text to show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoText {
    Binary,
    /// Over the capture cap (also for git-tracked content), or not readable any more.
    Unavailable,
    /// A symlink or a submodule.
    NotAFile,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetText {
    pub revision: u32,
    pub content: std::result::Result<String, NoText>,
}

/// Largest file `item_blob` returns.
pub const MAX_ITEM_BLOB: usize = 8 * 1024 * 1024;

/// One side of a non-text item's file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ItemBlob {
    Bytes {
        /// Only for common image formats.
        media_type: Option<&'static str>,
        bytes: Vec<u8>,
    },
    /// The item has no file on this side.
    Absent,
    /// Over [`MAX_ITEM_BLOB`] or the capture cap.
    TooLarge,
    /// A symlink or submodule, or content that is not readable any more.
    Unavailable,
}

/// The media type of a common image format, from the file extension (any case).
fn image_media_type(path: &RepoPath) -> Option<&'static str> {
    Some(match path.extension()?.to_ascii_lowercase().as_slice() {
        b"png" => "image/png",
        b"jpg" | b"jpeg" => "image/jpeg",
        b"gif" => "image/gif",
        b"webp" => "image/webp",
        b"bmp" => "image/bmp",
        b"ico" => "image/x-icon",
        b"svg" => "image/svg+xml",
        _ => return None,
    })
}

/// The two snapshots of a revision, each with the commit it is built on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SemanticSides {
    pub base: SnapshotId,
    pub base_commit: CommitId,
    pub target: SnapshotId,
    pub head: CommitId,
}

/// What a new thread is attached to. Ranges are in the text of the revision the
/// request names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NewThread {
    Range {
        side: Side,
        path: RepoPath,
        range: LineRange,
    },
    File {
        side: Side,
        path: RepoPath,
    },
    Changeset,
}

/// The result of a request that can start the review.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome<T> {
    pub value: T,
    /// The review as it began, if this request started it.
    pub started: Option<Lifecycle>,
}

impl<T> std::ops::Deref for Outcome<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

fn outcome<T>(st: &State, value: T, started: bool) -> Outcome<T> {
    Outcome {
        value,
        started: started.then(|| lifecycle_of(st)),
    }
}

/// A thread with its anchor resolved against the current revision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThreadView {
    pub id: ThreadId,
    /// As recorded, never replaced by a resolution.
    pub target: CommentTarget,
    /// The file the thread is shown at: where the anchor or file is now, else
    /// where it was. `None` for a changeset thread.
    pub path: Option<RepoPath>,
    /// `None` for a changeset thread.
    pub state: Option<AnchorState>,
    /// The lines of a range thread now; `None` when it is detached.
    pub location: Option<Location>,
    /// The line of the target text the thread's card is shown below in a unified diff; `Some`
    /// exactly when `location` is.
    pub target_line: Option<u32>,
    pub messages: Vec<Message>,
    pub status: ThreadStatus,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThreadsView {
    pub revision: u32,
    pub threads: Vec<ThreadView>,
}

/// What the current revision's diff needs, computed on first use.
struct Changes {
    diff: SnapshotDiff,
    /// Every path the diff mentions, on either side.
    changed: BTreeSet<RepoPath>,
    /// Text of the diffed files and of what recorded spans were marked in.
    blobs: BTreeMap<BlobId, Vec<u8>>,
    /// Recorded spans as they are now.
    remapped: Vec<(SpanId, Vec<ReviewedSpan>)>,
    flat: Vec<ReviewedSpan>,
    /// Range threads resolved from their recorded anchors.
    resolutions: HashMap<ThreadId, Resolution>,
    /// Both sides of each `diff.non_text` change.
    items: Vec<ItemView>,
}

/// A started review.
struct Review {
    id: SessionId,
    version: u32,
    updated_at: i64,
}

struct State {
    revision: u32,
    /// `None` while browsing.
    review: Option<Review>,
    filter: ReviewFilter,
    base: Snapshot,
    base_choice: BaseChoice,
    /// The commit `base` is the tree of.
    base_commit: CommitId,
    /// The commit target snapshots are built on: `HEAD`, as of the review's
    /// start while one is open.
    head: CommitId,
    target: Snapshot,
    target_status: SnapshotStatus,
    /// Other snapshots that recorded spans refer to.
    known: HashMap<SnapshotId, Snapshot>,
    recorded: Vec<RecordedSpan>,
    acks: Vec<NonTextAck>,
    threads: Vec<Thread>,
    degraded: Option<Degraded>,
    changes: Option<Changes>,
}

impl State {
    /// Every anchor the recorded spans and the range threads are placed by.
    fn anchors(&self) -> impl Iterator<Item = &Anchor> {
        let spans = (self.recorded.iter()).flat_map(|r| r.span.base.iter().chain(&r.span.target));
        let threads = self.threads.iter().filter_map(|t| match &t.target {
            CommentTarget::Range(a) => Some(a),
            _ => None,
        });
        spans.chain(threads)
    }

    /// The snapshot `id` if the session has it in memory.
    fn snapshot(&self, id: SnapshotId) -> Option<&Snapshot> {
        if id == self.base.id {
            Some(&self.base)
        } else if id == self.target.id {
            Some(&self.target)
        } else {
            self.known.get(&id)
        }
    }
}

/// Unix seconds now.
pub type UnixClock = fn() -> i64;

pub struct LiveSession<G, S> {
    /// Author of the messages written here: git's `user.name`, else "You".
    author: String,
    /// Shared with the session's semantic pool, which materializes snapshots from it.
    acquisition: Arc<Acquisition<G, S>>,
    name: String,
    /// The repository root, the key a review is stored under.
    repo: String,
    clock: UnixClock,
    /// Serializes everything that reads or edits review state.
    state: Mutex<State>,
}

fn ack_of(change: &NonTextChange) -> NonTextAck {
    NonTextAck {
        kind: change.kind,
        before: change
            .before
            .clone()
            .map(|e| (change.from.clone().unwrap_or_else(|| change.path.clone()), e)),
        after: change.after.clone().map(|e| (change.path.clone(), e)),
    }
}

/// What a session is called: the repository's directory name, else its whole path.
fn name_of_repo(repo: &str) -> String {
    std::path::Path::new(repo)
        .file_name()
        .map_or_else(|| repo.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// Git's `user.name`, else "You".
async fn author_of<G: GitRepo>(git: &G) -> String {
    git.user_name()
        .await
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "You".into())
}

impl<G: GitRepo + Sync, S: Store + ReviewStore + Sync> LiveSession<G, S> {
    /// Resumes the open review of the repository at `repo` (the root, as the
    /// key reviews are recorded under), or else starts out browsing the tree
    /// against the current branch's tip.
    pub async fn open(git: G, store: S, repo: String, clock: UnixClock) -> Result<Self> {
        let acquisition = Arc::new(Acquisition::new(git, store));
        let filter = acquisition.store.filter(repo.clone()).await?.normalized();
        let name = name_of_repo(&repo);
        if let Some(&id) = acquisition.store.open_sessions(repo.clone()).await?.last() {
            if let Some(session) = Self::resume(&acquisition, id, filter.clone(), &name, &repo, clock).await? {
                return Ok(session);
            }
            // Its start record or snapshots are gone: browse rather than fail to open.
            acquisition.git.delete_session_refs(id).await?;
            acquisition.store.close_session(id, clock(), false).await?;
        }
        let git = &acquisition.git;
        let head = git.head().await?;
        let (base_choice, base_commit) = base::default_choice(&git.branches().await?, &head);
        let base = git.commit_snapshot(&base_commit).await?;
        let Scan {
            snapshot: target,
            status,
        } = acquisition.reconcile(Dirty::All).await?;
        acquisition.record_commit(base.id, base_commit.clone());
        acquisition.record_commit(target.id, head.clone());
        let state = State {
            revision: 0,
            review: None,
            filter,
            base,
            base_choice,
            base_commit,
            head,
            target,
            target_status: status,
            known: HashMap::new(),
            recorded: Vec::new(),
            acks: Vec::new(),
            threads: Vec::new(),
            degraded: None,
            changes: None,
        };
        Ok(Self {
            author: author_of(&acquisition.git).await,
            acquisition,
            name,
            repo,
            clock,
            state: Mutex::new(state),
        })
    }

    /// Loads the review state of the stored session `id` and settles the tree as
    /// it is now, so recorded spans and threads remap onto whatever changed
    /// while Vet was not running (I3). `None` if the session cannot be
    /// reconstructed: its start record, its latest snapshot or its base commit
    /// is gone.
    async fn resume(
        acquisition: &Arc<Acquisition<G, S>>,
        id: SessionId,
        filter: ReviewFilter,
        name: &str,
        repo: &str,
        clock: UnixClock,
    ) -> Result<Option<Self>> {
        let store = &acquisition.store;
        let Some(stored) = store.session(id).await? else {
            return Ok(None);
        };
        let Some(pin) = store.pin(stored.pin).await? else {
            return Ok(None);
        };
        let timeline = store.timeline(id).await?;
        let latest = timeline.last().map_or(stored.changeset.target, |v| v.snapshot);
        let Some((target, target_status)) = store.snapshot(latest).await? else {
            return Ok(None);
        };
        let base_commit = stored.base_commit;
        let Ok(base) = acquisition.git.commit_snapshot(&base_commit).await else {
            return Ok(None);
        };
        // The session keeps its base however far the branches have moved since:
        // commits made while Vet was closed are part of what is under review,
        // and re-resolving would turn every mark against the old base into
        // `SeenAgainstOldBase`. Moving the base is the user's call (§6.5).
        for version in &timeline {
            acquisition.record_commit(version.snapshot, pin.head.clone());
        }
        acquisition.record_commit(base.id, base_commit.clone());
        acquisition.git.set_session_ref(id, "base", &base_commit).await?;
        acquisition.git.set_session_ref(id, "target", &pin.head).await?;
        let state = State {
            revision: 0,
            review: Some(Review {
                id,
                version: timeline.len().max(1) as u32,
                updated_at: timeline.last().map_or(0, |v| v.at),
            }),
            filter,
            base,
            base_choice: stored.base,
            base_commit,
            head: pin.head,
            target,
            target_status,
            known: HashMap::new(),
            recorded: stored.spans,
            acks: stored.acks,
            threads: stored.threads,
            degraded: None,
            changes: None,
        };
        let session = Self {
            author: author_of(&acquisition.git).await,
            acquisition: acquisition.clone(),
            name: name.to_owned(),
            repo: repo.to_owned(),
            clock,
            state: Mutex::new(state),
        };
        let scan = acquisition.reconcile(Dirty::All).await?;
        if scan.snapshot.id != session.settled_snapshot().await {
            let (revision, retained) = session.settled(scan).await?;
            if let Some(e) = retained {
                eprintln!("vet: could not keep the content of revision {revision}: {e}");
            }
        }
        Ok(Some(session))
    }

    /// Starts the review (SPEC "Session lifecycle"): records the base, `HEAD`
    /// and the current target, which becomes version 1, copies the target's
    /// dirty content into the store and takes the session refs. Does nothing
    /// if a review is open. Returns whether it started one.
    async fn begin(&self, st: &mut State) -> Result<bool> {
        if st.review.is_some() {
            return Ok(false);
        }
        let acquisition = &self.acquisition;
        let (git, store) = (&acquisition.git, &acquisition.store);
        let head = git.head().await?;
        acquisition.retain(&[&st.base], &st.target, &st.target_status).await?;
        let now = (self.clock)();
        store
            .put_snapshot(st.base.clone(), SnapshotStatus::of(&st.base), now)
            .await?;
        store
            .put_snapshot(st.target.clone(), st.target_status.clone(), now)
            .await?;
        let pin = store.put_pin(head.clone(), st.target.id).await?;
        let changeset = Changeset {
            base: st.base.id,
            target: st.target.id,
        };
        let id = store
            .open_session(
                self.repo.clone(),
                pin.id,
                changeset,
                st.base_choice.clone(),
                st.base_commit.clone(),
            )
            .await?;
        store.append_timeline(id, st.target.id, now).await?;
        git.set_session_ref(id, "base", &st.base_commit).await?;
        git.set_session_ref(id, "target", &head).await?;
        acquisition.record_commit(st.target.id, head.clone());
        st.head = head;
        st.review = Some(Review {
            id,
            version: 1,
            updated_at: now,
        });
        Ok(true)
    }

    /// Whether a review is open, and its version.
    pub async fn lifecycle(&self) -> Lifecycle {
        lifecycle_of(&*self.state.lock().await)
    }

    /// Ends the review: releases its refs, records whether it was complete, and
    /// goes back to browsing with no review state. Returns whether it was
    /// complete. Fails if no review is open. Bumps the revision.
    pub async fn finish_review(&self) -> Result<bool> {
        let mut st = self.state.lock().await;
        let Some(id) = st.review.as_ref().map(|r| r.id) else {
            return Err(SessionError::Invalid("no review is open".into()));
        };
        self.ensure_changes(&mut st).await?;
        let complete = {
            let changes = st.changes.as_ref().expect("computed");
            let acked = acknowledged(&st, &changes.diff);
            changeset_completion(&Coverage::new(&changes.flat), &st.filter, &changes.diff, &acked).is_complete()
        };
        self.acquisition.git.delete_session_refs(id).await?;
        self.acquisition
            .store
            .close_session(id, (self.clock)(), complete)
            .await?;
        st.review = None;
        st.recorded.clear();
        st.acks.clear();
        st.threads.clear();
        st.known.clear();
        st.revision += 1;
        st.changes = None;
        Ok(complete)
    }

    /// Changes what the target is compared against; the target stays the live
    /// tree. While a review is open, moves its base ref, and bumps the revision
    /// unless nothing changed. Recorded spans are untouched: the next manifest
    /// remaps them onto the new base, so base-side changes in reviewed regions
    /// show as `SeenAgainstOldBase` (§5.2), and choosing an earlier base again
    /// restores the earlier coverage. Returns the revision.
    pub async fn set_base(&self, choice: BaseChoice) -> Result<(BaseView, u32)> {
        let git = &self.acquisition.git;
        let (choice, commit) = base::resolve(git, choice).await.map_err(SessionError::Base)?;
        let snapshot = git.commit_snapshot(&commit).await?;

        let mut st = self.state.lock().await;
        if st.base_choice != choice || st.base_commit != commit {
            if let Some(review) = &st.review {
                let store = &self.acquisition.store;
                // Before the ref moves: git may drop the old base's objects afterwards.
                self.retain_anchored(&st).await?;
                store
                    .put_snapshot(snapshot.clone(), SnapshotStatus::of(&snapshot), (self.clock)())
                    .await?;
                git.set_session_ref(review.id, "base", &commit).await?;
                store
                    .set_base(review.id, snapshot.id, choice.clone(), commit.clone())
                    .await?;
            }
            self.acquisition.record_commit(snapshot.id, commit.clone());
            st.base = snapshot;
            st.base_choice = choice;
            st.base_commit = commit;
            st.revision += 1;
            st.changes = None;
        }
        Ok((self.base_view(&st, &git.branches().await?).await?, st.revision))
    }

    /// The base with its display facts; `branches` are the repository's.
    async fn base_view(&self, st: &State, branches: &[BranchInfo]) -> Result<BaseView> {
        let git = &self.acquisition.git;
        let prefer = match &st.base_choice {
            BaseChoice::Branch { name } => Some(name.as_str()),
            _ => None,
        };
        Ok(BaseView {
            choice: st.base_choice.clone(),
            commit: git.commit_info(&st.base_commit).await?,
            name: base::name_of(git, branches, prefer, &st.base_commit).await?,
            behind: base::behind(git, branches, &st.base_choice, &st.base_commit).await?,
        })
    }

    /// Replaces the repository's review filter, stored whether or not a review
    /// is open, and bumps the revision; completion and the excluded counts
    /// follow with the next manifest. Returns the revision.
    pub async fn set_filter(&self, filter: ReviewFilter) -> Result<u32> {
        let filter = filter.normalized();
        filter.validate().map_err(SessionError::Invalid)?;
        let mut st = self.state.lock().await;
        if st.filter == filter {
            return Ok(st.revision);
        }
        self.acquisition
            .store
            .set_filter(self.repo.clone(), filter.clone())
            .await?;
        st.filter = filter;
        st.revision += 1;
        Ok(st.revision)
    }

    /// Branches and recent commits to pick a base from.
    pub async fn base_candidates(&self) -> Result<BaseCandidates> {
        Ok(self.acquisition.git.base_candidates(MAX_BASE_COMMITS).await?)
    }

    pub async fn revision(&self) -> u32 {
        self.state.lock().await.revision
    }

    /// The latest settled target: what the review last showed.
    pub async fn target_snapshot(&self) -> Snapshot {
        self.state.lock().await.target.clone()
    }

    /// What knows the origin of the session's snapshots, for serving them to a
    /// language-server pool.
    pub fn acquisition(&self) -> Arc<Acquisition<G, S>> {
        self.acquisition.clone()
    }

    /// The snapshots of `revision`: what the semantic layer asks about. Fails
    /// if `revision` is no longer current.
    pub async fn semantic_sides(&self, revision: u32) -> Result<SemanticSides> {
        let st = self.state.lock().await;
        check_revision(&st, revision)?;
        Ok(SemanticSides {
            base: st.base.id,
            base_commit: st.base_commit.clone(),
            target: st.target.id,
            head: st.head.clone(),
        })
    }

    /// The text diff of `revision`, for the analyses that map symbols to hunks.
    pub async fn changed_files(&self, revision: u32) -> Result<Vec<FileDiff>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        Ok(st.changes.as_ref().expect("computed").diff.files.clone())
    }

    /// Running in place and the run check, on the session's acquisition.
    pub fn execution<'a, P>(
        &'a self,
        runner: &'a P,
        repo: String,
        root: PathBuf,
        config: RunCheckConfig,
    ) -> Execution<'a, G, S, P> {
        Execution {
            acquisition: &self.acquisition,
            runner,
            repo,
            root,
            config,
        }
    }

    /// The initial scan identifies the settled snapshot, so settling does not
    /// report it again.
    pub async fn settled_snapshot(&self) -> SnapshotId {
        self.state.lock().await.target.id
    }

    /// Makes a settled snapshot the target and returns the new revision. While a
    /// review is open it is also the next version: its content is stored and
    /// it joins the timeline. Review state is remapped when the next manifest
    /// is computed. A failed copy of its content is not fatal: the diff then
    /// reads the worktree, or shows the file as unavailable.
    pub async fn settled(&self, scan: Scan) -> Result<(u32, Option<BlobError>)> {
        let mut guard = self.state.lock().await;
        let st = &mut *guard;
        let mut retained = None;
        if let Some(review) = &st.review {
            retained = self
                .acquisition
                .retain(&[&st.base, &st.target], &scan.snapshot, &scan.status)
                .await
                .err();
            let store = &self.acquisition.store;
            let now = (self.clock)();
            store
                .put_snapshot(scan.snapshot.clone(), scan.status.clone(), now)
                .await?;
            store.append_timeline(review.id, scan.snapshot.id, now).await?;
        }
        // Older timeline snapshots stay materializable (§7.2).
        self.acquisition.record_commit(scan.snapshot.id, st.head.clone());
        let old = std::mem::replace(&mut st.target, scan.snapshot);
        st.target_status = scan.status;
        if let Some(review) = &mut st.review {
            st.known.insert(old.id, old);
            review.version += 1;
            review.updated_at = (self.clock)();
        }
        st.revision += 1;
        st.changes = None;
        Ok((st.revision, retained))
    }

    /// Settles snapshots of the worktree as `settling` reports them and calls
    /// `on_settled` with each new revision, until dropped.
    pub async fn follow<W: Watcher + Send, C: Clock + Send + Sync>(
        &self,
        mut settling: Settling<W, C>,
        mut on_settled: impl FnMut(u32) + Send,
    ) {
        self.state.lock().await.degraded = settling.degraded();
        loop {
            let scan = match settling.next_settled(&self.acquisition).await {
                Ok(scan) => scan,
                Err(e) => {
                    eprintln!("vet: reconciliation failed: {}", e.0);
                    continue;
                }
            };
            match self.settled(scan).await {
                Ok((revision, retained)) => {
                    if let Some(e) = retained {
                        eprintln!("vet: could not keep the content of revision {revision}: {e}");
                    }
                    self.state.lock().await.degraded = settling.degraded();
                    on_settled(revision);
                }
                Err(e) => eprintln!("vet: could not record a settled snapshot: {e}"),
            }
        }
    }

    pub async fn manifest(&self) -> Result<ManifestView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        let branches = self.acquisition.git.branches().await?;
        let base = self.base_view(&st, &branches).await?;
        let changes = st.changes.as_ref().expect("computed");
        let cov = Coverage::new(&changes.flat);
        let acked = acknowledged(&st, &changes.diff);
        let compiled = st.filter.compile();

        // Files by path, with their text diff and non-text items.
        type Parts<'a> = (Option<&'a FileDiff>, Vec<usize>);
        let mut by_path: BTreeMap<&RepoPath, Parts> = BTreeMap::new();
        for d in &changes.diff.files {
            by_path.entry(d.path()).or_default().0 = Some(d);
        }
        for (i, n) in changes.diff.non_text.iter().enumerate() {
            by_path.entry(&n.path).or_default().1.push(i);
        }

        // A missing side of a text diff is an empty text.
        let lines = |diff: Option<&FileDiff>, blob: Option<BlobId>| {
            diff.map_or(0, |_| {
                blob.and_then(|b| changes.blobs.get(&b)).map_or(1, |t| line_count(t))
            })
        };
        let mut files = Vec::new();
        for (path, (diff, indices)) in by_path {
            let indices: Vec<usize> = indices
                .into_iter()
                .filter(|&i| compiled.shows(&changes.diff.non_text[i]))
                .collect();
            let items: Vec<NonTextChange> = indices.iter().map(|&i| changes.diff.non_text[i].clone()).collect();
            if !compiled.passes(path) || (diff.is_none() && items.is_empty()) {
                continue;
            }
            let hunks = diff.map(|d| d.hunks.clone()).unwrap_or_default();
            files.push(FileView {
                path: path.clone(),
                old_path: diff
                    .and_then(|d| d.old_path.clone())
                    .or_else(|| items.iter().find_map(|n| n.from.clone()))
                    .filter(|old| old != path),
                status: status_of(diff, &items),
                base_lines: lines(diff, diff.and_then(|d| d.old_blob)),
                target_lines: lines(diff, diff.and_then(|d| d.new_blob)),
                delta: LineDelta::of(&hunks),
                hunks,
                review: review_of(&cov, diff, &items, &acked),
                completion: file_completion(&cov, &st.filter, diff, &items, &acked),
                non_text: indices.iter().map(|&i| changes.items[i].clone()).collect(),
            });
        }
        Ok(ManifestView {
            revision: st.revision,
            name: self.name.clone(),
            lifecycle: lifecycle_of(&st),
            base,
            target: TargetView {
                name: "Working tree".into(),
                branch: branches.iter().find(|b| b.current).map(|b| b.name.clone()),
            },
            filter: st.filter.clone(),
            completion: changeset_completion(&cov, &st.filter, &changes.diff, &acked),
            delta: files.iter().map(|f| f.delta).sum(),
            files,
            degraded: st.degraded,
        })
    }

    /// Both sides of a file with text, at the current revision.
    pub async fn file_text(&self, path: &str) -> Result<FileText> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        let changes = st.changes.as_ref().expect("computed");
        type Side = Option<(RepoPath, BlobId)>;
        let diff = find_diff(&changes.diff, path);
        let hunks = diff.map(|d| d.hunks.clone()).unwrap_or_default();
        let (old, new): (Side, Side) = if let Some(d) = diff {
            let side = |p: &Option<RepoPath>, b: Option<BlobId>| p.clone().zip(b);
            (side(&d.old_path, d.old_blob), side(&d.new_path, d.new_blob))
        } else if let Some(n) = changes.diff.non_text.iter().find(|n| n.path.to_string_lossy() == path) {
            let side = |p: RepoPath, e: &Option<vet_core::content::FileEntry>| {
                e.as_ref().filter(|e| e.kind == EntryKind::File).map(|e| (p, e.blob))
            };
            (
                side(n.from.clone().unwrap_or_else(|| n.path.clone()), &n.before),
                side(n.path.clone(), &n.after),
            )
        } else {
            return Err(SessionError::NotFound(format!("file {path}")));
        };
        let mut blobs = std::mem::take(&mut st.changes.as_mut().expect("computed").blobs);
        let mut texts = [None, None];
        let mut result = Ok(());
        for (text, side) in texts.iter_mut().zip([old, new]) {
            let Some((p, b)) = side else { continue };
            match self.text_of(&mut blobs, &p, b).await {
                Ok(Ok(t)) => *text = Some(t),
                Ok(Err(_)) => result = Err(SessionError::NotText(p.to_string_lossy())),
                Err(e) => result = Err(e),
            }
            if result.is_err() {
                break;
            }
        }
        st.changes.as_mut().expect("computed").blobs = blobs;
        result?;
        let [base, target] = texts;
        let changes = editor::text_changes(
            base.as_deref().unwrap_or_default(),
            target.as_deref().unwrap_or_default(),
            &hunks,
        );
        Ok(FileText {
            revision: st.revision,
            base,
            target,
            changes,
        })
    }

    /// The children of `path` (`""` is the root) in the target snapshot.
    pub async fn list_directory(&self, path: &str) -> Result<DirListing> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        let changes = st.changes.as_ref().expect("computed");
        let compiled = st.filter.compile();
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let prefix = prefix.as_bytes();
        let has_prefix = |p: &RepoPath, below: &[u8]| p.0.starts_with(below);

        let mut entries = Vec::new();
        let mut next = RepoPath(prefix.to_vec());
        while let Some((key, entry)) = st.target.entries.range(next.clone()..).next() {
            let Some(rest) = key.0.strip_prefix(prefix) else { break };
            let (name, below) = match rest.iter().position(|&b| b == b'/') {
                None => {
                    // Skips to the first key after this one.
                    next = RepoPath([key.0.as_slice(), &[0]].concat());
                    let kind = match entry.kind {
                        EntryKind::File => DirEntryKind::File,
                        EntryKind::Symlink => DirEntryKind::Symlink,
                        EntryKind::Submodule => DirEntryKind::Submodule,
                    };
                    entries.push(DirItem {
                        name: String::from_utf8_lossy(rest).into_owned(),
                        kind,
                        changed: changes.changed.contains(key),
                        excluded: !compiled.passes(key),
                    });
                    continue;
                }
                Some(i) => (&rest[..i], [&key.0[..prefix.len() + i], b"/"].concat()),
            };
            // Skips everything below the directory: `0` is the byte after `/`.
            next = RepoPath([&below[..below.len() - 1], b"0"].concat());
            let below = RepoPath(below);
            entries.push(DirItem {
                name: String::from_utf8_lossy(name).into_owned(),
                kind: DirEntryKind::Dir,
                changed: changes
                    .changed
                    .range(below.clone()..)
                    .next()
                    .is_some_and(|p| has_prefix(p, &below.0)),
                excluded: st
                    .target
                    .entries
                    .range(below.clone()..)
                    .take_while(|(p, _)| has_prefix(p, &below.0))
                    .all(|(p, _)| !compiled.passes(p)),
            });
        }
        if entries.is_empty() && !path.is_empty() {
            return Err(SessionError::NotFound(format!("directory {path}")));
        }
        entries.sort_by(|a, b| {
            let key = |i: &DirItem| (i.kind != DirEntryKind::Dir, i.name.clone().into_bytes());
            key(a).cmp(&key(b))
        });
        Ok(DirListing {
            revision: st.revision,
            entries,
        })
    }

    /// The text of any file of the target snapshot, changed or not.
    pub async fn target_text(&self, path: &str) -> Result<TargetText> {
        let st = self.state.lock().await;
        let key = RepoPath::from(path);
        let entry = st
            .target
            .entries
            .get(&key)
            .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
        let content = match entry.kind {
            EntryKind::File => self.text_of(&mut BTreeMap::new(), &key, entry.blob).await?,
            EntryKind::Symlink | EntryKind::Submodule => Err(NoText::NotAFile),
        };
        Ok(TargetText {
            revision: st.revision,
            content,
        })
    }

    /// Marks hunks (indices into the file's hunks at `revision`) seen or unseen.
    /// Marking starts the review if none is open.
    pub async fn set_seen(
        &self,
        revision: u32,
        path: &str,
        hunks: &[u32],
        seen: bool,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let started = seen && self.begin(&mut st).await?;
        let Prepared {
            inputs, spans: slots, ..
        } = self.prepare(&mut st).await?;
        let (remove, add) = {
            let changes = st.changes.as_ref().expect("computed");
            let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &changes.blobs)).collect();
            let diff = find_diff(&changes.diff, path).ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
            let blob_text = |blob: Option<BlobId>| blob.and_then(|b| changes.blobs.get(&b)).map(Vec::as_slice);
            let ids = (st.base.id, st.target.id);
            let (mut regions, mut add) = (Vec::new(), Vec::new());
            for &i in hunks {
                let hunk = diff
                    .hunks
                    .get(i as usize)
                    .ok_or_else(|| SessionError::NotFound(format!("hunk {i}")))?;
                regions.extend(hunk_regions(diff, hunk));
                if seen {
                    let texts = (blob_text(diff.old_blob), blob_text(diff.new_blob));
                    add.push(hunk_span(diff, hunk, ids, texts).ok_or_else(|| SessionError::NotText(path.into()))?);
                }
            }
            let mut remove = Vec::new();
            let mut kept = Vec::new();
            for (rec, &(b, t)) in st.recorded.iter().zip(&slots) {
                let (b, t) = (b.map(|i| &prepared[i]), t.map(|i| &prepared[i]));
                if let Some(rest) = cut_recorded(&rec.span, &regions, b, t) {
                    remove.push(rec.id);
                    kept.extend(rest);
                }
            }
            kept.extend(add);
            (remove, kept)
        };
        // Without a review there is nothing to un-mark and nothing to store.
        let ids = match &st.review {
            Some(review) => {
                self.acquisition
                    .store
                    .edit_spans(review.id, remove.clone(), add.clone())
                    .await?
            }
            None => Vec::new(),
        };
        st.recorded.retain(|r| !remove.contains(&r.id));
        st.recorded
            .extend(ids.into_iter().zip(add).map(|(id, span)| RecordedSpan { id, span }));
        self.remap(&mut st).await?;
        Ok(outcome(&st, update_of(&st, path)?, started))
    }

    /// Acknowledges or un-acknowledges one non-text item. Acknowledging starts
    /// the review if none is open.
    pub async fn acknowledge(
        &self,
        revision: u32,
        path: &str,
        kind: NonTextKind,
        acknowledged: bool,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let started = acknowledged && self.begin(&mut st).await?;
        let change = {
            let diff = &st.changes.as_ref().expect("computed").diff;
            find_non_text(diff, kind, path).ok_or_else(|| SessionError::NotFound(format!("change of {path}")))?
        };
        let ack = ack_of(change);
        let store = &self.acquisition.store;
        if let Some(review) = &st.review {
            if acknowledged {
                store.acknowledge(review.id, ack.clone()).await?;
            } else {
                store.unacknowledge(review.id, ack.clone()).await?;
            }
        }
        if !acknowledged {
            st.acks.retain(|a| *a != ack);
        } else if !st.acks.contains(&ack) {
            st.acks.push(ack);
        }
        Ok(outcome(&st, update_of(&st, path)?, started))
    }

    /// The bytes of one side of a non-text item's file at `revision`.
    pub async fn item_blob(&self, revision: u32, path: &str, kind: NonTextKind, side: Side) -> Result<ItemBlob> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let change = find_non_text(&st.changes.as_ref().expect("computed").diff, kind, path)
            .ok_or_else(|| SessionError::NotFound(format!("change of {path}")))?;
        let (file, entry) = match side {
            Side::Base => (change.from.as_ref().unwrap_or(&change.path), &change.before),
            Side::Target => (&change.path, &change.after),
        };
        let Some(entry) = entry else {
            return Ok(ItemBlob::Absent);
        };
        if entry.kind != EntryKind::File {
            return Ok(ItemBlob::Unavailable);
        }
        Ok(match self.acquisition.blob(file, entry.blob, true).await? {
            BlobRead::Found(bytes) if bytes.len() <= MAX_ITEM_BLOB => ItemBlob::Bytes {
                media_type: image_media_type(file),
                bytes,
            },
            BlobRead::Found(_) | BlobRead::TooLarge => ItemBlob::TooLarge,
            BlobRead::Missing => ItemBlob::Unavailable,
        })
    }

    /// Every thread, with range anchors resolved at the current revision.
    pub async fn threads(&self) -> Result<ThreadsView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        Ok(ThreadsView {
            revision: st.revision,
            threads: st.threads.iter().map(|t| thread_view(&st, t)).collect(),
        })
    }

    /// The target line a unified diff shows `line` of `side` of the file at `path` at, by the rule
    /// that places a thread's card (`ThreadView::target_line`). Fails if `revision` is no longer
    /// current.
    pub async fn line_in_target(&self, revision: u32, side: Side, path: &str, line: u32) -> Result<u32> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let location = Location {
            file: RepoPath::from(path),
            range: LineRange {
                start: line,
                end: line.saturating_add(1),
            },
        };
        Ok(card_line(st.changes.as_ref().expect("computed"), side, &location))
    }

    /// Opens a thread with its first message, and starts the review if none is
    /// open. `now` is unix seconds.
    pub async fn create_thread(
        &self,
        revision: u32,
        target: NewThread,
        body: String,
        now: i64,
    ) -> Result<Outcome<ThreadView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let opener = self.message(body, now)?;
        let target = match target {
            NewThread::Range { side, path, range } => {
                CommentTarget::Range(self.anchor_at(&st, side, &path, range).await?)
            }
            NewThread::File { side, path } => {
                if !side_snapshot(&st, side).entries.contains_key(&path) {
                    return Err(SessionError::NotFound(format!("file {}", path.to_string_lossy())));
                }
                CommentTarget::File { side, path }
            }
            NewThread::Changeset => CommentTarget::Changeset,
        };
        let started = self.begin(&mut st).await?;
        let review = st.review.as_ref().expect("begun").id;
        let id = self
            .acquisition
            .store
            .add_thread(review, target.clone(), opener.clone())
            .await?;
        st.threads.push(Thread {
            id,
            target,
            messages: vec![opener],
            status: ThreadStatus::Open,
        });
        self.remap(&mut st).await?;
        Ok(outcome(&st, thread_by_id(&st, id)?, started))
    }

    pub async fn reply(&self, thread: ThreadId, body: String, now: i64) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        find_thread(&st, thread)?;
        let message = self.message(body, now)?;
        self.acquisition.store.add_message(thread, message.clone()).await?;
        edit_thread(&mut st, thread, |t| t.messages.push(message));
        thread_by_id(&st, thread)
    }

    /// Resolves or reopens a thread.
    pub async fn set_thread_status(&self, thread: ThreadId, status: ThreadStatus) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        find_thread(&st, thread)?;
        self.acquisition.store.set_thread_status(thread, status).await?;
        edit_thread(&mut st, thread, |t| t.status = status);
        thread_by_id(&st, thread)
    }

    /// Deletes a thread; this is also how a detached one is discarded.
    pub async fn delete_thread(&self, thread: ThreadId) -> Result<()> {
        let mut st = self.state.lock().await;
        find_thread(&st, thread)?;
        self.acquisition.store.delete_thread(thread).await?;
        st.threads.retain(|t| t.id != thread);
        if let Some(changes) = st.changes.as_mut() {
            changes.resolutions.remove(&thread);
        }
        Ok(())
    }

    /// Re-anchors an `Inferred` thread at its current location, which makes it
    /// `Exact`. Fails if `revision` is no longer current.
    pub async fn confirm_anchor(&self, revision: u32, thread: ThreadId) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let location = match resolution_of(&st, thread)? {
            Resolution {
                state: AnchorState::Inferred { .. },
                location: Some(location),
            } => location.clone(),
            _ => return Err(SessionError::Invalid("the anchor was not moved".into())),
        };
        self.reanchor(&mut st, thread, &location).await
    }

    /// Re-anchors an `Ambiguous` thread at one of its candidates (an index into
    /// the candidates of its state). Fails if `revision` is no longer current.
    pub async fn attach_anchor(&self, revision: u32, thread: ThreadId, candidate: usize) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let location = match &resolution_of(&st, thread)?.state {
            AnchorState::Ambiguous { candidates } => candidates
                .get(candidate)
                .cloned()
                .ok_or_else(|| SessionError::NotFound(format!("candidate {candidate}")))?,
            _ => return Err(SessionError::Invalid("the anchor is not ambiguous".into())),
        };
        self.reanchor(&mut st, thread, &location).await
    }

    fn message(&self, body: String, at: i64) -> Result<Message> {
        if body.trim().is_empty() {
            return Err(SessionError::Invalid("a message cannot be empty".into()));
        }
        Ok(Message {
            author: self.author.clone(),
            body,
            at,
        })
    }

    /// Records the thread's anchor anew, from the current text at `location`.
    async fn reanchor(&self, st: &mut State, thread: ThreadId, location: &Location) -> Result<ThreadView> {
        let side = match &find_thread(st, thread)?.target {
            CommentTarget::Range(a) => a.side,
            _ => return Err(SessionError::Invalid("only a range thread has an anchor".into())),
        };
        let target = CommentTarget::Range(self.anchor_at(st, side, &location.file, location.range).await?);
        self.acquisition.store.set_thread_target(thread, target.clone()).await?;
        edit_thread(st, thread, |t| t.target = target);
        self.remap(st).await?;
        thread_by_id(st, thread)
    }

    /// Copies into the store the content that recorded spans and range threads
    /// are anchored in and only git holds, so their coverage survives git
    /// dropping the objects of a base the session left, and a restart.
    async fn retain_anchored(&self, st: &State) -> Result<()> {
        let wanted: BTreeSet<(SnapshotId, &RepoPath)> = st.anchors().map(|a| (a.snapshot, &a.file)).collect();
        let store = &self.acquisition.store;
        let mut stored: HashMap<SnapshotId, Option<Snapshot>> = HashMap::new();
        for (id, file) in wanted {
            let snapshot = if let Some(held) = st.snapshot(id) {
                Some(held)
            } else {
                if let std::collections::hash_map::Entry::Vacant(v) = stored.entry(id) {
                    v.insert(store.snapshot(id).await?.map(|(s, _)| s));
                }
                stored[&id].as_ref()
            };
            let Some(entry) = snapshot
                .and_then(|s| s.entries.get(file))
                .filter(|e| e.kind == EntryKind::File)
            else {
                continue;
            };
            self.acquisition.keep(file, entry.blob).await?;
        }
        Ok(())
    }

    /// An anchor on `range` of `path` in the current snapshot of `side`.
    async fn anchor_at(&self, st: &State, side: Side, path: &RepoPath, range: LineRange) -> Result<Anchor> {
        let snapshot = side_snapshot(st, side);
        let entry = snapshot
            .entries
            .get(path)
            .filter(|e| e.kind == EntryKind::File)
            .ok_or_else(|| SessionError::NotFound(format!("file {}", path.to_string_lossy())))?;
        let mut blobs = BTreeMap::new();
        if !self.fetch(&mut blobs, path, entry.blob, true).await? || is_binary(&blobs[&entry.blob]) {
            return Err(SessionError::NotText(path.to_string_lossy()));
        }
        Anchor::from_text(side, snapshot.id, path.clone(), &blobs[&entry.blob], range)
            .ok_or_else(|| SessionError::Invalid("the lines are outside the file".into()))
    }

    async fn ensure_changes(&self, st: &mut State) -> Result<()> {
        if st.changes.is_some() {
            return Ok(());
        }
        let diff = self
            .acquisition
            .git
            .diff(&self.acquisition.store, &st.base, &st.target)
            .await?;
        let mut blobs = BTreeMap::new();
        for d in &diff.files {
            for (path, blob) in [(&d.old_path, d.old_blob), (&d.new_path, d.new_blob)] {
                if let (Some(path), Some(blob)) = (path, blob) {
                    self.load(&mut blobs, path, blob).await?;
                }
            }
        }
        let mut items = Vec::new();
        for n in &diff.non_text {
            let before_path = n.from.as_ref().unwrap_or(&n.path);
            items.push(ItemView {
                kind: n.kind,
                before: self.describe(before_path, n.before.as_ref(), n.kind).await,
                after: self.describe(&n.path, n.after.as_ref(), n.kind).await,
            });
        }
        let changed = diff
            .files
            .iter()
            .flat_map(|d| [&d.old_path, &d.new_path])
            .flatten()
            .chain(
                diff.non_text
                    .iter()
                    .flat_map(|n| [Some(&n.path), n.from.as_ref()])
                    .flatten(),
            )
            .cloned()
            .collect();
        st.changes = Some(Changes {
            diff,
            changed,
            blobs,
            remapped: Vec::new(),
            flat: Vec::new(),
            resolutions: HashMap::new(),
            items,
        });
        self.remap(st).await
    }

    /// Unavailable content leaves `target` and `size` empty.
    async fn describe(&self, path: &RepoPath, entry: Option<&FileEntry>, kind: NonTextKind) -> Option<EntryInfo> {
        let entry = entry?;
        if entry.kind == EntryKind::Submodule {
            return Some(EntryInfo {
                entry: entry.clone(),
                target: self.acquisition.git.submodule_commit(entry.blob).await.ok().flatten(),
                size: None,
            });
        }
        let wanted = match entry.kind {
            EntryKind::Symlink => true,
            EntryKind::File => matches!(
                kind,
                NonTextKind::Binary | NonTextKind::Mode | NonTextKind::Rename | NonTextKind::Empty
            ),
            EntryKind::Submodule => false,
        };
        let mut blobs = BTreeMap::new();
        let loaded = wanted && self.load(&mut blobs, path, entry.blob).await.unwrap_or(false);
        let bytes = blobs.get(&entry.blob).filter(|_| loaded);
        Some(EntryInfo {
            entry: entry.clone(),
            target: bytes
                .filter(|_| entry.kind == EntryKind::Symlink)
                .map(|b| String::from_utf8_lossy(b).into_owned()),
            size: bytes.filter(|_| entry.kind == EntryKind::File).map(|b| b.len() as u64),
        })
    }

    /// Content from the store, else from git or the worktree. `false` if unavailable.
    async fn load(&self, blobs: &mut BTreeMap<BlobId, Vec<u8>>, path: &RepoPath, blob: BlobId) -> Result<bool> {
        self.fetch(blobs, path, blob, false).await
    }

    /// Like `load`; with `capped`, git content over the capture cap is not read.
    async fn fetch(
        &self,
        blobs: &mut BTreeMap<BlobId, Vec<u8>>,
        path: &RepoPath,
        blob: BlobId,
        capped: bool,
    ) -> Result<bool> {
        if blobs.contains_key(&blob) {
            return Ok(true);
        }
        Ok(match self.acquisition.blob(path, blob, capped).await? {
            BlobRead::Found(bytes) => {
                blobs.insert(blob, bytes);
                true
            }
            BlobRead::TooLarge | BlobRead::Missing => false,
        })
    }

    /// The text of a blob, or why there is none. Content over the capture cap is
    /// not read.
    async fn text_of(
        &self,
        blobs: &mut BTreeMap<BlobId, Vec<u8>>,
        path: &RepoPath,
        blob: BlobId,
    ) -> Result<std::result::Result<String, NoText>> {
        Ok(if !self.fetch(blobs, path, blob, true).await? {
            Err(NoText::Unavailable)
        } else {
            decode_text(&blobs[&blob]).ok_or(NoText::Binary)
        })
    }

    /// Remaps every recorded span onto the current changeset (§5.2) and
    /// resolves every range thread's anchor (§5.1), each from what was recorded.
    async fn remap(&self, st: &mut State) -> Result<()> {
        let Prepared { inputs, spans, threads } = self.prepare(st).await?;
        let to = Changeset {
            base: st.base.id,
            target: st.target.id,
        };
        let changes = st.changes.as_mut().expect("diffed");
        let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &changes.blobs)).collect();

        // Moved content is searched for among the changed files of the anchor's side.
        let (base_inputs, target_inputs) = if threads.iter().any(Option::is_some) {
            changed_inputs(&changes.diff)
        } else {
            Default::default()
        };
        let base_changed: Vec<PreparedFile> = (base_inputs.iter())
            .map(|i| PreparedFile::new(i, &changes.blobs))
            .collect();
        let target_changed: Vec<PreparedFile> = (target_inputs.iter())
            .map(|i| PreparedFile::new(i, &changes.blobs))
            .collect();
        let resolutions: HashMap<ThreadId, Resolution> = st
            .threads
            .iter()
            .zip(&threads)
            .filter_map(|(t, slot)| match (&t.target, slot) {
                (CommentTarget::Range(a), Some(i)) => {
                    let changed = if a.side == Side::Base {
                        &base_changed
                    } else {
                        &target_changed
                    };
                    Some((t.id, resolve_anchor(a, &prepared[*i], changed)))
                }
                _ => None,
            })
            .collect();
        drop((base_changed, target_changed));

        let slots = spans;
        let remapped: Vec<(SpanId, Vec<ReviewedSpan>)> = st
            .recorded
            .iter()
            .zip(&slots)
            .map(|(rec, &(b, t))| {
                (
                    rec.id,
                    remap_span(&rec.span, to, b.map(|i| &prepared[i]), t.map(|i| &prepared[i])),
                )
            })
            .collect();
        drop(prepared);
        changes.flat = remapped.iter().flat_map(|(_, s)| s.iter().cloned()).collect();
        changes.remapped = remapped;
        changes.resolutions = resolutions;
        Ok(())
    }

    /// The file pairs of the anchors' sides, and for each recorded span and
    /// thread the indices of its pairs. Loads the texts into `changes.blobs`.
    async fn prepare(&self, st: &mut State) -> Result<Prepared> {
        let anchors = |r: &RecordedSpan| {
            [(true, r.span.base.clone()), (false, r.span.target.clone())]
                .into_iter()
                .filter_map(|(base, a)| a.map(|a| (base, a)))
                .collect::<Vec<_>>()
        };
        let thread_anchor = |t: &Thread| match &t.target {
            CommentTarget::Range(a) => Some((a.side == Side::Base, a.clone())),
            _ => None,
        };
        let referenced: Vec<SnapshotId> = st.anchors().map(|a| a.snapshot).collect();
        for &id in &referenced {
            if st.snapshot(id).is_none()
                && let Some((snapshot, _)) = self.acquisition.store.snapshot(id).await?
            {
                st.known.insert(id, snapshot);
            }
        }
        st.known.retain(|id, _| referenced.contains(id));

        let mut blobs = std::mem::take(&mut st.changes.as_mut().expect("diffed").blobs);
        let renames = &st.changes.as_ref().expect("diffed").diff.renames;
        let file_entry = |snapshot: Option<&Snapshot>, path: &RepoPath| {
            snapshot
                .and_then(|s| s.entries.get(path))
                .filter(|e| e.kind == EntryKind::File)
                .map(|e| e.blob)
        };
        // One prepared file per anchored file and side.
        let mut index: HashMap<(bool, RepoPath, SnapshotId), usize> = HashMap::new();
        let mut inputs: Vec<FileRemapInput> = Vec::new();
        let mut at = |is_base: bool, a: &Anchor| {
            let key = (is_base, a.file.clone(), a.snapshot);
            *index.entry(key).or_insert_with(|| {
                // The base side stays at its path; the target side follows a
                // rename from `HEAD` if its file is gone.
                let new_path = if is_base || st.target.entries.contains_key(&a.file) {
                    Some(a.file.clone())
                } else {
                    renames.0.get(&a.file).cloned()
                };
                let current = if is_base { &st.base } else { &st.target };
                inputs.push(FileRemapInput {
                    old_blob: file_entry(st.snapshot(a.snapshot), &a.file),
                    new_blob: new_path.as_ref().and_then(|p| file_entry(Some(current), p)),
                    old_path: Some(a.file.clone()),
                    new_path,
                });
                inputs.len() - 1
            })
        };
        let mut spans: Vec<Slot> = Vec::new();
        for rec in &st.recorded {
            let mut slot = [None, None];
            for (is_base, a) in anchors(rec) {
                slot[if is_base { 0 } else { 1 }] = Some(at(is_base, &a));
            }
            spans.push((slot[0], slot[1]));
        }
        let threads = st
            .threads
            .iter()
            .map(|t| thread_anchor(t).map(|(is_base, a)| at(is_base, &a)))
            .collect();
        let mut loaded = Ok(());
        for input in &inputs {
            for (path, blob) in [(&input.old_path, input.old_blob), (&input.new_path, input.new_blob)] {
                if let (Some(path), Some(blob)) = (path, blob)
                    && let Err(e) = self.load(&mut blobs, path, blob).await
                {
                    loaded = Err(e);
                }
            }
        }
        st.changes.as_mut().expect("diffed").blobs = blobs;
        loaded.map(|()| Prepared { inputs, spans, threads })
    }
}

/// Indices into the prepared files of a recorded span's base and target side.
type Slot = (Option<usize>, Option<usize>);

struct Prepared {
    inputs: Vec<FileRemapInput>,
    /// One per recorded span.
    spans: Vec<Slot>,
    /// One per thread; `None` unless it is a range thread.
    threads: Vec<Option<usize>>,
}

/// The changed text files of the diff as seen on the base side and on the
/// target side, for finding moved content.
fn changed_inputs(diff: &SnapshotDiff) -> (Vec<FileRemapInput>, Vec<FileRemapInput>) {
    let (mut base, mut target) = (Vec::new(), Vec::new());
    for d in &diff.files {
        if let (Some(path), Some(blob)) = (&d.old_path, d.old_blob) {
            base.push(FileRemapInput {
                old_path: Some(path.clone()),
                new_path: Some(path.clone()),
                old_blob: Some(blob),
                new_blob: Some(blob),
            });
        }
        if d.new_path.is_some() && d.new_blob.is_some() {
            target.push(FileRemapInput {
                old_path: d.old_path.clone(),
                new_path: d.new_path.clone(),
                old_blob: d.old_blob,
                new_blob: d.new_blob,
            });
        }
    }
    (base, target)
}

fn side_snapshot(st: &State, side: Side) -> &Snapshot {
    match side {
        Side::Base => &st.base,
        Side::Target => &st.target,
    }
}

fn find_thread(st: &State, id: ThreadId) -> Result<&Thread> {
    st.threads
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| SessionError::NotFound(format!("thread {}", id.0)))
}

fn edit_thread(st: &mut State, id: ThreadId, edit: impl FnOnce(&mut Thread)) {
    if let Some(t) = st.threads.iter_mut().find(|t| t.id == id) {
        edit(t);
    }
}

/// The resolution of a range thread at the current revision.
fn resolution_of(st: &State, id: ThreadId) -> Result<&Resolution> {
    find_thread(st, id)?;
    let changes = st.changes.as_ref().expect("computed");
    changes
        .resolutions
        .get(&id)
        .ok_or_else(|| SessionError::Invalid("only a range thread has an anchor".into()))
}

fn thread_by_id(st: &State, id: ThreadId) -> Result<ThreadView> {
    Ok(thread_view(st, find_thread(st, id)?))
}

/// File threads follow a git-detected rename and detach only when the file is
/// gone; changeset threads never detach.
fn thread_view(st: &State, t: &Thread) -> ThreadView {
    let changes = st.changes.as_ref().expect("computed");
    let (path, state, location) = match &t.target {
        CommentTarget::Range(a) => {
            let r = &changes.resolutions[&t.id];
            let path = r.location.as_ref().map_or(&a.file, |l| &l.file);
            (Some(path.clone()), Some(r.state.clone()), r.location.clone())
        }
        CommentTarget::File { side, path } => {
            let moved = (*side == Side::Target && !st.target.entries.contains_key(path))
                .then(|| changes.diff.renames.0.get(path))
                .flatten()
                .filter(|new| st.target.entries.contains_key(*new));
            let state = if moved.is_some() || side_snapshot(st, *side).entries.contains_key(path) {
                AnchorState::Exact {
                    whitespace_changed: false,
                }
            } else {
                AnchorState::Stale {
                    reason: StaleReason::FileDeleted,
                }
            };
            (Some(moved.unwrap_or(path).clone()), Some(state), None)
        }
        CommentTarget::Changeset => (None, None, None),
    };
    let target_line = match (&t.target, &location) {
        (CommentTarget::Range(a), Some(l)) => Some(card_line(changes, a.side, l)),
        _ => None,
    };
    ThreadView {
        id: t.id,
        target: t.target.clone(),
        path,
        state,
        location,
        target_line,
        messages: t.messages.clone(),
        status: t.status,
    }
}

/// The target line a range thread's card is shown below: the line its range ends on, as the
/// target text has it, but not past the last line of the file.
fn card_line(changes: &Changes, side: Side, location: &Location) -> u32 {
    let diff = changes.diff.files.iter().find(|d| {
        let path = if side == Side::Base { &d.old_path } else { &d.new_path };
        path.as_ref() == Some(&location.file)
    });
    let hunks = diff.map_or(&[][..], |d| &d.hunks[..]);
    let last = location.range.end.saturating_sub(1).max(location.range.start);
    let line = target_line(hunks, side, last);
    // A deleted file has no target text; an unchanged one is not in the diff and cannot overrun.
    // The empty line after a final line break holds no text, so no card sits below it.
    let last = diff.map(|d| {
        d.new_blob
            .and_then(|b| changes.blobs.get(&b))
            .map_or(0, |t| last_text_line(t))
    });
    last.map_or(line, |last| line.min(last))
}

fn lifecycle_of(st: &State) -> Lifecycle {
    match &st.review {
        None => Lifecycle::Browsing,
        Some(r) => Lifecycle::Open {
            version: r.version,
            updated_at: r.updated_at,
        },
    }
}

fn check_revision(st: &State, revision: u32) -> Result<()> {
    if revision == st.revision {
        Ok(())
    } else {
        Err(SessionError::Stale { current: st.revision })
    }
}

fn find_diff<'a>(diff: &'a SnapshotDiff, path: &str) -> Option<&'a FileDiff> {
    diff.files.iter().find(|d| d.path().to_string_lossy() == path)
}

/// The changes of `diff` that are acknowledged.
fn acknowledged(st: &State, diff: &SnapshotDiff) -> Vec<NonTextChange> {
    diff.non_text
        .iter()
        .filter(|n| st.acks.contains(&ack_of(n)))
        .cloned()
        .collect()
}

fn status_of(diff: Option<&FileDiff>, items: &[NonTextChange]) -> FileStatus {
    match diff {
        Some(d) => match (&d.old_path, &d.new_path) {
            (None, _) => FileStatus::Added,
            (_, None) => FileStatus::Deleted,
            (Some(old), Some(new)) if old != new => FileStatus::Renamed,
            _ => FileStatus::Modified,
        },
        None => match items.first() {
            Some(n) if n.before.is_none() => FileStatus::Added,
            Some(n) if n.after.is_none() => FileStatus::Deleted,
            Some(n) if n.from.is_some() => FileStatus::Renamed,
            _ => FileStatus::Modified,
        },
    }
}

fn review_of(cov: &Coverage, diff: Option<&FileDiff>, items: &[NonTextChange], acked: &[NonTextChange]) -> ReviewView {
    ReviewView {
        hunks: diff.map(|d| cov.hunk_states(d)).unwrap_or_default(),
        acknowledged: items.iter().filter(|n| acked.contains(n)).map(|n| n.kind).collect(),
    }
}

/// The review state of one file and the changeset's completion.
fn update_of(st: &State, path: &str) -> Result<ReviewUpdateView> {
    let changes = st.changes.as_ref().expect("computed");
    let cov = Coverage::new(&changes.flat);
    let acked = acknowledged(st, &changes.diff);
    let compiled = st.filter.compile();
    let diff = find_diff(&changes.diff, path);
    let items: Vec<NonTextChange> = changes
        .diff
        .non_text
        .iter()
        .filter(|n| n.path.to_string_lossy() == path && compiled.shows(n))
        .cloned()
        .collect();
    let file = diff
        .map(|d| d.path().clone())
        .or_else(|| items.first().map(|n| n.path.clone()))
        .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    Ok(ReviewUpdateView {
        revision: st.revision,
        path: file,
        review: review_of(&cov, diff, &items, &acked),
        completion: file_completion(&cov, &st.filter, diff, &items, &acked),
        changeset: changeset_completion(&cov, &st.filter, &changes.diff, &acked),
    })
}

/// The completion of one file: its text diff, if it has one, and its shown non-text items.
fn file_completion(
    cov: &Coverage,
    filter: &ReviewFilter,
    diff: Option<&FileDiff>,
    items: &[NonTextChange],
    acked: &[NonTextChange],
) -> Completion {
    cov.completion(filter, diff.map(slice::from_ref).unwrap_or_default(), items, acked)
}

/// The completion of everything the diff changes, as far as the filter shows it.
fn changeset_completion(
    cov: &Coverage,
    filter: &ReviewFilter,
    diff: &SnapshotDiff,
    acked: &[NonTextChange],
) -> Completion {
    cov.completion(filter, &diff.files, &diff.non_text, acked)
}

fn find_non_text<'a>(diff: &'a SnapshotDiff, kind: NonTextKind, path: &str) -> Option<&'a NonTextChange> {
    diff.non_text
        .iter()
        .find(|n| n.kind == kind && n.path.to_string_lossy() == path)
}

#[cfg(test)]
mod tests;
