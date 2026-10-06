//! A live review session of a dirty tree (§5, §6.1, §6.2).
//!
//! **Changeset.** The session pins `HEAD` and dirty-at-open (§6.1) and stores
//! the changeset `HEAD → dirty-at-open`, but what it displays is `HEAD → the
//! latest settled snapshot`, from the session's base: the pin until
//! `set_base` chooses another (§6.5). The review is of
//! everything the agent has done to the tree, so reviewing the live part
//! (`dirty-at-open → now`) alone would hide reviewed work behind a changeset
//! that has no review state of its own. Marks recorded against earlier targets
//! carry over by remapping (§5.2), so the live part is exactly what shows up
//! as unseen. Showing it separately is an interdiff, which belongs to the
//! timeline (M1).
//!
//! **Revision.** Starts at 0 and increases by one with each settled snapshot.
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
    BlobId, Changeset, EntryKind, FileDiff, FileEntry, Hunk, LineRange, NonTextChange, NonTextKind, RepoPath, Snapshot,
    SnapshotDiff, SnapshotId, SnapshotStatus, image_media_type,
};
use vet_core::filter::ReviewFilter;
use vet_core::marking::{hunk_regions, hunk_span};
use vet_core::prepared::PreparedFile;
use vet_core::remap::{cut_recorded, remap_span};
use vet_core::resolve::{Resolution, resolve_anchor};
use vet_core::review::{
    Anchor, AnchorState, CommentTarget, FileRemapInput, Location, Message, ReviewedSpan, Side, StaleReason, Thread,
    ThreadId, ThreadStatus,
};

use crate::acquisition::{Acquisition, SharedAcquisition};
use crate::execution::Execution;
use crate::execution::check::RunCheckConfig;
use crate::pin::{BaseChoice, CommitId};
use crate::ports::{
    BaseCandidates, BlobRead, Clock, Degraded, Dirty, GitError, GitRepo, ReviewStore, Scan, Store, StoreError, Watcher,
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

type Result<T> = std::result::Result<T, SessionError>;

/// How many recent commits a base can be picked from.
const MAX_BASE_COMMITS: usize = 200;

/// Display form of a path; invalid UTF-8 is replaced.
pub fn path_str(path: &RepoPath) -> String {
    String::from_utf8_lossy(&path.0).into_owned()
}

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
    pub non_text: Vec<ItemView>,
    pub review: ReviewView,
    pub completion: Completion,
}

/// The session's base as chosen and as resolved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BaseView {
    pub choice: BaseChoice,
    pub commit: CommitId,
    /// What the choice is, without the commit: `pin`, `merge-base <ref>`, `commit`.
    pub label: String,
}

/// The changeset through the review filter; hidden files are only counted in
/// `completion`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ManifestView {
    pub revision: u32,
    pub name: String,
    pub base: BaseView,
    pub filter: ReviewFilter,
    pub files: Vec<FileView>,
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

/// The two snapshots of a revision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SemanticSides {
    pub base: SnapshotId,
    pub target: SnapshotId,
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

struct State {
    revision: u32,
    filter: ReviewFilter,
    base: Snapshot,
    base_choice: BaseChoice,
    /// The commit `base` is the tree of.
    base_commit: CommitId,
    target: Snapshot,
    /// Other snapshots that recorded spans refer to.
    known: HashMap<SnapshotId, Snapshot>,
    recorded: Vec<RecordedSpan>,
    acks: Vec<NonTextAck>,
    threads: Vec<Thread>,
    degraded: Option<Degraded>,
    changes: Option<Changes>,
}

pub struct LiveSession<G, S> {
    pub id: SessionId,
    /// Author of the messages written here: git's `user.name`, else "You".
    author: String,
    /// Shared with the session's semantic pool, which materializes snapshots from it.
    acquisition: Arc<Acquisition<G, S>>,
    /// The pinned commit every target snapshot of the session is built on.
    head: CommitId,
    /// The tree of `head`: the base while the choice is the pin.
    pin_base: Snapshot,
    name: String,
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

/// Git's `user.name`, else "You".
async fn author_of<G: GitRepo>(git: &G) -> String {
    git.user_name()
        .await
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "You".into())
}

