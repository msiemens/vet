//! A live session of a dirty tree (§5, §6.1, §6.2).
//!
//! **Lifecycle.** A session starts out browsing: the live diff against the
//! base, with nothing stored. The first review action (marking seen,
//! acknowledging, a new thread) starts the review: it records the base, `HEAD`
//! and the current target as the working tree changeset's version 1, and from
//! then on every settled snapshot and every other base is the next version.
//! Finishing releases the review's refs and returns to browsing (see SPEC
//! "Session lifecycle"); threads that are still open stay on view, because they
//! belong to the changeset, not to the review.
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
//! **Source.** One changeset is on view at a time: the working tree, a local
//! branch or a pull request (`source`). Switching parks the state of the one on
//! view in memory and takes up the other's, so what a changeset had (its review,
//! its marks, its threads) is as it was when it comes back; revisions only
//! increase across switches. A branch's or pull request's target is its tip
//! commit, not the live tree: it is built from git, never settles, and moves
//! only when `observe` sees the ref (the branch, or the pull request's head as
//! last fetched) or the merge base move.
//!
//! This module is the lifecycle and persistence of a review. The rest of the
//! session is split by what it does with the state: `remap` computes a
//! revision's changes, `manifest` and `content` and `browse` show them, `marks`
//! and `threads` edit the review, `blobs` reads the content they all need, and
//! `source` and `listing` are about which changeset is on view.

