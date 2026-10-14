//! Which changeset is on view: building one from its source, switching between
//! them, and a branch's tip and base as they are observed to move.

use std::collections::HashMap;
use std::sync::Arc;

use vet_core::filter::ReviewFilter;
use vet_core::review::Thread;

use super::hold::Held;
use super::state::State;
use super::{LiveSession, Result, SessionError, UnixClock};
use crate::acquisition::Acquisition;
use crate::base;
use crate::changeset::{BaseChoice, ChangesetId, CommitId, Source, Version, VersionOrigin};
use crate::platform::PullRequestHead;
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::pull_request::PullRequestRefs;
use crate::review::visible_threads;
use vet_core::content::SnapshotStatus;

/// What the store holds of a changeset that is about to be put on view.
pub(super) enum Stored {
    /// Its open review, as it was left.
    Resumed(Box<State>),
    /// No review is open (or the open one could not be reconstructed): the changeset is browsed,
    /// with the threads that are still open.
    Browsing {
        changeset: Option<ChangesetId>,
        threads: Vec<Thread>,
    },
}

/// The commit the target of the changeset of `source` is built on: a branch's tip, or a pull
/// request's head as it was fetched. The working tree has none to look up.
pub(super) async fn tip_of<G: GitRepo>(git: &G, source: &Source) -> Result<CommitId> {
    match source {
        Source::WorkingTree => Err(SessionError::Invalid("the working tree is not a commit".into())),
        Source::Branch { name } => git
            .branch_tip(name)
            .await?
            .ok_or_else(|| SessionError::NotFound(format!("branch {name}"))),
        Source::PullRequest { remote, number } => git
            .resolve_commit(&PullRequestRefs::of(remote, *number).head)
            .await?
            .ok_or_else(|| SessionError::NotFound(format!("pull request #{number}, which has not been fetched"))),
    }
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The source of the changeset on view.
    pub async fn source(&self) -> Source {
        self.published.read().source.clone()
    }

    /// How the platform last named the pull request of `source`, as stored.
    pub async fn pull_request_head(&self, source: &Source) -> Result<Option<PullRequestHead>> {
        let store = &self.acquisition.store;
        Ok(store.pull_request_head(self.repo.clone(), source.clone()).await?)
    }

    /// Stores how the platform names the pull request of `source` now, unless that is stored already.
    /// Answers whether it was not.
    pub async fn remember_pull_request_head(&self, source: &Source, head: PullRequestHead) -> Result<bool> {
        if self.pull_request_head(source).await?.as_ref() == Some(&head) {
            return Ok(false);
        }
        self.hold()
            .await
            .writes()
            .set_pull_request_head(self.repo.clone(), source.clone(), head)
            .await?;
        Ok(true)
    }

    /// Reads what the store holds of the changeset of `source` in the repository at `repo`; a
    /// review that cannot be reconstructed is closed through `writes`.
    pub(super) async fn stored(
        acquisition: &Acquisition<G, S>,
        writes: &S::Writes,
        repo: &str,
        source: &Source,
        filter: &ReviewFilter,
        clock: UnixClock,
    ) -> Result<Stored> {
        let changeset = acquisition.store.changeset(repo.to_owned(), source.clone()).await?;
        if let Some(changeset) = changeset
            && let Some(id) = acquisition.store.open_review(changeset).await?
        {
            if let Some(state) = Self::resumed(acquisition, changeset, id, filter.clone(), source.clone()).await? {
                return Ok(Stored::Resumed(Box::new(state)));
            }
            // Its start record or snapshots are gone: browse rather than fail to open.
            acquisition.git.delete_changeset_refs(changeset).await?;
            writes.close_review(id, clock(), false).await?;
        }
        let threads = match changeset {
            Some(changeset) => visible_threads(acquisition.store.threads(changeset).await?, None),
            None => Vec::new(),
        };
        Ok(Stored::Browsing { changeset, threads })
    }

    /// Puts the changeset of `source` on view, in place of the one that is. What the
    /// other had stays as it was for when it comes back. A branch is read afresh: its review
    /// is resumed, else it is browsed against its base, and a branch that moved while
    /// it was off view gets the version that says so. Returns the new revision, which is
    /// higher than any shown so far, or `None` if `source` was on view already and nothing
    /// moved. Fails, changing nothing, if the branch does not exist.
    pub async fn open_changeset(&self, source: Source) -> Result<Option<u32>> {
        let mut st = self.hold().await;
        if st.source == source {
            return self.observe_on_view(&mut st).await;
        }
        let writes = st.writes();
        let was_parked = st.parked().contains_key(&source);
        let mut incoming = match st.parked().remove(&source) {
            Some(state) => state,
            None => Arc::new(self.enter(&writes, &source, st.project_filter()).await?),
        };
        if let Err(e) = self.observe_in(&writes, Arc::make_mut(&mut incoming)).await {
            if was_parked {
                st.parked().insert(source, incoming);
            }
            return Err(e);
        }
        let tree = Arc::make_mut(&mut incoming);
        // The filter is the project's, and was perhaps changed while this one was off view; a
        // narrowing ends when its changeset leaves the view.
        tree.filter = st.project_filter().clone();
        tree.project = None;
        tree.revision = st.revision.max(tree.revision) + 1;
        let outgoing = st.replace(incoming);
        st.parked().insert(outgoing.source.clone(), outgoing);
        st.publish();
        self.revisions.advance(st.revision);
        Ok(Some(st.revision))
    }

    /// Looks at the branch on view again: its tip and, for a merge base, where it is now. If
    /// either moved (new commits, an amend, a rebase), the target and the base follow, and while
    /// a review is open that is its next version. For the working tree it looks at whether `HEAD`
    /// still builds on the base. Returns the new revision, or `None` if nothing moved.
    pub async fn observe(&self) -> Result<Option<u32>> {
        self.observe_on_view(&mut self.hold().await).await
    }

    /// [`LiveSession::observe_in`] on the state on view, whose new revision is announced.
    async fn observe_on_view(&self, st: &mut Held<'_, S::Writes>) -> Result<Option<u32>> {
        let writes = st.writes();
        let moved = self.observe_in(&writes, st).await?;
        if let Some(revision) = moved {
            st.publish();
            self.revisions.advance(revision);
        }
        Ok(moved)
    }

    /// Looks at the branch of `st`, which need not be the one on view: only the
    /// revision of the one on view is announced, by the caller.
    async fn observe_in(&self, store: &S::Writes, st: &mut State) -> Result<Option<u32>> {
        if st.source == Source::WorkingTree {
            if !self.track_head(st).await {
                return Ok(None);
            }
            st.revision += 1;
            st.changes = None;
            return Ok(Some(st.revision));
        }
        let git = &self.acquisition.git;
        let tip = tip_of(git, &st.source).await?;
        // A merge base follows the history; a base on a commit stays where it is.
        let base_commit = match &st.base_choice {
            BaseChoice::MergeBase { .. } => base::resolve(git, st.base_choice.clone(), &tip).await?.1,
            _ => st.base_commit.clone(),
        };
        if tip == st.head && base_commit == st.base_commit {
            return Ok(None);
        }
        let target = Arc::new(git.commit_snapshot(&tip).await?);
        let base = if base_commit == st.base_commit {
            st.base.clone()
        } else {
            Arc::new(git.commit_snapshot(&base_commit).await?)
        };
        let now = (self.clock)();
        if let (Some(review), Some(changeset)) = (&st.review, st.changeset) {
            // The old base's content is kept apart from git, which may drop its objects.
            self.retain_anchored(st, st.anchors()).await?;
            let version = Version {
                n: review.version + 1,
                base: base.id,
                base_commit: base_commit.clone(),
                target: target.id,
                tip: tip.clone(),
                at: now,
                origin: VersionOrigin::Observed,
                change: self.change_of((&st.base, &st.target), &base, &target).await?,
            };
            let (base_status, target_status) = (SnapshotStatus::of(&base), SnapshotStatus::of(&target));
            let snapshots = [(&*target, &target_status), (&*base, &base_status)];
            let previous = Some((&st.base_commit, &st.head));
            let write = store.append_version(changeset, version.clone());
            self.record_version(store, changeset, &version, previous, &snapshots, write)
                .await?;
            if let Some(review) = &mut st.review {
                review.version = version.n;
                review.updated_at = now;
            }
        }
        self.acquisition.record_commit(base.id, base_commit.clone());
        self.acquisition.record_commit(target.id, tip.clone());
        st.target_status = SnapshotStatus::of(&target);
        let old = std::mem::replace(&mut st.target, target);
        if st.review.is_some() {
            st.known.insert(old.id, old);
        }
        st.base = base;
        st.base_commit = base_commit;
        st.head = tip;
        st.revision += 1;
        st.changes = None;
        Ok(Some(st.revision))
    }

    /// The state of a changeset that was never on view in this session. The working tree
    /// always was.
    async fn enter(&self, writes: &S::Writes, source: &Source, filter: &ReviewFilter) -> Result<State> {
        let acquisition = &self.acquisition;
        let git = &acquisition.git;
        let tip = tip_of(git, source).await?;
        let stored = Self::stored(acquisition, writes, &self.repo, source, filter, self.clock).await?;
        let (changeset, threads) = match stored {
            Stored::Resumed(state) => return Ok(*state),
            Stored::Browsing { changeset, threads } => (changeset, threads),
        };
        let (base_choice, base_commit) = match source {
            Source::PullRequest { remote, number } => {
                base::for_pull_request(git, &PullRequestRefs::of(remote, *number), &tip).await
            }
            _ => base::for_branch(git, &tip).await,
        }?;
        let base = Arc::new(git.commit_snapshot(&base_commit).await?);
        let target = Arc::new(git.commit_snapshot(&tip).await?);
        acquisition.record_commit(base.id, base_commit.clone());
        acquisition.record_commit(target.id, tip.clone());
        Ok(State {
            source: source.clone(),
            revision: 0,
            seq: 0,
            changeset,
            review: None,
            filter: filter.clone(),
            project: None,
            base,
            base_choice,
            base_commit,
            head: tip,
            off_base: false,
            target_status: SnapshotStatus::of(&target),
            target,
            known: HashMap::new(),
            recorded: Vec::new(),
            acks: Vec::new(),
            threads,
            discussion: None,
            changes: None,
        })
    }
}