fn diff_path(d: &FileDiff) -> &RepoPath {
    d.new_path.as_ref().or(d.old_path.as_ref()).expect("a diff has a path")
}

fn line_count(bytes: &[u8]) -> u32 {
    bytes.split_inclusive(|&b| b == b'\n').count() as u32
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8000)].contains(&0)
}

impl<G: GitRepo + Sync, S: Store + ReviewStore + Sync> LiveSession<G, S> {
    /// Resumes the open session of the repository at `repo` (the root, as the
    /// key sessions are recorded under), or else pins the tree and records a
    /// session with its first snapshots (§6.1). `now` is unix seconds, for
    /// closing a session that cannot be resumed.
    pub async fn open(git: G, store: S, name: String, repo: String, now: i64) -> Result<Self> {
        let acquisition = Arc::new(Acquisition::new(git, store));
        if let Some(&id) = acquisition.store.open_sessions(repo.clone()).await?.last() {
            if let Some(session) = Self::resume(&acquisition, id, &name).await? {
                return Ok(session);
            }
            // Its pin or snapshots are gone: start over rather than fail to open.
            acquisition.git.delete_session_refs(id).await?;
            acquisition.store.close_session(id, now).await?;
        }
        let opened = acquisition.open_pin().await?;
        let store = &acquisition.store;
        store
            .put_snapshot(opened.base.clone(), SnapshotStatus::of(&opened.base))
            .await?;
        store.put_snapshot(opened.target.clone(), opened.status).await?;
        let pin = store.put_pin(opened.head.clone(), opened.target.id).await?;
        let filter = ReviewFilter::default();
        let changeset = Changeset {
            base: opened.base.id,
            target: opened.target.id,
        };
        let id = store.open_session(repo, pin.id, changeset, filter.clone()).await?;
        store.append_timeline(id, opened.target.id).await?;
        // Every snapshot of the session is built on the pinned commit.
        for name in ["base", "target"] {
            acquisition.git.set_session_ref(id, name, &opened.head).await?;
        }
        let state = State {
            revision: 0,
            filter,
            base: opened.base.clone(),
            base_choice: BaseChoice::Pin,
            base_commit: opened.head.clone(),
            target: opened.target,
            known: HashMap::new(),
            recorded: Vec::new(),
            acks: Vec::new(),
            threads: Vec::new(),
            degraded: None,
            changes: None,
        };
        Ok(Self {
            id,
            author: author_of(&acquisition.git).await,
            acquisition,
            head: opened.head,
            pin_base: opened.base,
            name,
            state: Mutex::new(state),
        })
    }