mod blobs;
mod browse;
mod content;
mod interdiff;
mod listing;
mod manifest;
mod marks;
mod remap;
mod source;
mod state;
mod threads;
mod versions;
mod views;

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;
use vet_core::completion::Coverage;
use vet_core::content::{EntryKind, FileDiff, RepoPath, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::review::{Anchor, CommentTarget, Thread, ThreadStatus};

pub use self::blobs::NoText;
pub use self::browse::{DirEntryKind, DirItem, DirListing};
pub use self::content::{FileText, ItemBlob, MAX_ITEM_BLOB, TargetText};
pub use self::interdiff::View;
pub use self::listing::{ChangesetList, ChangesetRow, Divergence, RowReview};
use self::manifest::changeset_completion;
use self::source::Stored;
use self::state::{OpenReview, State, acknowledged, check_revision, lifecycle_of};
pub use self::threads::{CommentAt, CommentSpot, NewThread, ThreadView, ThreadsView};
pub use self::versions::VersionView;
pub use self::views::{
    BaseView, EntryInfo, FileStatus, FileView, InterdiffView, ItemView, Lifecycle, ManifestView, ModeChange, Outcome,
    ReviewUpdateView, ReviewView, SemanticSides, TargetView,
};
use crate::acquisition::{Acquisition, BlobError};
use crate::base;
use crate::changeset::{BaseChoice, ChangesetId, Source, Version, VersionOrigin};
use crate::ports::{
    BaseCandidates, BranchInfo, Clock, Degraded, Dirty, GitError, GitRepo, LiveStore, Scan, StoreError, Watcher,
};
use crate::progress::Tasks;
use crate::review::{ReviewId, visible_threads};
use crate::revisions::Revisions;
use crate::settle::{Settled, Settling};

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
    /// The repository root, which with a source is the key a changeset is stored under.
    repo: String,
    clock: UnixClock,
    /// Serializes everything that reads or edits review state: of the changeset on view.
    state: Mutex<State>,
    /// The changesets that were on view and are not now, by source. Locked after `state`.
    parked: Mutex<HashMap<Source, State>>,
    /// Why the working tree is polled instead of watched, if it is.
    degraded: std::sync::Mutex<Option<Degraded>>,
    /// Told every time the revision on view moves, so that work for an older one stops.
    revisions: Arc<Revisions>,
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

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// Resumes the open review of the working tree of the repository at `repo`
    /// (the root, as the key changesets are recorded under), or else starts out
    /// browsing the tree against the current branch's tip. The threads that are
    /// still open from earlier reviews are on view either way.
    pub async fn open(git: G, store: S, repo: String, clock: UnixClock) -> Result<Self> {
        Self::open_reporting(git, store, repo, clock, Tasks::default()).await
    }

    /// Like [`LiveSession::open`], with the scans and diffs of the session reporting to `tasks`.
    pub async fn open_reporting(git: G, store: S, repo: String, clock: UnixClock, tasks: Tasks) -> Result<Self> {
        let acquisition = Arc::new(Acquisition::new(git, store).reporting(tasks));
        let filter = acquisition.store.filter(repo.clone()).await?.normalized();
        let name = name_of_repo(&repo);
        let source = Source::WorkingTree;
        let (changeset, threads) = match Self::stored(&acquisition, &repo, &source, &filter, clock).await? {
            Stored::Resumed(state) => {
                let session = Self::assemble(acquisition, name, repo, clock, *state).await;
                session.catch_up().await?;
                return Ok(session);
            }
            Stored::Browsing { changeset, threads } => (changeset, threads),
        };
        let git = &acquisition.git;
        let head = git.head().await?;
        // The base chosen last time, unless it no longer resolves (its branch is gone).
        let remembered = match acquisition.store.browsing_base(repo.clone()).await? {
            Some(choice) => base::resolve(git, choice, &head).await.ok(),
            None => None,
        };
        let (base_choice, base_commit) = match remembered {
            Some(found) => found,
            None => base::default_choice(&git.branches().await?, &head),
        };
        let base = git.commit_snapshot(&base_commit).await?;
        let Scan {
            snapshot: target,
            status,
        } = acquisition.reconcile(Dirty::All).await?;
        acquisition.record_commit(base.id, base_commit.clone());
        acquisition.record_commit(target.id, head.clone());
        let state = State {
            source,
            revision: 0,
            changeset,
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
            threads,
            changes: None,
        };
        Ok(Self::assemble(acquisition, name, repo, clock, state).await)
    }

    async fn assemble(
        acquisition: Arc<Acquisition<G, S>>,
        name: String,
        repo: String,
        clock: UnixClock,
        state: State,
    ) -> Self {
        Self {
            author: author_of(&acquisition.git).await,
            acquisition,
            name,
            repo,
            clock,
            state: Mutex::new(state),
            parked: Mutex::default(),
            degraded: std::sync::Mutex::new(None),
            revisions: Arc::default(),
        }
    }

    /// The state of the stored review `id` of `changeset`, a changeset of `source`.
    /// `None` if it cannot be reconstructed: its versions, its latest snapshot or its
    /// base commit is gone.
    async fn resumed(
        acquisition: &Acquisition<G, S>,
        changeset: ChangesetId,
        id: ReviewId,
        filter: ReviewFilter,
        source: Source,
    ) -> Result<Option<State>> {
        let store = &acquisition.store;
        let Some(review) = store.review(id).await? else {
            return Ok(None);
        };
        let versions = store.versions(changeset).await?;
        let Some(latest) = versions.last() else {
            return Ok(None);
        };
        let Some((target, target_status)) = store.snapshot(latest.target).await? else {
            return Ok(None);
        };
        let Ok(base) = acquisition.git.commit_snapshot(&latest.base_commit).await else {
            return Ok(None);
        };
        // The review keeps its base however far the branches have moved since:
        // commits made while Vet was closed are part of what is under review,
        // and re-resolving would turn every mark against the old base into
        // `SeenAgainstOldBase`. Moving the base is the user's call (§6.5).
        for (i, version) in versions.iter().enumerate() {
            acquisition.record_commit(version.target, version.tip.clone());
            acquisition.record_commit(version.base, version.base_commit.clone());
            let previous = i.checked_sub(1).map(|p| &versions[p]);
            Self::keep_commits(acquisition, changeset, version, previous).await?;
        }
        let threads = visible_threads(store.threads(changeset).await?, Some(id));
        Ok(Some(State {
            source,
            revision: 0,
            changeset: Some(changeset),
            review: Some(OpenReview {
                id,
                version: latest.n,
                updated_at: latest.at,
                last_reviewed: review.last_reviewed,
            }),
            filter,
            base,
            base_choice: review.base,
            base_commit: latest.base_commit.clone(),
            head: latest.tip.clone(),
            target,
            target_status,
            known: HashMap::new(),
            recorded: review.spans,
            acks: review.acks,
            threads,
            changes: None,
        }))
    }

    /// Settles the tree as it is now, so recorded spans and threads remap onto
    /// whatever changed while Vet was not running (I3).
    async fn catch_up(&self) -> Result<()> {
        let scan = self.acquisition.reconcile(Dirty::All).await?;
        if scan.snapshot.id != self.settled_snapshot().await {
            let (revision, retained) = self.settled(scan).await?;
            if let Some(e) = retained {
                eprintln!("vet: could not keep the content of revision {revision}: {e}");
            }
        }
        Ok(())
    }

    /// Takes the refs that keep the commits of `version` alive in git, except those
    /// the `previous` version already holds.
    async fn keep_commits(
        acquisition: &Acquisition<G, S>,
        changeset: ChangesetId,
        version: &Version,
        previous: Option<&Version>,
    ) -> Result<()> {
        for (name, commit) in version.commits_to_keep(previous) {
            acquisition
                .git
                .set_changeset_ref(changeset, version.n, name, commit)
                .await?;
        }
        Ok(())
    }

    /// Starts the review (SPEC "Session lifecycle"): records the base, `HEAD`
    /// and the current target, which becomes version 1, copies the target's
    /// dirty content into the store and takes the changeset's refs. Does nothing
    /// if a review is open. Returns whether it started one.
    async fn begin(&self, st: &mut State) -> Result<bool> {
        if st.review.is_some() {
            return Ok(false);
        }
        let acquisition = &self.acquisition;
        let (git, store) = (&acquisition.git, &acquisition.store);
        // A committed target stays in git, which the version's refs keep.
        let head = match st.source {
            Source::WorkingTree => {
                acquisition.retain(&[&st.base], &st.target, &st.target_status).await?;
                git.head().await?
            }
            _ => st.head.clone(),
        };
        let now = (self.clock)();
        store
            .put_snapshot(st.base.clone(), SnapshotStatus::of(&st.base), now)
            .await?;
        store
            .put_snapshot(st.target.clone(), st.target_status.clone(), now)
            .await?;
        let first = Version {
            n: 1,
            base: st.base.id,
            base_commit: st.base_commit.clone(),
            target: st.target.id,
            tip: head.clone(),
            at: now,
            origin: VersionOrigin::Settled,
            change: self.change_of(None, &st.base, &st.target).await,
        };
        let (changeset, id) = store
            .start_review(
                self.repo.clone(),
                st.source.clone(),
                st.base_choice.clone(),
                first.clone(),
                now,
            )
            .await?;
        // Refs of an earlier review that was never finished would hold commits for nothing.
        git.delete_changeset_refs(changeset).await?;
        Self::keep_commits(acquisition, changeset, &first, None).await?;
        acquisition.record_commit(st.target.id, head.clone());
        st.head = head;
        st.changeset = Some(changeset);
        st.review = Some(OpenReview {
            id,
            version: 1,
            updated_at: now,
            last_reviewed: None,
        });
        Ok(true)
    }

    /// Whether a review is open, and its version.
    pub async fn lifecycle(&self) -> Lifecycle {
        lifecycle_of(&*self.state.lock().await)
    }

    /// Ends the review: releases its refs, records whether it was complete, and
    /// goes back to browsing with no review state. The threads that are still
    /// open stay on view, since they belong to the changeset; the resolved ones
    /// remain in the finished review. Returns whether it was complete. Fails if
    /// no review is open. Bumps the revision.
    pub async fn finish_review(&self) -> Result<bool> {
        let mut st = self.state.lock().await;
        let (Some(id), Some(changeset)) = (st.review.as_ref().map(|r| r.id), st.changeset) else {
            return Err(SessionError::Invalid("no review is open".into()));
        };
        self.ensure_changes(&mut st).await?;
        let complete = {
            let changes = st.changes.as_ref().expect("computed");
            let acked = acknowledged(&st, &changes.diff);
            changeset_completion(&Coverage::new(&changes.flat), &st.filter, &changes.diff, &acked).is_complete()
        };
        // What the open threads are anchored in must outlive the refs.
        let open = |t: &&Thread| t.status == ThreadStatus::Open;
        let anchors = st.threads.iter().filter(open).filter_map(|t| match &t.target {
            CommentTarget::Range(a) => Some(a),
            _ => None,
        });
        self.retain_anchored(&st, anchors).await?;
        self.acquisition.git.delete_changeset_refs(changeset).await?;
        self.acquisition
            .store
            .close_review(id, (self.clock)(), complete)
            .await?;
        st.review = None;
        st.recorded.clear();
        st.acks.clear();
        st.threads.retain(|t| t.status == ThreadStatus::Open);
        st.known.clear();
        st.revision += 1;
        st.changes = None;
        self.revisions.advance(st.revision);
        Ok(complete)
    }

    /// Changes what the target is compared against; the target stays as it is.
    /// While a review is open, a base on another commit is the next
    /// version. Bumps the revision unless nothing changed. Recorded spans are
    /// untouched: the next manifest remaps them onto the new base, so base-side
    /// changes in reviewed regions show as `SeenAgainstOldBase` (§5.2), and
    /// choosing an earlier base again restores the earlier coverage. Returns the
    /// revision.
    pub async fn set_base(&self, choice: BaseChoice) -> Result<(BaseView, u32)> {
        let git = &self.acquisition.git;
        let mut st = self.state.lock().await;
        // A merge base is where the target's history left the reference's: `HEAD` for the live tree.
        let tip = match st.source {
            Source::WorkingTree => git.head().await?,
            _ => st.head.clone(),
        };
        let (choice, commit) = base::resolve(git, choice, &tip).await.map_err(SessionError::Base)?;
        let snapshot = git.commit_snapshot(&commit).await?;

        if st.source == Source::WorkingTree {
            self.acquisition
                .store
                .set_browsing_base(self.repo.clone(), choice.clone())
                .await?;
        }
        if st.base_choice != choice || st.base_commit != commit {
            if let (Some(review), Some(changeset)) = (&st.review, st.changeset) {
                let store = &self.acquisition.store;
                let (id, number) = (review.id, review.version + 1);
                // The old base's content is kept apart from git, which may drop its objects.
                self.retain_anchored(&st, st.anchors()).await?;
                let now = (self.clock)();
                store
                    .put_snapshot(snapshot.clone(), SnapshotStatus::of(&snapshot), now)
                    .await?;
                store.set_base(id, choice.clone()).await?;
                if commit != st.base_commit {
                    let version = Version {
                        n: number,
                        base: snapshot.id,
                        base_commit: commit.clone(),
                        target: st.target.id,
                        tip: st.head.clone(),
                        at: now,
                        origin: VersionOrigin::BaseMoved,
                        change: self
                            .change_of(Some((&st.base, &st.target)), &snapshot, &st.target)
                            .await,
                    };
                    let previous = store.versions(changeset).await?.pop();
                    store.append_version(changeset, version.clone()).await?;
                    Self::keep_commits(&self.acquisition, changeset, &version, previous.as_ref()).await?;
                    if let Some(review) = &mut st.review {
                        review.version = number;
                        review.updated_at = now;
                    }
                }
            }
            self.acquisition.record_commit(snapshot.id, commit.clone());
            st.base = snapshot;
            st.base_choice = choice;
            st.base_commit = commit;
            st.revision += 1;
            st.changes = None;
            self.revisions.advance(st.revision);
        }
        Ok((self.base_view(&st, &git.branches().await?).await?, st.revision))
    }

    /// The base as it displays now. How far its branch has moved on is read again, so the answer
    /// changes after a fetch while the base itself stays where it is.
    pub async fn base(&self) -> Result<BaseView> {
        let st = self.state.lock().await;
        self.base_view(&st, &self.acquisition.git.branches().await?).await
    }

    /// The base with its display facts; `branches` are the repository's.
    async fn base_view(&self, st: &State, branches: &[BranchInfo]) -> Result<BaseView> {
        let git = &self.acquisition.git;
        let prefer = match &st.base_choice {
            BaseChoice::MergeBase { reference } => Some(reference.as_str()),
            _ => None,
        };
        let own = manifest::target_view(&st.source, branches).branch;
        Ok(BaseView {
            choice: st.base_choice.clone(),
            commit: git.commit_info(&st.base_commit).await?,
            name: base::name_of(git, branches, prefer, &st.base_commit).await?,
            contained_in: base::contained_in(&st.base_choice, &st.base_commit, &st.head, own.as_deref()),
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
        self.revisions.advance(st.revision);
        Ok(st.revision)
    }

    /// Branches and recent commits to pick a base from.
    pub async fn base_candidates(&self) -> Result<BaseCandidates> {
        Ok(self.acquisition.git.base_candidates(MAX_BASE_COMMITS).await?)
    }

    pub async fn revision(&self) -> u32 {
        self.state.lock().await.revision
    }

    /// The newest revision on view, as the session announces it. Semantic work for
    /// an older one is dropped.
    pub fn revisions(&self) -> Arc<Revisions> {
        self.revisions.clone()
    }

    fn set_degraded(&self, degraded: Option<Degraded>) {
        *self.degraded.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = degraded;
    }

    /// Why the working tree is polled instead of watched, if it is; none for any other source.
    fn degraded_for(&self, source: &Source) -> Option<Degraded> {
        let degraded = *self.degraded.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        degraded.filter(|_| *source == Source::WorkingTree)
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
            live_target: st.source == Source::WorkingTree,
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
    /// the changeset gains a version. Review state is remapped when the next
    /// manifest is computed. A failed copy of its content is not fatal: the diff
    /// then reads the worktree, or shows the file as unavailable.
    /// It is the working tree's, wherever that is: on view, or parked while a branch is.
    pub async fn settled(&self, scan: Scan) -> Result<(u32, Option<BlobError>)> {
        let mut guard = self.state.lock().await;
        if guard.source == Source::WorkingTree {
            let settled = self.settle_tree(&mut guard, scan).await?;
            self.revisions.advance(settled.0);
            return Ok(settled);
        }
        let mut parked = self.parked.lock().await;
        let tree = parked
            .get_mut(&Source::WorkingTree)
            .expect("the working tree is parked while another changeset is on view");
        self.settle_tree(tree, scan).await
    }

    /// Settles `scan` into `st`, which need not be the one on view: only the revision of
    /// the one on view is announced, by the caller.
    async fn settle_tree(&self, st: &mut State, scan: Scan) -> Result<(u32, Option<BlobError>)> {
        let mut retained = None;
        if let (Some(review), Some(changeset)) = (&st.review, st.changeset) {
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
            let version = Version {
                n: review.version + 1,
                base: st.base.id,
                base_commit: st.base_commit.clone(),
                target: scan.snapshot.id,
                tip: st.head.clone(),
                at: now,
                origin: VersionOrigin::Settled,
                change: self
                    .change_of(Some((&st.base, &st.target)), &st.base, &scan.snapshot)
                    .await,
            };
            store.append_version(changeset, version).await?;
        }
        // Older versions stay materializable (§7.2).
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

    /// Settles snapshots of the worktree as `settling` reports them, and looks at the branch
    /// on view when a ref was written, calling `on_settled` with each new revision of the
    /// changeset on view, until dropped. A settled tree that is not on view is not announced:
    /// it is not what is shown.
    pub async fn follow<W: Watcher + Send, C: Clock + Send + Sync>(
        &self,
        mut settling: Settling<W, C>,
        mut on_settled: impl FnMut(u32) + Send,
    ) {
        self.set_degraded(settling.degraded());
        loop {
            let scan = match settling.next_settled(&self.acquisition).await {
                Ok(Settled::Tree(scan)) => scan,
                Ok(Settled::RefsMoved) => {
                    match self.observe().await {
                        Ok(Some(revision)) => on_settled(revision),
                        Ok(None) => {}
                        Err(e) => eprintln!("vet: could not look at the branch on view: {e}"),
                    }
                    continue;
                }
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
                    self.set_degraded(settling.degraded());
                    if self.state.lock().await.source == Source::WorkingTree {
                        on_settled(revision);
                    }
                }
                Err(e) => eprintln!("vet: could not record a settled snapshot: {e}"),
            }
        }
    }

    /// Copies into the store the content that `anchors` (those of recorded spans
    /// and range threads) are placed in and only git holds, so their coverage
    /// survives git dropping the objects of a base the changeset left, and a
    /// restart.
    async fn retain_anchored<'a>(&self, st: &State, anchors: impl Iterator<Item = &'a Anchor>) -> Result<()> {
        let wanted: BTreeSet<(SnapshotId, &RepoPath)> = anchors.map(|a| (a.snapshot, &a.file)).collect();
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
