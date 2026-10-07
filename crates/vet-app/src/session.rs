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
//!
//! This module is the lifecycle and persistence of a review. The rest of the
//! session is split by what it does with the state: `remap` computes a
//! revision's changes, `manifest` and `content` and `browse` show them, `marks`
//! and `threads` edit the review, and `blobs` reads the content they all need.

mod blobs;
mod browse;
mod content;
mod manifest;
mod marks;
mod remap;
mod state;
mod threads;
mod views;

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;
use vet_core::completion::Coverage;
use vet_core::content::{Changeset, EntryKind, FileDiff, RepoPath, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;

pub use self::blobs::NoText;
pub use self::browse::{DirEntryKind, DirItem, DirListing};
pub use self::content::{FileText, ItemBlob, MAX_ITEM_BLOB, TargetText};
use self::manifest::changeset_completion;
use self::state::{Review, State, acknowledged, check_revision, lifecycle_of};
pub use self::threads::{CommentAt, CommentSpot, NewThread, ThreadView, ThreadsView};
pub use self::views::{
    BaseView, EntryInfo, FileStatus, FileView, ItemView, Lifecycle, ManifestView, ModeChange, Outcome,
    ReviewUpdateView, ReviewView, SemanticSides, TargetView,
};
use crate::acquisition::{Acquisition, BlobError};
use crate::base;
use crate::pin::BaseChoice;
use crate::ports::{
    BaseCandidates, BranchInfo, Clock, Dirty, GitError, GitRepo, ReviewStore, Scan, Store, StoreError, Watcher,
};
use crate::progress::Tasks;
use crate::review::SessionId;
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
        Self::open_reporting(git, store, repo, clock, Tasks::default()).await
    }

    /// Like [`LiveSession::open`], with the scans and diffs of the session reporting to `tasks`.
    pub async fn open_reporting(git: G, store: S, repo: String, clock: UnixClock, tasks: Tasks) -> Result<Self> {
        let acquisition = Arc::new(Acquisition::new(git, store).reporting(tasks));
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
        self.replace_filter(&mut *self.state.lock().await, filter).await
    }

    /// Adds the file at `path`, or with `directory` everything below the folder at `path`, to the
    /// filter's excluded globs, and bumps the revision unless it was excluded already. Returns
    /// the revision.
    pub async fn exclude_path(&self, path: &str, directory: bool) -> Result<u32> {
        let mut st = self.state.lock().await;
        let mut filter = st.filter.clone();
        let glob = exclude_glob(path, directory);
        if !filter.exclude.contains(&glob) {
            filter.exclude.push(glob);
        }
        self.replace_filter(&mut st, filter).await
    }

    async fn replace_filter(&self, st: &mut State, filter: ReviewFilter) -> Result<u32> {
        let filter = filter.normalized();
        filter.validate().map_err(SessionError::Invalid)?;
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

    /// For each key `key_of` yields over the files of `revision`'s target
    /// snapshot, the first file with it; stops once `limit` keys are found.
    pub async fn target_file_per_key<K: PartialEq>(
        &self,
        revision: u32,
        limit: usize,
        key_of: impl Fn(&RepoPath) -> Option<K>,
    ) -> Result<Vec<(K, RepoPath)>> {
        let st = self.state.lock().await;
        check_revision(&st, revision)?;
        let mut found: Vec<(K, RepoPath)> = Vec::new();
        for path in st.target.entries.keys() {
            if found.len() >= limit {
                break;
            }
            if let Some(key) = key_of(path)
                && !found.iter().any(|(k, _)| *k == key)
            {
                found.push((key, path.clone()));
            }
        }
        Ok(found)
    }

    /// The text diff of `revision`, for the analyses that map symbols to hunks.
    pub async fn changed_files(&self, revision: u32) -> Result<Vec<FileDiff>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        Ok(st.changes.as_ref().expect("computed").diff.files.clone())
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
}

/// The filter glob for exactly the file at `path`, or everything below the folder at `path`:
/// anchored at the root, with the characters that are glob syntax matching themselves.
fn exclude_glob(path: &str, directory: bool) -> String {
    let mut glob = String::from("/");
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
            glob.extend(['[', c, ']']);
        } else {
            glob.push(c);
        }
    }
    if directory {
        glob.push_str("/**");
    }
    glob
}

#[cfg(test)]
pub(crate) mod tests;