    /// Loads the review state of the stored session `id` and settles the tree as
    /// it is now, so recorded spans and threads remap onto whatever changed
    /// while Vet was not running (I3). `None` if the session cannot be
    /// reconstructed: its pin, its latest snapshot or the pinned commit is gone.
    async fn resume(acquisition: &Arc<Acquisition<G, S>>, id: SessionId, name: &str) -> Result<Option<Self>> {
        let store = &acquisition.store;
        let Some(stored) = store.session(id).await? else {
            return Ok(None);
        };
        let Some(pin) = store.pin(stored.pin).await? else {
            return Ok(None);
        };
        let timeline = store.timeline(id).await?;
        let latest = timeline.last().copied().unwrap_or(stored.changeset.target);
        let Some((target, _)) = store.snapshot(latest).await? else {
            return Ok(None);
        };
        let Ok(pin_base) = acquisition.git.commit_snapshot(&pin.head).await else {
            return Ok(None);
        };
        // The session keeps its pin however far `HEAD` has moved since: commits
        // made while Vet was closed are part of what is under review, and
        // re-pinning would turn every mark against the old base into
        // `SeenAgainstOldBase`. Moving the base is the user's call (§6.5).
        for snapshot in timeline.iter().copied().chain([pin_base.id]) {
            acquisition.record_commit(snapshot, pin.head.clone());
        }
        for name in ["base", "target"] {
            acquisition.git.set_session_ref(id, name, &pin.head).await?;
        }
        let state = State {
            revision: 0,
            filter: stored.filter,
            base: pin_base.clone(),
            base_choice: BaseChoice::Pin,
            base_commit: pin.head.clone(),
            target,
            known: HashMap::new(),
            recorded: stored.spans,
            acks: stored.acks,
            threads: stored.threads,
            degraded: None,
            changes: None,
        };
        let session = Self {
            id,
            author: author_of(&acquisition.git).await,
            acquisition: acquisition.clone(),
            head: pin.head,
            pin_base,
            name: name.to_owned(),
            state: Mutex::new(state),
        };
        // A merge base follows its reference as it is now; one that no longer
        // resolves falls back to the pin.
        if stored.base != BaseChoice::Pin && session.set_base(stored.base).await.is_err() {
            store.set_base(id, session.pin_base.id, BaseChoice::Pin).await?;
        }
        let scan = acquisition.reconcile(Dirty::All).await?;
        if scan.snapshot.id != session.settled_snapshot().await {
            let (revision, retained) = session.settled(scan).await?;
            if let Some(e) = retained {
                eprintln!("vet: could not keep the content of revision {revision}: {}", e.0);
            }
        }
        Ok(Some(session))
    }

    /// Releases the session's refs and records it closed. `now` is unix seconds.
    pub async fn close(&self, now: i64) -> Result<()> {
        self.acquisition.git.delete_session_refs(self.id).await?;
        Ok(self.acquisition.store.close_session(self.id, now).await?)
    }

    /// Changes what the target is compared against; the target stays the live
    /// tree. Moves the session's base ref, and bumps the revision unless nothing
    /// changed. Recorded spans are untouched: the next manifest remaps them onto
    /// the new base, so base-side changes in reviewed regions show as
    /// `SeenAgainstOldBase` (§5.2), and choosing an earlier base again restores
    /// the earlier coverage. Returns the revision.
    pub async fn set_base(&self, choice: BaseChoice) -> Result<(BaseView, u32)> {
        let git = &self.acquisition.git;
        let base_err = |e: GitError| SessionError::Base(e.0);
        let (choice, commit) = match choice {
            BaseChoice::Pin => (BaseChoice::Pin, self.head.clone()),
            BaseChoice::MergeBase { reference } => {
                let commit = git.merge_base(&reference).await.map_err(base_err)?;
                (BaseChoice::MergeBase { reference }, commit)
            }
            BaseChoice::Commit { id } => {
                let commit = git.resolve_commit(&id.0).await.map_err(base_err)?;
                (BaseChoice::Commit { id: commit.clone() }, commit)
            }
        };
        let snapshot = match choice {
            BaseChoice::Pin => self.pin_base.clone(),
            _ => git.commit_snapshot(&commit).await?,
        };

        let mut st = self.state.lock().await;
        if st.base_choice == choice && st.base.id == snapshot.id {
            return Ok((base_view(&st), st.revision));
        }
        let store = &self.acquisition.store;
        // Before the ref moves: git may drop the old base's objects afterwards.
        self.retain_anchored(&st).await?;
        store
            .put_snapshot(snapshot.clone(), SnapshotStatus::of(&snapshot))
            .await?;
        git.set_session_ref(self.id, "base", &commit).await?;
        self.acquisition.record_commit(snapshot.id, commit.clone());
        store.set_base(self.id, snapshot.id, choice.clone()).await?;
        st.base = snapshot;
        st.base_choice = choice;
        st.base_commit = commit;
        st.revision += 1;
        st.changes = None;
        Ok((base_view(&st), st.revision))
    }

