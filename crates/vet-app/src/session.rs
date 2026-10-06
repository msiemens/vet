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

use std::collections::{BTreeMap, HashMap};
use std::{fmt, slice};

use tokio::sync::Mutex;
use vet_core::completion::{Completion, Coverage, HunkState};
use vet_core::content::{
    BlobId, Changeset, EntryKind, FileDiff, FileEntry, Hunk, NonTextChange, NonTextKind, RepoPath, Snapshot,
    SnapshotDiff, SnapshotId, SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::marking::{hunk_regions, hunk_span};
use vet_core::prepared::PreparedFile;
use vet_core::remap::{cut_recorded, remap_span};
use vet_core::review::{FileRemapInput, ReviewedSpan};

use crate::acquisition::Acquisition;
use crate::pin::{BaseChoice, CommitId};
use crate::ports::{BaseCandidates, Clock, Degraded, GitError, GitRepo, ReviewStore, Scan, Store, StoreError, Watcher};
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
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => write!(f, "git: {}", e.0),
            Self::Store(e) => write!(f, "store: {}", e.0),
            Self::Stale { current } => write!(f, "stale revision; the changeset is at revision {current}"),
            Self::NotFound(what) => write!(f, "no such {what}"),
            Self::NotText(path) => write!(f, "{path} has no text to show"),
            Self::Base(message) => f.write_str(message),
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

/// What the current revision's diff needs, computed on first use.
struct Changes {
    diff: SnapshotDiff,
    /// Text of the diffed files and of what recorded spans were marked in.
    blobs: BTreeMap<BlobId, Vec<u8>>,
    /// Recorded spans as they are now.
    remapped: Vec<(SpanId, Vec<ReviewedSpan>)>,
    flat: Vec<ReviewedSpan>,
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
    degraded: Option<Degraded>,
    changes: Option<Changes>,
}

pub struct LiveSession<G, S> {
    pub id: SessionId,
    acquisition: Acquisition<G, S>,
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
    /// Pins the tree, records the session and its first snapshots (§6.1).
    pub async fn open(git: G, store: S, name: String) -> Result<Self> {
        let acquisition = Acquisition::new(git, store);
        let opened = acquisition.open_pin().await?;
        let store = &acquisition.store;
        store
            .put_snapshot(opened.base.clone(), SnapshotStatus::default())
            .await?;
        store.put_snapshot(opened.target.clone(), opened.status).await?;
        let pin = store.put_pin(opened.head.clone(), opened.target.id).await?;
        let filter = ReviewFilter::default();
        let changeset = Changeset {
            base: opened.base.id,
            target: opened.target.id,
        };
        let id = store.open_session(pin.id, changeset, filter.clone()).await?;
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
            degraded: None,
            changes: None,
        };
        Ok(Self {
            id,
            acquisition,
            head: opened.head,
            pin_base: opened.base,
            name,
            state: Mutex::new(state),
        })
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
        store.put_snapshot(snapshot.clone(), SnapshotStatus::default()).await?;
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

    /// Branches and recent commits to pick a base from.
    pub async fn base_candidates(&self) -> Result<BaseCandidates> {
        Ok(self.acquisition.git.base_candidates(MAX_BASE_COMMITS).await?)
    }

    pub async fn revision(&self) -> u32 {
        self.state.lock().await.revision
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
            match self.load(&mut blobs, &p, b).await {
                Ok(true) if !is_binary(&blobs[&b]) => *text = Some(String::from_utf8_lossy(&blobs[&b]).into_owned()),
                Ok(_) => result = Err(SessionError::NotText(path_str(&p))),
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

    /// Marks hunks (indices into the file's hunks at `revision`) seen or unseen.
    pub async fn set_seen(&self, revision: u32, path: &str, hunks: &[u32], seen: bool) -> Result<ReviewUpdateView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let (inputs, slots) = self.prepare(&mut st).await?;
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
        st.changes = Some(Changes {
            diff,
            blobs,
            remapped: Vec::new(),
            flat: Vec::new(),
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
            EntryKind::File => matches!(kind, NonTextKind::Binary | NonTextKind::Mode | NonTextKind::Rename),
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
        if blobs.contains_key(&blob) {
            return Ok(true);
        }
        let bytes = match self.acquisition.store.blob(blob).await? {
            Some(bytes) => Some(bytes),
            None => self.acquisition.git.read_blob(path, blob).await?,
        };
        Ok(bytes.map(|b| blobs.insert(blob, b)).is_some())
    }

    /// Remaps every recorded span onto the current changeset (§5.2).
    async fn remap(&self, st: &mut State) -> Result<()> {
        let (inputs, slots) = self.prepare(st).await?;
        let to = Changeset {
            base: st.base.id,
            target: st.target.id,
        };
        let changes = st.changes.as_mut().expect("diffed");
        let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &changes.blobs)).collect();
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
        Ok(())
    }

    /// The file pairs of the recorded spans' sides, and for each span the
    /// indices of its base and target pair. Loads the texts into `changes.blobs`.
    async fn prepare(&self, st: &mut State) -> Result<(Vec<FileRemapInput>, Vec<Slot>)> {
        let anchors = |r: &RecordedSpan| {
            [(true, r.span.base.clone()), (false, r.span.target.clone())]
                .into_iter()
                .filter_map(|(base, a)| a.map(|a| (base, a)))
                .collect::<Vec<_>>()
        };
        for a in st.recorded.iter().flat_map(anchors).map(|(_, a)| a) {
            if a.snapshot != st.base.id
                && a.snapshot != st.target.id
                && !st.known.contains_key(&a.snapshot)
                && let Some((snapshot, _)) = self.acquisition.store.snapshot(a.snapshot).await?
            {
                st.known.insert(a.snapshot, snapshot);
            }
        }
        let referenced: Vec<SnapshotId> = st.recorded.iter().flat_map(anchors).map(|(_, a)| a.snapshot).collect();
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
        let mut slots: Vec<Slot> = Vec::new();
        for rec in &st.recorded {
            let mut slot = [None, None];
            for (is_base, a) in anchors(rec) {
                let key = (is_base, a.file.clone(), a.snapshot);
                let at = *index.entry(key).or_insert_with(|| {
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
                });
                slot[if is_base { 0 } else { 1 }] = Some(at);
            }
            slots.push((slot[0], slot[1]));
        }
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
        loaded.map(|()| (inputs, slots))
    }
}

/// Indices into the prepared files of a recorded span's base and target side.
type Slot = (Option<usize>, Option<usize>);

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
