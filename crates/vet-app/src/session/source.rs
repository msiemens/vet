//! Which changeset is on view: building one from its source, switching between
//! them, and a branch's tip and base as they are observed to move.

use std::collections::HashMap;

use vet_core::filter::ReviewFilter;
use vet_core::review::Thread;

use super::state::State;
use super::{LiveSession, Result, SessionError, UnixClock};
use crate::acquisition::Acquisition;
use crate::base;
use crate::changeset::{BaseChoice, ChangesetId, Source, Version, VersionOrigin};
use crate::ports::{GitRepo, LiveStore};
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

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// The source of the changeset on view.
    pub async fn source(&self) -> Source {
        self.state.lock().await.source.clone()
    }

    /// Reads what the store holds of the changeset of `source` in the repository at `repo`.
    pub(super) async fn stored(
        acquisition: &Acquisition<G, S>,
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
            acquisition.store.close_review(id, clock(), false).await?;
        }
        let threads = match changeset {
            Some(changeset) => visible_threads(acquisition.store.threads(changeset).await?, None),
            None => Vec::new(),
        };
        Ok(Stored::Browsing { changeset, threads })
    }

    /// Puts the changeset of `source` on view, in place of the one that is. What the
    /// other had stays as it was for when it comes back. A branch is read afresh: its review
    /// is resumed, else it is browsed against the default base, and a branch that moved while
    /// it was off view gets the version that says so. Returns the new revision, which is
    /// higher than any shown so far, or `None` if `source` was on view already and nothing
    /// moved. Fails, changing nothing, if the branch does not exist.
    pub async fn open_changeset(&self, source: Source) -> Result<Option<u32>> {
        let mut st = self.state.lock().await;
        if st.source == source {
            return self.observe_in(&mut st).await;
        }
        let mut parked = self.parked.lock().await;
        let was_parked = parked.contains_key(&source);
        let mut incoming = match parked.remove(&source) {
            Some(state) => state,
            None => self.enter(&source, &st.filter).await?,
        };
        if let Err(e) = self.observe_in(&mut incoming).await {
            if was_parked {
                parked.insert(source, incoming);
            }
            return Err(e);
        }
        // The filter is the project's, and was perhaps changed while this one was off view.
        incoming.filter = st.filter.clone();
        incoming.revision = st.revision.max(incoming.revision) + 1;
        let outgoing = std::mem::replace(&mut *st, incoming);
        parked.insert(outgoing.source.clone(), outgoing);
        Ok(Some(st.revision))
    }

    /// Looks at the branch on view again: its tip and, for a merge base, where it is now. If
    /// either moved (new commits, an amend, a rebase), the target and the base follow, and while
    /// a review is open that is its next version. Returns the new revision, or `None` if nothing
    /// moved or the changeset on view is not a branch's.
    pub async fn observe(&self) -> Result<Option<u32>> {
        self.observe_in(&mut *self.state.lock().await).await
    }

    async fn observe_in(&self, st: &mut State) -> Result<Option<u32>> {
        let Source::Branch { name } = &st.source else {
            return Ok(None);
        };
        let git = &self.acquisition.git;
        let tip = git
            .branch_tip(name)
            .await
            .map_err(|_| SessionError::NotFound(format!("branch {name}")))?;
        // A merge base follows the history; a base chosen as a branch's tip or a commit stays where it was chosen.
        let base_commit = match &st.base_choice {
            BaseChoice::MergeBase { .. } => {
                let resolved = base::resolve(git, st.base_choice.clone(), &tip).await;
                resolved.map_err(SessionError::Base)?.1
            }
            _ => st.base_commit.clone(),
        };
        if tip == st.head && base_commit == st.base_commit {
            return Ok(None);
        }
        let target = git.commit_snapshot(&tip).await?;
        let base = if base_commit == st.base_commit {
            st.base.clone()
        } else {
            git.commit_snapshot(&base_commit).await?
        };
        let now = (self.clock)();
        if let (Some(review), Some(changeset)) = (&st.review, st.changeset) {
            let store = &self.acquisition.store;
            // The old base's content is kept apart from git, which may drop its objects.
            self.retain_anchored(st, st.anchors()).await?;
            store
                .put_snapshot(target.clone(), SnapshotStatus::of(&target), now)
                .await?;
            store.put_snapshot(base.clone(), SnapshotStatus::of(&base), now).await?;
            let version = Version {
                n: review.version + 1,
                base: base.id,
                base_commit: base_commit.clone(),
                target: target.id,
                tip: tip.clone(),
                at: now,
                origin: VersionOrigin::Observed,
            };
            let previous = store.versions(changeset).await?.pop();
            store.append_version(changeset, version.clone()).await?;
            Self::keep_commits(&self.acquisition, changeset, &version, previous.as_ref()).await?;
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
    async fn enter(&self, source: &Source, filter: &ReviewFilter) -> Result<State> {
        let Source::Branch { name } = source else {
            return Err(SessionError::Invalid("this changeset cannot be opened".into()));
        };
        let acquisition = &self.acquisition;
        let git = &acquisition.git;
        let tip = git
            .branch_tip(name)
            .await
            .map_err(|_| SessionError::NotFound(format!("branch {name}")))?;
        let (changeset, threads) = match Self::stored(acquisition, &self.repo, source, filter, self.clock).await? {
            Stored::Resumed(state) => return Ok(*state),
            Stored::Browsing { changeset, threads } => (changeset, threads),
        };
        let (base_choice, base_commit) = base::default_for_branch(git, &tip).await.map_err(SessionError::Base)?;
        let base = git.commit_snapshot(&base_commit).await?;
        let target = git.commit_snapshot(&tip).await?;
        acquisition.record_commit(base.id, base_commit.clone());
        acquisition.record_commit(target.id, tip.clone());
        Ok(State {
            source: source.clone(),
            revision: 0,
            changeset,
            review: None,
            filter: filter.clone(),
            base,
            base_choice,
            base_commit,
            head: tip,
            target_status: SnapshotStatus::of(&target),
            target,
            known: HashMap::new(),
            recorded: Vec::new(),
            acks: Vec::new(),
            threads,
            changes: None,
        })
    }
}