    /// Replaces the review filter and bumps the revision; completion and the
    /// excluded counts follow with the next manifest. Returns the revision.
    pub async fn set_filter(&self, filter: ReviewFilter) -> Result<u32> {
        filter.validate().map_err(SessionError::Invalid)?;
        let mut st = self.state.lock().await;
        if st.filter == filter {
            return Ok(st.revision);
        }
        self.acquisition.store.set_filter(self.id, filter.clone()).await?;
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

    /// What serves the session's snapshots to its language-server pool.
    pub fn materializer(&self) -> SharedAcquisition<G, S> {
        SharedAcquisition(self.acquisition.clone())
    }

    /// The snapshots of `revision`: what the semantic layer asks about. Fails
    /// if `revision` is no longer current.
    pub async fn semantic_sides(&self, revision: u32) -> Result<SemanticSides> {
        let st = self.state.lock().await;
        check_revision(&st, revision)?;
        Ok(SemanticSides {
            base: st.base.id,
            target: st.target.id,
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

    /// Stores a settled snapshot, makes it the target and returns the new
    /// revision. Review state is remapped when the next manifest is computed.
    /// A failed copy of its content is not fatal: the diff then reads the
    /// worktree, or shows the file as unavailable.
    pub async fn settled(&self, scan: Scan) -> Result<(u32, Option<GitError>)> {
        let (base, previous) = {
            let st = self.state.lock().await;
            (st.base.clone(), st.target.clone())
        };
        let retained = self
            .acquisition
            .retain(&[&base, &previous], &scan.snapshot, &scan.status)
            .await
            .err();
        let store = &self.acquisition.store;
        store.put_snapshot(scan.snapshot.clone(), scan.status).await?;
        store.append_timeline(self.id, scan.snapshot.id).await?;
        // Older timeline snapshots stay materializable (§7.2).
        self.acquisition.record_commit(scan.snapshot.id, self.head.clone());
        let mut st = self.state.lock().await;
        let old = std::mem::replace(&mut st.target, scan.snapshot);
        st.known.insert(old.id, old);
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
                        eprintln!("vet: could not keep the content of revision {revision}: {}", e.0);
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
        let changes = st.changes.as_ref().expect("computed");
        let cov = Coverage::new(&changes.flat);
        let acked = acknowledged(&st, &changes.diff);
        let compiled = st.filter.compile();

        // Files by path, with their text diff and non-text items.
        type Parts<'a> = (Option<&'a FileDiff>, Vec<usize>);
        let mut by_path: BTreeMap<&RepoPath, Parts> = BTreeMap::new();
        for d in &changes.diff.files {
            by_path.entry(diff_path(d)).or_default().0 = Some(d);
        }
        for (i, n) in changes.diff.non_text.iter().enumerate() {
            by_path.entry(&n.path).or_default().1.push(i);
        }

        let lines = |blob: Option<BlobId>| blob.and_then(|b| changes.blobs.get(&b)).map_or(0, |t| line_count(t));
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
            files.push(FileView {
                path: path.clone(),
                old_path: diff
                    .and_then(|d| d.old_path.clone())
                    .or_else(|| items.iter().find_map(|n| n.from.clone()))
                    .filter(|old| old != path),
                status: status_of(diff, &items),
                base_lines: lines(diff.and_then(|d| d.old_blob)),
                target_lines: lines(diff.and_then(|d| d.new_blob)),
                hunks: diff.map(|d| d.hunks.clone()).unwrap_or_default(),
                review: review_of(&cov, diff, &items, &acked),
                completion: cov.completion(
                    &st.filter,
                    diff.map(slice::from_ref).unwrap_or_default(),
                    &items,
                    &acked,
                ),
                non_text: indices.iter().map(|&i| changes.items[i].clone()).collect(),
            });
        }
        Ok(ManifestView {
            revision: st.revision,
            name: self.name.clone(),
            base: base_view(&st),
            filter: st.filter.clone(),
            completion: cov.completion(&st.filter, &changes.diff.files, &changes.diff.non_text, &acked),
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
        let (old, new): (Side, Side) = if let Some(d) = find_diff(&changes.diff, path) {
            let side = |p: &Option<RepoPath>, b: Option<BlobId>| p.clone().zip(b);
            (side(&d.old_path, d.old_blob), side(&d.new_path, d.new_blob))
        } else if let Some(n) = changes.diff.non_text.iter().find(|n| path_str(&n.path) == path) {
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
                Ok(Err(_)) => result = Err(SessionError::NotText(path_str(&p))),
                Err(e) => result = Err(e),
            }
            if result.is_err() {
                break;
            }
        }
        st.changes.as_mut().expect("computed").blobs = blobs;
        result?;
        let [base, target] = texts;
        Ok(FileText {
            revision: st.revision,
            base,
            target,
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
        let key = RepoPath(path.as_bytes().to_vec());
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
    pub async fn set_seen(&self, revision: u32, path: &str, hunks: &[u32], seen: bool) -> Result<ReviewUpdateView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
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
        let ids = self
            .acquisition
            .store
            .edit_spans(self.id, remove.clone(), add.clone())
            .await?;
        st.recorded.retain(|r| !remove.contains(&r.id));
        st.recorded
            .extend(ids.into_iter().zip(add).map(|(id, span)| RecordedSpan { id, span }));
        self.remap(&mut st).await?;
        update_of(&st, path)
    }

    /// Acknowledges or un-acknowledges one non-text item.
    pub async fn acknowledge(
        &self,
        revision: u32,
        path: &str,
        kind: NonTextKind,
        acknowledged: bool,
    ) -> Result<ReviewUpdateView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let change = {
            let diff = &st.changes.as_ref().expect("computed").diff;
            diff.non_text
                .iter()
                .find(|n| n.kind == kind && path_str(&n.path) == path)
                .ok_or_else(|| SessionError::NotFound(format!("change of {path}")))?
        };
        let ack = ack_of(change);
        let store = &self.acquisition.store;
        if acknowledged {
            store.acknowledge(self.id, ack.clone()).await?;
            if !st.acks.contains(&ack) {
                st.acks.push(ack);
            }
        } else {
            store.unacknowledge(self.id, ack.clone()).await?;
            st.acks.retain(|a| *a != ack);
        }
        update_of(&st, path)
    }

    /// The bytes of one side of a non-text item's file at `revision`.
    pub async fn item_blob(&self, revision: u32, path: &str, kind: NonTextKind, side: Side) -> Result<ItemBlob> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let change = (st.changes.as_ref().expect("computed").diff.non_text.iter())
            .find(|n| n.kind == kind && path_str(&n.path) == path)
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
        let read = match self.acquisition.store.blob(entry.blob).await? {
            Some(bytes) => BlobRead::Found(bytes),
            None => self.acquisition.git.read_blob_capped(file, entry.blob).await?,
        };
        Ok(match read {
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

    /// Opens a thread with its first message. `now` is unix seconds.
    pub async fn create_thread(&self, revision: u32, target: NewThread, body: String, now: i64) -> Result<ThreadView> {
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
                    return Err(SessionError::NotFound(format!("file {}", path_str(&path))));
                }
                CommentTarget::File { side, path }
            }
            NewThread::Changeset => CommentTarget::Changeset,
        };
        let id = self
            .acquisition
            .store
            .add_thread(self.id, target.clone(), opener.clone())
            .await?;
        st.threads.push(Thread {
            id,
            target,
            messages: vec![opener],
            status: ThreadStatus::Open,
            platform: None,
        });
        self.remap(&mut st).await?;
        thread_by_id(&st, id)
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
        let thread_anchors = st.threads.iter().filter_map(|t| match &t.target {
            CommentTarget::Range(a) => Some(a),
            _ => None,
        });
        let anchors = (st.recorded.iter())
            .flat_map(|r| r.span.base.iter().chain(&r.span.target))
            .chain(thread_anchors);
        let wanted: BTreeSet<(SnapshotId, &RepoPath)> = anchors.map(|a| (a.snapshot, &a.file)).collect();
        let store = &self.acquisition.store;
        let mut stored: HashMap<SnapshotId, Option<Snapshot>> = HashMap::new();
        for (id, file) in wanted {
            let snapshot = if id == st.base.id {
                Some(&st.base)
            } else if id == st.target.id {
                Some(&st.target)
            } else if let Some(known) = st.known.get(&id) {
                Some(known)
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
            if store.blob(entry.blob).await?.is_none()
                && let Some(bytes) = self.acquisition.git.read_blob(file, entry.blob).await?
            {
                store.put_blob(bytes).await?;
            }
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
            .ok_or_else(|| SessionError::NotFound(format!("file {}", path_str(path))))?;
        let mut blobs = BTreeMap::new();
        if !self.fetch(&mut blobs, path, entry.blob, true).await? || is_binary(&blobs[&entry.blob]) {
            return Err(SessionError::NotText(path_str(path)));
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
        let read = match self.acquisition.store.blob(blob).await? {
            Some(bytes) => BlobRead::Found(bytes),
            None if capped => self.acquisition.git.read_blob_capped(path, blob).await?,
            None => match self.acquisition.git.read_blob(path, blob).await? {
                Some(bytes) => BlobRead::Found(bytes),
                None => BlobRead::Missing,
            },
        };
        Ok(match read {
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
        } else if is_binary(&blobs[&blob]) {
            Err(NoText::Binary)
        } else {
            Ok(String::from_utf8_lossy(&blobs[&blob]).into_owned())
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
        let referenced: Vec<SnapshotId> = st
            .recorded
            .iter()
            .flat_map(anchors)
            .chain(st.threads.iter().filter_map(thread_anchor))
            .map(|(_, a)| a.snapshot)
            .collect();
        for &id in &referenced {
            if id != st.base.id
                && id != st.target.id
                && !st.known.contains_key(&id)
                && let Some((snapshot, _)) = self.acquisition.store.snapshot(id).await?
            {
                st.known.insert(id, snapshot);
            }
        }
        st.known.retain(|id, _| referenced.contains(id));

        let mut blobs = std::mem::take(&mut st.changes.as_mut().expect("diffed").blobs);
        let renames = &st.changes.as_ref().expect("diffed").diff.renames;
        let snapshot_of = |id: SnapshotId| {
            if id == st.base.id {
                Some(&st.base)
            } else if id == st.target.id {
                Some(&st.target)
            } else {
                st.known.get(&id)
            }
        };
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
                    old_blob: file_entry(snapshot_of(a.snapshot), &a.file),
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
    ThreadView {
        id: t.id,
        target: t.target.clone(),
        path,
        state,
        location,
        messages: t.messages.clone(),
        status: t.status,
    }
}

fn base_view(st: &State) -> BaseView {
    BaseView {
        label: match &st.base_choice {
            BaseChoice::Pin => "pin".into(),
            BaseChoice::MergeBase { reference } => format!("merge-base {reference}"),
            BaseChoice::Commit { .. } => "commit".into(),
        },
        choice: st.base_choice.clone(),
        commit: st.base_commit.clone(),
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
    diff.files.iter().find(|d| path_str(diff_path(d)) == path)
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
        .filter(|n| path_str(&n.path) == path && compiled.shows(n))
        .cloned()
        .collect();
    let file = diff
        .map(|d| diff_path(d).clone())
        .or_else(|| items.first().map(|n| n.path.clone()))
        .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    Ok(ReviewUpdateView {
        revision: st.revision,
        path: file,
        review: review_of(&cov, diff, &items, &acked),
        completion: cov.completion(
            &st.filter,
            diff.map(slice::from_ref).unwrap_or_default(),
            &items,
            &acked,
        ),
        changeset: cov.completion(&st.filter, &changes.diff.files, &changes.diff.non_text, &acked),
    })
}

#[cfg(test)]
mod tests;
