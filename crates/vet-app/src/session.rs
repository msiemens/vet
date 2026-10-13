//! A live session of a dirty tree (§5, §6.1, §6.2).
//!
//! **Lifecycle.** A session starts out browsing: the live diff against the
//! base, with nothing stored. The first review action (marking seen,
//! acknowledging, a new thread) starts the review: it records the base, `HEAD`
//! and the current target as the working tree changeset's version 1, and from
//! then on every settled snapshot is the next version. Finishing releases the
//! review's refs and returns to browsing (see SPEC "Session lifecycle");
//! threads that are still open stay on view, because they belong to the
//! changeset, not to the review.
//!
//! **Changeset.** What it displays is `base → the latest settled snapshot`,
//! the base being fixed by the kind of changeset (§6.5): the working tree's is
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
//! This module is the lifecycle and persistence of a review. The rest of the
//! session is split by what it does with the state: `derived` computes the
//! diffs, interdiffs and commits without touching it, `remap` puts a revision's
//! review state onto them, `manifest` and `content` and `browse` show them,
//! `marks` and `threads` edit the review, `blobs` reads the content they all
//! need, `ahead` copies the working tree's content before a review needs it,
//! and `source` and `listing` are about which changeset is on view.

mod ahead;
mod base_diff;
mod blobs;
mod browse;
mod content;
mod derived;
mod hold;
mod interdiff;
mod lens;
mod lines;
mod listing;
mod manifest;
mod marks;
mod remap;
mod revision;
mod search;
mod source;
mod state;
mod sync;
mod threads;
mod trust;
mod versions;
mod views;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;
use vet_core::content::{EntryKind, RepoPath, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::review::{Anchor, CommentTarget, Thread, ThreadStatus};
use vet_engine::{Engine, Limits};

use self::ahead::Retained;
pub use self::base_diff::{BaseDiffKind, BaseDiffRow, BaseDiffView};
pub use self::blobs::NoText;
pub use self::browse::{DirEntryKind, DirItem, DirListing, FileListing};
pub use self::content::{FileText, ItemBlob, MAX_ITEM_BLOB, TargetText};
use self::derived::{Derived, Diffed};
use self::hold::{Held, Published, Writer};
pub use self::lens::{CommitKind, CommitUnitView, CommitsView};
use self::lines::LinesBetween;
pub use self::listing::{ChangesetList, ChangesetRow, Divergence, RowReview};
pub use self::revision::{ChangedFiles, RevisionView};
pub use self::search::{MAX_HITS, SearchFile, SearchQuery, SearchResult, SearchScope};
use self::source::Stored;
pub use self::state::View;
use self::state::{OpenReview, Shown, State, lifecycle_of};
pub use self::sync::{DiscussionView, Keep, SubmitReport, SyncNotice, SyncReport, VoteSent};
pub use self::threads::{CommentAt, CommentSpot, NewThread, ThreadView, ThreadsView};
pub use self::versions::VersionView;
use self::versions::first_change;
pub use self::views::{
    BaseView, Contained, EntryInfo, ExcludedBy, ExcludedFile, ExcludedGroup, FileStatus, FileView, InterdiffView,
    ItemView, Lifecycle, ManifestView, ModeChange, Outcome, ReviewUpdateView, ReviewView, SemanticSides, TargetView,
};
use crate::acquisition::{Acquisition, BlobError};
use crate::base;
use crate::changeset::{BaseChoice, ChangesetId, CommitId, Source, Version, VersionChange, VersionOrigin};
use crate::platform::PlatformError;
use crate::ports::{
    BranchInfo, Clock, CommitInfo, Degraded, Dirty, GitError, GitRepo, LiveStore, ReviewWrites, Scan, StoreError,
    Watcher,
};
use crate::progress::Tasks;
use crate::review::{ReviewId, visible_threads};
use crate::revisions::Revisions;
use crate::settle::{Settled, Settling};

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
    /// The chosen base does not resolve; the message is fit to show.
    Base(String),
    /// The request does not apply to the thread or the text; the message is fit to show.
    Invalid(String),
    /// The platform could not do what the request needed of it.
    Platform(PlatformError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Platform(e) => e.fmt(f),
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
    /// Held while threads are exchanged with a platform, so two exchanges never overlap. Locked
    /// before the token, which is not held across the platform's answers.
    exchange: Mutex<()>,
    /// The line deltas between commits that the list of changesets shows.
    lines: Engine<LinesBetween>,
    /// The diffs, interdiffs and commits the review state is remapped onto.
    derived: Arc<Derived>,
    /// What git said of the commits the manifests name, which it does not change.
    commit_infos: std::sync::Mutex<HashMap<CommitId, CommitInfo>>,
    /// The copies of the working tree's content into the store under way.
    ahead: Engine<Retained>,
    /// What a copy ahead of a review reads and writes through: the session's acquisition over
    /// handles of their own, so that it draws on what nobody waits for.
    copying: Arc<Acquisition<G, S>>,
}

/// How many line deltas the list of changesets keeps.
const KEPT_LINE_DELTAS: usize = 4096;

/// What settling a snapshot into a tree needs that takes a while, worked out for the tree as it
/// was found, without the token.
struct Settlement {
    base: SnapshotId,
    target: SnapshotId,
    /// A review of the tree was open, so the settled snapshot is its next version.
    reviewing: bool,
    /// Why the content of the version could not be kept, if it could not.
    retained: Option<BlobError>,
    /// What the version changed against the one before it.
    change: Option<VersionChange>,
    /// The diff of the base against the settled snapshot, if it could be had.
    diffed: Option<Arc<Diffed>>,
}

impl Settlement {
    /// Whether `tree` is still the one this was worked out for.
    fn fits(&self, tree: &State) -> bool {
        let reviewing = tree.review.is_some() && tree.changeset.is_some();
        tree.base.id == self.base && tree.target.id == self.target && reviewing == self.reviewing
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

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
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
        let writes = acquisition.store.writes();
        let (changeset, threads) = match Self::stored(&acquisition, &writes, &repo, &source, &filter, clock).await? {
            Stored::Resumed(mut state) => {
                state.off_base = base::head_left(&acquisition.git, &state.base_commit).await;
                let session = Self::assemble(acquisition, name, repo, clock, *state).await;
                session.catch_up().await?;
                return Ok(session);
            }
            Stored::Browsing { changeset, threads } => (changeset, threads),
        };
        let git = &acquisition.git;
        let head = git.head().await?;
        // Browsing keeps the base the tree was opened against across restarts, until a review
        // that ends lets the next session pin its own.
        let pinned = acquisition.store.pinned_base(repo.clone(), source.clone()).await?;
        let (base_choice, base_commit, base) = match pinned {
            Some(commit) => match git.commit_snapshot(&commit).await {
                Ok(base) => {
                    let (choice, commit) = base::for_working_tree(&commit);
                    (choice, commit, base)
                }
                Err(_) => Self::pin(&acquisition, &writes, &repo, &source, &head).await?,
            },
            None => Self::pin(&acquisition, &writes, &repo, &source, &head).await?,
        };
        let base = Arc::new(base);
        let off_base = base::head_left(git, &base_commit).await;
        // The session opens at `HEAD`, which builds on itself.
        let Scan {
            snapshot: target,
            status,
        } = acquisition.reconcile(Dirty::All).await?;
        let target = Arc::new(target);
        acquisition.record_commit(base.id, base_commit.clone());
        acquisition.record_commit(target.id, head.clone());
        let state = State {
            source,
            revision: 0,
            seq: 0,
            changeset,
            review: None,
            filter,
            project: None,
            base,
            base_choice,
            base_commit,
            head,
            off_base,
            target,
            target_status: status,
            known: HashMap::new(),
            recorded: Vec::new(),
            acks: Vec::new(),
            threads,
            discussion: None,
            changes: None,
        };
        Ok(Self::assemble(acquisition, name, repo, clock, state).await)
    }

    /// Pins the working tree to `head` as its base and remembers that.
    async fn pin(
        acquisition: &Acquisition<G, S>,
        writes: &S::Writes,
        repo: &str,
        source: &Source,
        head: &CommitId,
    ) -> Result<(BaseChoice, CommitId, Snapshot)> {
        let (choice, commit) = base::for_working_tree(head);
        let base = acquisition.git.commit_snapshot(&commit).await?;
        writes
            .set_pinned_base(repo.to_owned(), source.clone(), Some(commit.clone()))
            .await?;
        Ok((choice, commit, base))
    }

    async fn assemble(
        acquisition: Arc<Acquisition<G, S>>,
        name: String,
        repo: String,
        clock: UnixClock,
        state: State,
    ) -> Self {
        let writes = acquisition.store.writes();
        Self {
            author: author_of(&acquisition.git).await,
            derived: Arc::new(Derived::new(&acquisition.tasks)),
            commit_infos: std::sync::Mutex::default(),
            ahead: ahead::copies(&acquisition.tasks),
            copying: acquisition.clone(),
            acquisition,
            name,
            repo,
            clock,
            published: Arc::new(Published::new(state)),
            token: Mutex::new(Writer::new(writes)),
            degraded: std::sync::Mutex::new(None),
            revisions: Arc::default(),
            exchange: Mutex::new(()),
            lines: Engine::new(Limits::entries(KEPT_LINE_DELTAS)),
        }
    }

    /// The same session, its copies ahead of a review reading through `git` and writing through
    /// `store`, other handles on the session's repository and store.
    pub fn copying_through(self, git: G, store: S) -> Self {
        Self {
            copying: Arc::new(self.acquisition.through(git, store)),
            ..self
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
        let (base, target) = (Arc::new(base), Arc::new(target));
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
            seq: 0,
            changeset: Some(changeset),
            review: Some(OpenReview {
                id,
                version: latest.n,
                updated_at: latest.at,
                last_reviewed: review.last_reviewed,
                summary: review.summary,
            }),
            filter,
            project: None,
            base,
            base_choice: review.base,
            base_commit: latest.base_commit.clone(),
            head: latest.tip.clone(),
            off_base: false,
            target,
            target_status,
            known: HashMap::new(),
            recorded: review.spans.into_iter().map(Arc::new).collect(),
            acks: review.acks,
            threads,
            discussion: None,
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
                tracing::warn!(revision, error = %e, "could not keep the content of the revision");
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
            self.retained(&self.acquisition, (&base, &target, &status), diffed)
                .await?;
        }
    }

    /// Starts the review (SPEC "Session lifecycle"): records the base, `HEAD`
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
        store
            .put_snapshot(Snapshot::clone(&st.base), SnapshotStatus::of(&st.base), now)
            .await?;
        store
            .put_snapshot(Snapshot::clone(&st.target), st.target_status.clone(), now)
            .await?;
        let first = Version {
            n: 1,
            base: st.base.id,
            base_commit: st.base_commit.clone(),
            target: st.target.id,
            tip: head.clone(),
            at: now,
            origin: VersionOrigin::Settled,
            change: first_change(&st.changes().diffed),
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
            summary: None,
        });
        Ok(true)
    }

    /// Whether a review is open, and its version.
    pub async fn lifecycle(&self) -> Lifecycle {
        lifecycle_of(&self.published.read())
    }

    /// Ends the review: releases its refs, records whether it was complete, and
    /// goes back to browsing with no review state. The threads that are still
    /// open stay on view, since they belong to the changeset; the resolved ones
    /// remain in the finished review. Returns whether it was complete. Fails if
    /// no review is open. Bumps the revision.
    pub async fn finish_review(&self) -> Result<bool> {
        let (mut st, _) = self.writing(View::Changeset).await?;
        let (Some(id), Some(changeset)) = (st.review.as_ref().map(|r| r.id), st.changeset) else {
            return Err(SessionError::Invalid("no review is open".into()));
        };
        let complete = st.changes().remapped.completion.is_complete();
        // What the open threads are anchored in must outlive the refs.
        let open = |t: &&Thread| t.status == ThreadStatus::Open;
        let anchors = st.threads.iter().filter(open).filter_map(|t| match &t.target {
            CommentTarget::Range(a) => Some(a),
            _ => None,
        });
        self.retain_anchored(&st, anchors).await?;
        self.acquisition.git.delete_changeset_refs(changeset).await?;
        st.writes().close_review(id, (self.clock)(), complete).await?;
        if st.source == Source::WorkingTree {
            // The next session of the workspace pins the `HEAD` it finds.
            st.writes()
                .set_pinned_base(self.repo.clone(), Source::WorkingTree, None)
                .await?;
        }
        st.review = None;
        st.recorded.clear();
        st.acks.clear();
        st.threads.retain(|t| t.status == ThreadStatus::Open || t.is_posted());
        st.known.clear();
        st.revision += 1;
        st.changes = None;
        st.publish();
        self.revisions.advance(st.revision);
        Ok(complete)
    }

    /// What git says of `commit`, asked once while the session lives.
    async fn commit_info(&self, commit: &CommitId) -> Result<CommitInfo> {
        const KEPT: usize = 64;
        let kept = |infos: &HashMap<CommitId, CommitInfo>| infos.get(commit).cloned();
        let found = kept(
            &self
                .commit_infos
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if let Some(info) = found {
            return Ok(info);
        }
        let info = self.acquisition.git.commit_info(commit).await?;
        let mut infos = self
            .commit_infos
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if infos.len() >= KEPT {
            infos.clear();
        }
        infos.insert(commit.clone(), info.clone());
        Ok(info)
    }

    /// The base with its display facts; `branches` are the repository's.
    async fn base_view(&self, st: &State, branches: &[BranchInfo]) -> Result<BaseView> {
        let own = manifest::target_view(&st.source, &st.head, branches).branch;
        let contained = base::contained_in(&st.base_choice, &st.base_commit, &st.head, own.as_deref());
        Ok(BaseView {
            commit: self.commit_info(&st.base_commit).await?,
            contained_in: match contained {
                Some(reference) => Some(self.contained(st, &reference).await?),
                None => None,
            },
            into: self.merged_into(st).await?,
            head_off_base: st.off_base,
        })
    }

    /// What there is to say of a changeset whose tip `reference` already holds.
    async fn contained(&self, st: &State, reference: &str) -> Result<Contained> {
        let git = &self.acquisition.git;
        let named = match &st.source {
            Source::PullRequest { .. } => self.pull_request_head(&st.source).await?.map(|head| head.target_branch),
            _ => git.default_branch().await?,
        };
        let branch = named.unwrap_or_else(|| base::short_reference(reference));
        let reference_tip = git.resolve_commit(reference).await?;
        let same_tip = reference_tip == st.head;
        let merged_by = if same_tip {
            None
        } else {
            match base::merging_commit(git, &st.head, &reference_tip).await {
                Ok(Some(merge)) => Some(self.commit_info(&merge).await?),
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!(error = e, "the commit that merged the changeset is not found");
                    None
                }
            }
        };
        Ok(Contained {
            branch,
            same_tip,
            merged_by,
        })
    }

    /// The branch a pull request is merged into, when that is not the repository's default branch.
    async fn merged_into(&self, st: &State) -> Result<Option<String>> {
        if !matches!(st.source, Source::PullRequest { .. }) {
            return Ok(None);
        }
        let Some(head) = self.pull_request_head(&st.source).await? else {
            tracing::warn!(source = ?st.source, "the target branch of the pull request is not stored");
            return Ok(None);
        };
        // With no default branch to compare with, the target is said.
        let default = self.acquisition.git.default_branch().await?;
        Ok((default.as_ref() != Some(&head.target_branch)).then_some(head.target_branch))
    }

    /// Looks at the working tree's `HEAD` again and keeps whether it still builds on the base.
    /// Answers whether that changed. Does nothing for another changeset.
    async fn track_head(&self, st: &mut State) -> bool {
        if st.source != Source::WorkingTree {
            return false;
        }
        let off_base = base::head_left(&self.acquisition.git, &st.base_commit).await;
        std::mem::replace(&mut st.off_base, off_base) != off_base
    }

    /// Replaces the repository's review filter, stored whether or not a review
    /// is open, and bumps the revision; completion and the excluded counts
    /// follow with the next manifest. Returns the revision.
    pub async fn set_filter(&self, filter: ReviewFilter) -> Result<u32> {
        self.replace_filter(&mut self.hold().await, filter).await
    }

    /// Adds the file at `path`, or with `directory` everything below the folder at `path`, to the
    /// filter's excluded globs, and bumps the revision unless it was excluded already. Returns
    /// the revision.
    pub async fn exclude_path(&self, path: &str, directory: bool) -> Result<u32> {
        let mut st = self.hold().await;
        let mut filter = st.project_filter().clone();
        let glob = exclude_glob(path, directory);
        if !filter.exclude.contains(&glob) {
            filter.exclude.push(glob);
        }
        self.replace_filter(&mut st, filter).await
    }

    async fn replace_filter(&self, st: &mut Held<'_, S::Writes>, filter: ReviewFilter) -> Result<u32> {
        let filter = filter.normalized();
        filter.validate().map_err(SessionError::Invalid)?;
        if *st.project_filter() == filter && st.project.is_none() {
            return Ok(st.revision);
        }
        let writes = st.writes();
        writes.set_filter(self.repo.clone(), filter.clone()).await?;
        st.filter = filter;
        st.project = None;
        self.refilter(st).await
    }

    /// Narrows the review on view to `filter`, which replaces the project's filter until the
    /// narrowing is reset (`None`), the project's filter is edited or another changeset comes
    /// on view. Nothing is stored. Returns the revision.
    pub async fn narrow(&self, filter: Option<ReviewFilter>) -> Result<u32> {
        let mut st = self.hold().await;
        let project = st.project_filter().clone();
        let (filter, narrowed) = match filter {
            Some(narrowing) => (narrowing.normalized(), true),
            None => (project.clone(), false),
        };
        filter.validate().map_err(SessionError::Invalid)?;
        if st.filter == filter {
            return Ok(st.revision);
        }
        st.filter = filter;
        st.project = narrowed.then_some(project);
        self.refilter(&mut st).await
    }

    /// Counts the review again under `st.filter` as a new revision.
    async fn refilter(&self, st: &mut Held<'_, S::Writes>) -> Result<u32> {
        let writes = st.writes();
        st.revision += 1;
        if st.changes.is_some() {
            for view in st.remapped_views() {
                remap::recount(st, view);
            }
        }
        st.publish();
        self.revisions.advance(st.revision);
        self.note_progress(&writes, st).await?;
        Ok(st.revision)
    }

    pub async fn revision(&self) -> u32 {
        self.published.read().revision
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
    pub async fn target_snapshot(&self) -> Arc<Snapshot> {
        self.published.read().target.clone()
    }

    /// What knows the origin of the session's snapshots, for serving them to a
    /// language-server pool.
    pub fn acquisition(&self) -> Arc<Acquisition<G, S>> {
        self.acquisition.clone()
    }

    /// The initial scan identifies the settled snapshot, so settling does not
    /// report it again.
    pub async fn settled_snapshot(&self) -> SnapshotId {
        self.published.read().target.id
    }

    /// Makes a settled snapshot the target and returns the new revision. While a
    /// review is open it is also the next version: its content is stored and
    /// the changeset gains a version. The revision comes with its diff and the review state
    /// remapped onto it, which are worked out before the token is taken. A failed copy of its
    /// content is not fatal: the diff then reads the worktree, or shows the file as unavailable.
    /// It is the working tree's, wherever that is: on view, or parked while a branch is.
    pub async fn settled(&self, scan: Scan) -> Result<(u32, Option<BlobError>)> {
        let Scan { snapshot, status } = scan;
        let snapshot = Arc::new(snapshot);
        loop {
            let tree = self.working_tree().await;
            let settlement = self.settlement(&tree, &snapshot, &status).await?;
            let mut held = self.hold().await;
            if held.source == Source::WorkingTree {
                if !settlement.fits(&held) {
                    continue;
                }
                let writes = held.writes();
                let settled = self
                    .settle_tree(&writes, &mut held, snapshot, status, settlement)
                    .await?;
                held.publish();
                self.revisions.advance(settled.0);
                return Ok(settled);
            }
            let writes = held.writes();
            let tree = held
                .parked()
                .get_mut(&Source::WorkingTree)
                .expect("the working tree is parked while another changeset is on view");
            if !settlement.fits(tree) {
                continue;
            }
            return self
                .settle_tree(&writes, Arc::make_mut(tree), snapshot, status, settlement)
                .await;
        }
    }

    /// The working tree's state: the one on view, or the one parked while another changeset is.
    async fn working_tree(&self) -> Arc<State> {
        let st = self.published.read();
        if st.source == Source::WorkingTree {
            return st;
        }
        let mut held = self.hold().await;
        let tree = held.parked().get(&Source::WorkingTree);
        Arc::clone(tree.expect("the working tree is parked while another changeset is on view"))
    }

    /// What settling `snapshot` into `tree` needs that takes a while: the content kept for its
    /// version and what it changed, while a review is open, and the diff the revision is shown
    /// with. A diff that cannot be had is left for the first request of the revision to try again.
    async fn settlement(&self, tree: &State, snapshot: &Arc<Snapshot>, status: &SnapshotStatus) -> Result<Settlement> {
        let mut settlement = Settlement {
            base: tree.base.id,
            target: tree.target.id,
            reviewing: tree.review.is_some() && tree.changeset.is_some(),
            retained: None,
            change: None,
            diffed: None,
        };
        if settlement.reviewing {
            let retained = self
                .acquisition
                .retain(
                    &[&*tree.base, &*tree.target],
                    snapshot,
                    status,
                    &BTreeMap::new(),
                    |_, _| {},
                )
                .await;
            settlement.retained = retained.err();
            let sides = (&tree.base, &tree.target);
            settlement.change = Some(self.change_of(sides, &tree.base, snapshot).await?);
        }
        settlement.diffed = self.diff_of(&tree.base, snapshot).await.ok();
        Ok(settlement)
    }

    /// Settles `snapshot` into `st`, which need not be the one on view: only the revision of
    /// the one on view is announced, by the caller.
    async fn settle_tree(
        &self,
        store: &S::Writes,
        st: &mut State,
        snapshot: Arc<Snapshot>,
        status: SnapshotStatus,
        settlement: Settlement,
    ) -> Result<(u32, Option<BlobError>)> {
        // The agent commits on top of the base: the snapshot is built on the `HEAD` of now.
        let previous_head = st.head.clone();
        if st.source == Source::WorkingTree
            && let Ok(head) = self.acquisition.git.head().await
        {
            st.head = head;
        }
        if let (Some(review), Some(changeset)) = (&st.review, st.changeset) {
            let now = (self.clock)();
            store
                .put_snapshot(Snapshot::clone(&snapshot), status.clone(), now)
                .await?;
            let version = Version {
                n: review.version + 1,
                base: st.base.id,
                base_commit: st.base_commit.clone(),
                target: snapshot.id,
                tip: st.head.clone(),
                at: now,
                origin: VersionOrigin::Settled,
                change: settlement.change.expect("worked out while the review was open"),
            };
            let n = version.n;
            store.append_version(changeset, version).await?;
            if st.head != previous_head {
                self.acquisition
                    .git
                    .set_changeset_ref(changeset, n, "tip", &st.head)
                    .await?;
            }
        }
        // Older versions stay materializable (§7.2).
        self.acquisition.record_commit(snapshot.id, st.head.clone());
        let old = std::mem::replace(&mut st.target, snapshot);
        st.target_status = status;
        if let Some(review) = &mut st.review {
            st.known.insert(old.id, old);
            review.version += 1;
            review.updated_at = (self.clock)();
        }
        st.revision += 1;
        st.changes = None;
        self.track_head(st).await;
        if let Some(diffed) = settlement.diffed
            && let Err(e) = self.install_diff(store, st, diffed).await
        {
            tracing::warn!(error = %e, "could not remap the review onto the settled tree");
            st.changes = None;
        }
        Ok((st.revision, settlement.retained))
    }

    /// Settles snapshots of the worktree as `settling` reports them, and looks at the branch
    /// on view when a ref was written, calling `on_settled` with each new revision of the
    /// changeset on view, until dropped. A settled tree that is not on view is not announced:
    /// it is not what is shown. `on_quiet` is called when `settling` finds the settled tree
    /// quiet, `on_edits` when edits start or stop waiting to settle.
    pub async fn follow<W: Watcher + Send, C: Clock + Send + Sync>(
        &self,
        mut settling: Settling<W, C>,
        mut on_settled: impl FnMut(u32) + Send,
        mut on_quiet: impl FnMut() + Send,
        mut on_edits: impl FnMut(bool) + Send,
    ) {
        self.set_degraded(settling.degraded());
        loop {
            let scan = match settling.next_settled(&self.acquisition).await {
                Ok(Settled::Tree(scan)) => scan,
                Ok(Settled::Quiet) => {
                    on_quiet();
                    continue;
                }
                Ok(Settled::Edits(waiting)) => {
                    on_edits(waiting);
                    continue;
                }
                Ok(Settled::RefsMoved) => {
                    match self.observe().await {
                        Ok(Some(revision)) => on_settled(revision),
                        Ok(None) => {}
                        Err(e) => tracing::warn!(error = %e, "could not look at the branch on view"),
                    }
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e.0, "reconciliation failed");
                    continue;
                }
            };
            match self.settled(scan).await {
                Ok((revision, retained)) => {
                    if let Some(e) = retained {
                        tracing::warn!(revision, error = %e, "could not keep the content of the revision");
                    }
                    self.set_degraded(settling.degraded());
                    if self.published.read().source == Source::WorkingTree {
                        on_settled(revision);
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not record a settled snapshot"),
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
