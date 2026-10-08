//! The working tree's dirty content, copied into the store ahead of the review that keeps it.
//! Starting a review copies what the target has that the base does not (§6.1); on a large
//! changeset that is most of the first mark's time. Copied while the session browses, the
//! first mark finds the content there and copies only what changed since. A copy is a flight
//! of `vet-engine`, so the first mark joins one under way instead of copying the same files.
//! What is never referenced by a review is removed by the store's collection, after its grace.

use std::sync::Arc;

use vet_core::content::{Snapshot, SnapshotId, SnapshotStatus};
use vet_engine::{Computed, Engine, Kind, Limits, Progress, Retention};

use super::derived::Diffed;
use super::{LiveSession, Result, SessionError};
use crate::acquisition::Acquisition;
use crate::changeset::Source;
use crate::ports::{GitRepo, LiveStore};
use crate::progress::{TaskKind, Tasks};

/// The content `target` has that `base` does not, in the store.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Retained {
    base: SnapshotId,
    target: SnapshotId,
}

impl Kind for Retained {
    type Value = ();
    type Error = SessionError;

    /// The store, not the engine, knows what it holds: a copy that finished is asked again,
    /// and the store answers which blobs are missing.
    fn retention(&self) -> Retention {
        Retention::InFlight
    }
}

/// The engine of the copies, each reporting as a task with how many files it stored.
pub(super) fn copies(tasks: &Tasks) -> Engine<Retained> {
    let tasks = tasks.clone();
    Engine::new(Limits::entries(0))
        .observing(move |_| Some(Arc::new(tasks.start(TaskKind::Store, "Storing changed files")) as _))
}

impl<G, S> LiveSession<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    /// Copies the content of `target` that `base` does not have into the store through
    /// `acquisition`; joins the copy of the pair under way, if there is one, whatever it copies
    /// through. The texts come from `diffed`, the diff of the pair, which read them already;
    /// the rest (binaries, files over the cap) from git or the worktree.
    pub(super) async fn retained(
        &self,
        acquisition: &Arc<Acquisition<G, S>>,
        (base, target, status): (&Arc<Snapshot>, &Arc<Snapshot>, &SnapshotStatus),
        diffed: Arc<Diffed>,
    ) -> Result<()> {
        let key = Retained {
            base: base.id,
            target: target.id,
        };
        let compute = |progress: Progress| {
            let acquisition = Arc::clone(acquisition);
            let (base, target, status) = (Arc::clone(base), Arc::clone(target), status.clone());
            async move {
                let report = |done, total| progress.report(Some((done, total)), None);
                acquisition
                    .retain(&[&*base], &target, &status, &diffed.blobs, report)
                    .await?;
                Ok(Computed::new((), false))
            }
        };
        self.ahead.get(key, compute).await?;
        Ok(())
    }

    /// Copies the working tree's dirty content into the store, as starting a review of it
    /// would, through the handles of [`LiveSession::copying_through`]. Does nothing unless the
    /// working tree is on view and no review of it is open.
    pub async fn copy_ahead(&self) -> Result<()> {
        let (base, target, status) = {
            let st = self.hold().await;
            if st.source != Source::WorkingTree || st.review.is_some() {
                return Ok(());
            }
            (st.base.clone(), st.target.clone(), st.target_status.clone())
        };
        let diffed = self.diff_of(&base, &target).await?;
        self.retained(&self.copying, (&base, &target, &status), diffed).await
    }
}
