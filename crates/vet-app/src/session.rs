//! A live session of a dirty tree (§4.1, §4.2, §5).
//!
//! **Lifecycle.** A session starts out browsing: the live diff against the
//! base, with nothing stored. The first review action (marking seen,
//! acknowledging, a new thread) starts the review: it records the base, `HEAD`
//! and the current target as the working tree changeset's version 1, and from
//! then on every settled snapshot is the next version. Finishing releases the
//! review's refs and returns to browsing (see SPEC §4.5);
//! threads that are still open stay on view, because they belong to the
//! changeset, not to the review.
//!
//! **Changeset.** What it displays is `base → the latest settled snapshot`,
//! the base being fixed by the kind of changeset (§4.4): the working tree's is
//! the `HEAD` it was opened at, wherever `HEAD` goes after. The review is of
//! everything the agent has done to the tree, so reviewing the live part
//! (`previous version → now`) alone would hide reviewed work behind a
//! changeset that has no review state of its own. Marks recorded against
//! earlier targets carry over by remapping (§5.2), so the live part is exactly
//! what shows up as unseen. Showing it separately is an interdiff, which
//! belongs to the timeline (M1). A `HEAD` that no longer builds on the base
//! (`off_base`) is told, since the changes then include work from elsewhere.
//!
//! **Revision.** Starts at 0 and increases by one with each settled snapshot,
//! filter change, observed move of a branch, and finished review.
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
//! This module is the changeset on view: the token, the published state, the
//! revision announcements and the settle gate, and how a review begins, records
//! its versions and ends. `open` opens a session, `settling` settles the working
//! tree's snapshots into it, `filter` changes its review filter and `base_view`
//! describes the base. Two parts beside it never touch that state: `facts`
//! holds what the repository says and its caches (`derived` computes the diffs,
//! interdiffs and commits, `lines` and `listing` read the queue's counts and
//! rows from it), and `ahead` copies the working tree's content before a review
//! needs it. The third, `exchange`, is the platform exchange, whose commands are
//! review commands. `remap` puts a revision's review state onto the diffs,
//! `manifest` and `content` and `browse` show them, `marks` and `threads` edit
//! the review, `blobs` reads the content they all need, and `source` is about
//! which changeset is on view.

