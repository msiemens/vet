//! How many lines a changeset of the list changes, for every row and not just the one on view:
//! the line delta between the commit its base is and the commit its tip is.

use std::sync::Arc;

use vet_core::content::LineDelta;
use vet_engine::{Computed, Kind, Retention};

use super::LiveSession;
use super::source::tip_of;
use crate::acquisition::Acquisition;
use crate::base;
use crate::changeset::{CommitId, Source};
use crate::ports::{GitRepo, LiveStore};
use crate::pull_request::PullRequestRefs;

/// The lines between two commits. The key names both, and what git has of a commit never changes,
/// so the value is kept for as long as the engine has room.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct LinesBetween {
    base: CommitId,
    tip: CommitId,
}

impl Kind for LinesBetween {
    type Value = LineDelta;
    /// Fit to show.
    type Error = String;

    fn retention(&self) -> Retention {
        Retention::Kept
    }
}

impl<G, S> LiveSession<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    /// The lines the changeset of `source` changes against its base, counted like a diff of the
    /// two commits does. `None` for the working tree, whose manifest is live, and for a changeset
    /// whose tip or base cannot be had: a pull request that was not fetched, a branch without a
    /// common ancestor with the default one.
    pub async fn lines_of(&self, source: &Source) -> super::Result<Option<LineDelta>> {
        let git = &self.acquisition.git;
        if *source == Source::WorkingTree {
            return Ok(None);
        }
        let Ok(tip) = tip_of(git, source).await else {
            return Ok(None);
        };
        let Some(base) = self.base_of(source, &tip).await? else {
            return Ok(None);
        };
        let acquisition = Arc::clone(&self.acquisition);
        let key = LinesBetween {
            base: base.clone(),
            tip: tip.clone(),
        };
        let counted = self
            .lines
            .get(key, move |_| async move {
                lines_between(&acquisition, &base, &tip)
                    .await
                    .map(|d| Computed::new(d, true))
            })
            .await;
        Ok(counted.ok().map(|got| got.value))
    }

    /// The commit the changeset of `source`, whose tip is `tip`, is compared against: the base its
    /// latest review chose, else the default one for its kind. `None` if that does not resolve.
    async fn base_of(&self, source: &Source, tip: &CommitId) -> super::Result<Option<CommitId>> {
        let (git, store) = (&self.acquisition.git, &self.acquisition.store);
        let mut chosen = None;
        if let Some(changeset) = store.changeset(self.repo.clone(), source.clone()).await?
            && let Some(review) = store.latest_review(changeset).await?
        {
            chosen = store.review(review).await?.map(|r| r.base);
        }
        let resolved = match (chosen, source) {
            (Some(choice), _) => base::resolve(git, choice, tip).await,
            (None, Source::PullRequest { remote, number }) => {
                base::default_for_pull_request(git, &PullRequestRefs::of(remote, *number), tip).await
            }
            (None, _) => base::default_for_branch(git, tip).await,
        };
        Ok(resolved.ok().map(|(_, commit)| commit))
    }
}

async fn lines_between<G: GitRepo, S: LiveStore + Sync>(
    acquisition: &Acquisition<G, S>,
    base: &CommitId,
    tip: &CommitId,
) -> Result<LineDelta, String> {
    let git = &acquisition.git;
    let old = git.commit_snapshot(base).await.map_err(|e| e.0)?;
    let new = git.commit_snapshot(tip).await.map_err(|e| e.0)?;
    let diff = git.diff(&acquisition.store, &old, &new).await.map_err(|e| e.0)?.diff;
    Ok(diff.files.iter().map(|file| LineDelta::of(&file.hunks)).sum())
}
