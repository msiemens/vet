//! How many lines a changeset of the list changes, for every row and not just the one on view:
//! the line delta between the commit its base is and the commit its tip is.

use std::sync::Arc;

use vet_core::content::{LineDelta, Snapshot};
use vet_engine::Computed;

use super::LiveSession;
use super::derived::LinesBetween;
use super::source::tip_of;
use crate::acquisition::Acquisition;
use crate::base;
use crate::changeset::{CommitId, Source};
use crate::ports::{GitError, GitRepo, LiveStore};

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
        let derived = Arc::clone(&self.facts.derived);
        let key = LinesBetween {
            base: base.clone(),
            tip: tip.clone(),
        };
        let counted = self
            .facts
            .lines
            .get(key, move |_| async move {
                let old = derived.snapshot(&acquisition, &base).await?;
                let new = derived.snapshot(&acquisition, &tip).await?;
                lines_between(&acquisition, &old, &new)
                    .await
                    .map(|d| Computed::new(d, true))
            })
            .await;
        Ok(counted.ok().map(|got| got.value))
    }

    /// The commit the changeset of `source`, whose tip is `tip`, is compared against: the base its
    /// open review recorded, else the one for its kind. `None` if that does not resolve. The same
    /// rule the review view uses, so the queue's line counts and the view agree.
    async fn base_of(&self, source: &Source, tip: &CommitId) -> super::Result<Option<CommitId>> {
        let (git, store) = (&self.acquisition.git, &self.acquisition.store);
        let mut chosen = None;
        // Only an open review pins the base; a finished one left the changeset browsing, which the
        // view reads against the kind's base again.
        if let Some(changeset) = store.changeset(self.repo.clone(), source.clone()).await?
            && let Some(review) = store.open_review(changeset).await?
        {
            chosen = store.review(review).await?.map(|r| r.base);
        }
        let base = match chosen {
            Some(choice) => base::resolve(git, choice, tip).await.ok().map(|(_, commit)| commit),
            None => self.base_for(source, tip).await.ok().map(|(_, commit)| commit),
        };
        Ok(base)
    }
}

async fn lines_between<G: GitRepo, S: LiveStore + Sync>(
    acquisition: &Acquisition<G, S>,
    old: &Snapshot,
    new: &Snapshot,
) -> Result<LineDelta, GitError> {
    let diff = acquisition.git.diff(&acquisition.store, old, new).await?.diff;
    Ok(diff.files.iter().map(|file| LineDelta::of(&file.hunks)).sum())
}