mod ahead;
mod base_diff;
mod base_view;
mod blobs;
mod browse;
mod content;
mod derived;
mod exchange;
mod facts;
mod filter;
mod hold;
mod interdiff;
mod lens;
mod lines;
mod listing;
mod lru;
mod manifest;
mod marks;
mod open;
mod pushes;
mod remap;
mod revision;
mod search;
mod settling;
mod source;
mod state;
mod threads;
mod trust;
mod versions;
mod views;

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;
use vet_core::content::{EntryKind, RepoPath, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::review::{Anchor, CommentTarget, Thread, ThreadStatus};

use self::ahead::Copies;
pub use self::base_diff::{BaseDiffKind, BaseDiffRow, BaseDiffView};
pub use self::blobs::NoText;
pub use self::browse::{DirEntryKind, DirItem, DirListing, FileListing, PatternMatch, PatternMatches};
pub use self::content::{FileText, ItemBlob, MAX_ITEM_BLOB, TargetText};
use self::exchange::Exchange;
pub use self::exchange::{
    Blocker, Brought, DiscussionView, Keep, Outgoing, SubmitReport, SyncNotice, SyncReport, Unread, VoteSent, is_unread,
};
pub use self::facts::Facts;
use self::hold::{Held, Published, Writer};
pub use self::lens::{CommitKind, CommitUnitView, CommitsView};
pub use self::listing::{ChangesetList, ChangesetRow, Divergence, RowReview};
pub use self::revision::{ChangedFiles, RevisionView};
pub use self::search::{MAX_HITS, SearchFile, SearchQuery, SearchResult, SearchScope};
pub use self::state::View;
use self::state::{OpenReview, Shown, State, lifecycle_of};
pub use self::threads::{CommentAt, CommentSpot, NewThread, ThreadView, ThreadsView};
pub use self::versions::VersionView;
use self::versions::first_change;
pub use self::views::{
    BaseView, ChangesSince, Contained, EmptyReason, EntryInfo, ExcludedBy, ExcludedFile, ExcludedGroup, FileStatus,
    FileView, InterdiffView, ItemView, Lifecycle, ManifestView, ModeChange, Outcome, ReviewUpdateView, ReviewView,
    SemanticSides, TargetView,
};
use crate::acquisition::{Acquisition, BlobError};
use crate::changeset::{ChangesetId, CommitId, Snapshots, Source, Version, VersionOrigin};
use crate::platform::PlatformError;
use crate::ports::{Degraded, GitError, GitRepo, LiveStore, ReviewWrites, StoreError};
use crate::revisions::Revisions;
use crate::settle::Wanted;

#[derive(Clone, Debug)]
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
    /// The request does not apply to the thread or the text; the message is fit to show.
    Invalid(String),
    /// The platform could not do what the request needed of it.
    Platform(PlatformError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Platform(e) => e.fmt(f),
            // A revision that is not there, or has no history in common, is said as it is: it is fit to show.
            Self::Git(GitError::UnknownRevision(message) | GitError::Unrelated(message)) => f.write_str(message),
            Self::Git(e) => write!(f, "git: {e}"),
            Self::Store(e) => write!(f, "store: {}", e.0),
            Self::Stale { current } => write!(f, "stale revision; the changeset is at revision {current}"),
            Self::NotFound(what) => write!(f, "no such {what}"),
            Self::NotText(path) => write!(f, "{path} has no text to show"),
            Self::Invalid(message) => f.write_str(message),
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

impl From<PlatformError> for SessionError {
    fn from(e: PlatformError) -> Self {
        Self::Platform(e)
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

/// Unix seconds now.
pub type UnixClock = fn() -> i64;

/// Where the changeset on view stands: its revision and how often its snapshots moved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    pub revision: u32,
    pub snapshot: u32,
}

pub struct LiveSession<G, S: LiveStore> {
    /// Author of the messages written here: git's `user.name`, else "You".
    author: String,
    /// Shared with the session's semantic pool, which materializes snapshots from it.
    acquisition: Arc<Acquisition<G, S>>,
    name: String,
    /// The repository root, which with a source is the key a changeset is stored under.
    repo: String,
    clock: UnixClock,
    /// The review state of the changeset on view, as the last command left it.
    published: Arc<Published>,
    /// Held by the one command that changes review state at a time, which asks for it in the order
    /// the viewer acted in. It guards the changesets that were on view and are not now.
    token: Mutex<Writer<S::Writes>>,
    /// Why the working tree is polled instead of watched, if it is.
    degraded: std::sync::Mutex<Option<Degraded>>,
    /// Told every time the revision on view moves, so that work for an older one stops.
    revisions: Arc<Revisions>,
    exchange: Exchange,
    /// What the repository says that no review state shapes, and the caches of it.
    facts: Facts<G, S>,
    copies: Copies<G, S>,
    /// What the settle loop asks before reconciling the working tree: while the tree is not on
    /// view and has no open review, edits only mark it changed and are not scanned.
    settle_wanted: Arc<Wanted>,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Takes the refs that keep the commits of `version` alive in git, except those the version
    /// before it already holds, whose base commit and tip are `previous`, and those `existing`
    /// says point at the same commit already. A resumption passes what it found in git so that
    /// coming back writes none that is still there.
    async fn keep_commits(
        acquisition: &Acquisition<G, S>,
        changeset: ChangesetId,
        version: &Version,
        previous: Option<(&CommitId, &CommitId)>,
        existing: Option<&HashMap<(u32, String), CommitId>>,
    ) -> Result<()> {
        for (name, commit) in version.commits_to_keep(previous) {
            if existing.is_some_and(|kept| kept.get(&(version.n, name.to_owned())) == Some(commit)) {
                continue;
            }
            acquisition
                .git
                .set_changeset_ref(changeset, version.n, name, commit)
                .await?;
        }
        Ok(())
    }

    /// Makes `version` of the review of `changeset` durable: the content it needs in the store
    /// (`snapshots`), the refs that keep its commits, and last `write`, the store's record of it.
    /// A failure before the record leaves the review as it was, so the same version can be tried
    /// again; callers publish the version in memory right after, with nothing awaited between.
    async fn record_version<T>(
        &self,
        store: &S::Writes,
        changeset: ChangesetId,
        version: &Version,
        previous: Option<(&CommitId, &CommitId)>,
        snapshots: &[(&Snapshot, &SnapshotStatus)],
        write: impl Future<Output = std::result::Result<T, StoreError>> + Send,
    ) -> Result<T> {
        self.record_content(store, changeset, version, previous, snapshots)
            .await?;
        Ok(write.await?)
    }

    /// What [`LiveSession::record_version`] makes durable before the store's record of the version.
    async fn record_content(
        &self,
        store: &S::Writes,
        changeset: ChangesetId,
        version: &Version,
        previous: Option<(&CommitId, &CommitId)>,
        snapshots: &[(&Snapshot, &SnapshotStatus)],
    ) -> Result<()> {
        for (snapshot, status) in snapshots {
            store
                .put_snapshot(Snapshot::clone(snapshot), SnapshotStatus::clone(status), version.at)
                .await?;
        }
        Self::keep_commits(&self.acquisition, changeset, version, previous, None).await
    }

    /// The write token, with the state as the last command left it.
    async fn hold(&self) -> Held<'_, S::Writes> {
        Held::new(self.token.lock().await, &self.published)
    }

    /// What `view` shows, for a command that starts the review if none is open. Starting it needs
    /// the working tree's dirty content in the store, which can take a while, or a copy ahead under
    /// way to end: that is done first, without the token, so the other commands go on meanwhile and
    /// the token's holder never waits for a copy. If the revision moved meanwhile, it starts over.
    /// What it returns has the content held: [`LiveSession::begin`] copies nothing.
    async fn beginning(&self, view: View) -> Result<(Held<'_, S::Writes>, Shown)> {
        let mut copied = None;
        loop {
            let (st, shown) = self.writing(view).await?;
            if st.review.is_some() || st.source != Source::WorkingTree || copied == Some(st.revision) {
                return Ok((st, shown));
            }
            copied = Some(st.revision);
            let (base, target, status) = (st.base.clone(), st.target.clone(), st.target_status.clone());
            let diffed = st.changes().diffed.clone();
            drop(st);
            self.copies
                .retained(&self.acquisition, (&base, &target, &status), diffed)
                .await?;
        }
    }

    /// Starts the review (SPEC §4.5): records the base, `HEAD`
    /// and the current target, which becomes version 1, and takes the
    /// changeset's refs. The target's dirty content is in the store already:
    /// [`LiveSession::beginning`] saw to it. Does nothing if a review is open.
    /// Returns whether it started one.
    async fn begin(&self, st: &mut Held<'_, S::Writes>) -> Result<bool> {
        if st.review.is_some() {
            return Ok(false);
        }
        let acquisition = &self.acquisition;
        let git = &acquisition.git;
        let store = st.writes();
        // A committed target stays in git, which the version's refs keep.
        let head = match st.source {
            Source::WorkingTree => git.head().await?,
            _ => st.head.clone(),
        };
        let now = (self.clock)();
        let changeset = store.ensure_changeset(self.repo.clone(), st.source.clone()).await?;
        // Refs of an earlier review that was never finished would hold commits for nothing.
        git.delete_changeset_refs(changeset).await?;
        // A pull request has the pushes the platform recorded as versions from the first one, the
        // state it is at being the last.
        let pushes = self.new_pushes(st, &[], &head);
        let (earlier, previous) = match &pushes {
            Some(pushes) if pushes.len() > 1 => self.history(st, &pushes[..pushes.len() - 1], 1, None).await?,
            _ => (Vec::new(), None),
        };
        let change = match &previous {
            Some((old_base, old_target)) => self.change_of((old_base, old_target), &st.base, &st.target).await?,
            None => first_change(&st.changes().diffed),
        };
        let current = Version {
            n: u32::try_from(earlier.len()).unwrap_or(u32::MAX) + 1,
            snapshots: Some(Snapshots {
                base: st.base.id,
                target: st.target.id,
            }),
            base_commit: st.base_commit.clone(),
            tip: head.clone(),
            at: pushes.as_ref().and_then(|p| p.last()?.at).unwrap_or(now),
            origin: VersionOrigin::Settled,
            change,
        };
        let mut versions: Vec<Version> = earlier.iter().map(|r| r.version.clone()).collect();
        versions.push(current.clone());
        let start = store.start_review(
            self.repo.clone(),
            st.source.clone(),
            st.base_choice.clone(),
            versions.clone(),
            now,
        );
        for recording in &earlier {
            let statuses = recording.snapshots();
            let snapshots: Vec<(&Snapshot, &SnapshotStatus)> =
                statuses.iter().map(|(s, status)| (*s, status)).collect();
            let kept = Version::last_commits(&versions[..recording.version.n as usize - 1]);
            self.record_content(&store, changeset, &recording.version, kept, &snapshots)
                .await?;
        }
        let base_status = SnapshotStatus::of(&st.base);
        let snapshots = [(&*st.base, &base_status), (&*st.target, &st.target_status)];
        let kept = Version::last_commits(&versions[..versions.len() - 1]);
        let (_, id) = self
            .record_version(&store, changeset, &current, kept, &snapshots, start)
            .await?;
        acquisition.record_commit(st.target.id, head.clone());
        st.head = head;
        st.changeset = Some(changeset);
        st.review = Some(OpenReview {
            id,
            version: current.n,
            updated_at: now,
            last_reviewed: None,
            summary: None,
        });
        Ok(true)
    }

    /// Whether a review is open, and its version.
    pub async fn lifecycle(&self) -> Lifecycle {
        lifecycle_of(&self.published.read())
    }

    /// Ends the review: records whether it was complete, releases its refs, and
    /// goes back to browsing with no review state. The threads that are still
    /// open stay on view, since they belong to the changeset; the resolved ones
    /// remain in the finished review. Returns whether it was complete. Fails if
    /// no review is open. Bumps the revision.
    pub async fn finish_review(&self) -> Result<bool> {
        let (mut st, _) = self.writing(View::Changeset).await?;
        let (Some(id), Some(changeset)) = (st.review.as_ref().map(|r| r.id), st.changeset) else {
            return Err(SessionError::Invalid("no review is open".into()));
        };
        let complete = manifest::stored_completion(&st).is_complete();
        // What the open threads are anchored in must outlive the refs.
        let open = |t: &&Thread| t.status == ThreadStatus::Open;
        let anchors = st.threads.iter().filter(open).filter_map(|t| match &t.target {
            CommentTarget::Range(a) => Some(a),
            _ => None,
        });
        self.retain_anchored(&st, anchors).await?;
        if st.source == Source::WorkingTree {
            // The next session of the workspace pins the `HEAD` it finds.
            st.writes()
                .set_pinned_base(self.repo.clone(), Source::WorkingTree, None)
                .await?;
        }
        st.writes().close_review(id, (self.clock)(), complete).await?;
        // The review is closed in the store: the state follows with nothing awaited between.
        st.review = None;
        st.recorded.clear();
        st.acks.clear();
        st.threads.retain(|t| t.status == ThreadStatus::Open || t.is_posted());
        st.known.clear();
        st.revision += 1;
        st.changes = None;
        st.publish();
        self.revisions.advance(st.revision);
        // Refs left behind by a failure here are taken again with the next review's first version.
        if let Err(e) = self.acquisition.git.delete_changeset_refs(changeset).await {
            tracing::warn!(error = %e, "could not release the refs of the finished review");
        }
        Ok(complete)
    }

    pub async fn revision(&self) -> u32 {
        self.published.read().revision
    }

    /// The revision on view and the count of snapshot moves, read together: a revision whose
    /// snapshot did not move is a new filter's.
    pub async fn position(&self) -> Position {
        let st = self.published.read();
        Position {
            revision: st.revision,
            snapshot: st.snapshot,
        }
    }

    /// The latest version of the open review; none while browsing.
    pub fn review_version(&self) -> Option<u32> {
        self.published.read().review.as_ref().map(|r| r.version)
    }

    /// The newest revision on view, as the session announces it. Semantic work for
    /// an older one is dropped.
    pub fn revisions(&self) -> Arc<Revisions> {
        self.revisions.clone()
    }

    /// The settle loop's gate on the working tree: while the tree is not on view and has no open
    /// review, edits only mark it changed, and coming on view settles it once. The loop must take
    /// it ([`Settling::wanted`](crate::settle::Settling::wanted)) for the gate to apply.
    pub fn settle_wanted(&self) -> Arc<Wanted> {
        self.settle_wanted.clone()
    }

    /// The latest settled target: what the review last showed.
    pub async fn target_snapshot(&self) -> Arc<Snapshot> {
        self.published.read().target.clone()
    }

    /// What knows the origin of the session's snapshots, for serving them to a
    /// language-server pool.
    pub fn acquisition(&self) -> Arc<Acquisition<G, S>> {
        self.acquisition.clone()
    }

    /// What the repository says that no review state shapes: the queue's rows and line counts.
    pub fn facts(&self) -> &Facts<G, S> {
        &self.facts
    }

    /// The initial scan identifies the settled snapshot, so settling does not
    /// report it again.
    pub async fn settled_snapshot(&self) -> SnapshotId {
        self.published.read().target.id
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

#[cfg(test)]
pub(crate) mod tests;
