//! The working tree's dirty content, copied into the store ahead of the review that keeps it.
//! Starting a review copies what the target has that the base does not (§6.1); on a large
//! changeset that is most of the first mark's time. Copied while the session browses, the
//! first mark finds the content there and copies only what changed since. A copy is a flight
//! of `vet-engine`, so the first mark joins one under way instead of copying the same files.
//! What is never referenced by a review is removed by the store's collection, after its grace.

use std::sync::Arc;

use vet_core::content::{Snapshot, SnapshotId, SnapshotStatus};
use vet_engine::{Computed, Kind, Retention};

use super::{LiveSession, Result, SessionError};
use crate::changeset::Source;
use crate::ports::{GitRepo, LiveStore};

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

impl<G, S> LiveSession<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    /// Copies the content of `target` that `base` does not have into the store; joins the copy
    /// of the pair under way, if there is one.
    pub(super) async fn retained(&self, base: &Snapshot, target: &Snapshot, status: &SnapshotStatus) -> Result<()> {
        let key = Retained {
            base: base.id,
            target: target.id,
        };
        let compute = |_| {
            let acquisition = Arc::clone(&self.acquisition);
            let (base, target, status) = (base.clone(), target.clone(), status.clone());
            async move {
                acquisition.retain(&[&base], &target, &status).await?;
                Ok(Computed::new((), false))
            }
        };
        self.ahead.get(key, compute).await?;
        Ok(())
    }

    /// Copies the working tree's dirty content into the store, as starting a review of it
    /// would. Does nothing unless the working tree is on view and no review of it is open.
    pub async fn copy_ahead(&self) -> Result<()> {
        let (base, target, status) = {
            let st = self.state.lock().await;
            if st.source != Source::WorkingTree || st.review.is_some() {
                return Ok(());
            }
            (st.base.clone(), st.target.clone(), st.target_status.clone())
        };
        self.retained(&base, &target, &status).await
    }
}
