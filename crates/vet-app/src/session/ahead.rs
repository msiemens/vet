//! The working tree's dirty content, copied into the store ahead of the review that keeps it.
//! Starting a review copies what the target has that the base does not (§4.1); on a large
//! changeset that is most of the first mark's time. Copied while the session browses, the
//! first mark finds the content there and copies only what changed since. A copy is a flight
//! of `vet-engine`, so the first mark joins one under way instead of copying the same files,
//! and one that succeeded is kept per snapshot pair, so asking again costs nothing.
//! What is never referenced by a review is removed by the store's collection, after its grace.

use std::pin::Pin;
use std::sync::Arc;

use vet_core::content::{Snapshot, SnapshotId, SnapshotStatus};
use vet_engine::{Computed, Engine, Kind, Limits, Progress, Retention};

use super::derived::Diffed;
use super::{LiveSession, Result, SessionError};
use crate::acquisition::Acquisition;
use crate::changeset::Source;
use crate::ports::{GitRepo, LiveStore, Retainer};
use crate::progress::TaskKind;

/// The content `target` has that `base` does not, in the store.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Retained {
    base: SnapshotId,
    target: SnapshotId,
}

impl Kind for Retained {
    type Value = ();
    type Error = SessionError;

    /// Content leaves the store only in the collection at store open
    /// (`vet_backend::open_store`), so a pair whose copy succeeded stays stored for the session.
    fn retention(&self) -> Retention {
        Retention::Kept
    }
}

/// The copies of the working tree's content into the store under way.
pub(super) struct Copies<G, S> {
    /// Each copy reports as a task with how many files it stored.
    engine: Engine<Retained>,
    /// What a copy ahead of a review reads and writes through: the session's acquisition over
    /// handles of their own, so that it draws on what nobody waits for.
    through: Arc<Acquisition<G, S>>,
}

impl<G, S> Copies<G, S> {
    pub(super) fn new(acquisition: &Arc<Acquisition<G, S>>) -> Self {
        let tasks = acquisition.tasks.clone();
        Self {
            engine: Engine::new(Limits::entries(16))
                .observing(move |_| Some(Arc::new(tasks.start(TaskKind::Store, "Storing changed files")) as _)),
            through: Arc::clone(acquisition),
        }
    }

    /// The same copies, reading and writing through `through`.
    pub(super) fn reading_through(self, through: Acquisition<G, S>) -> Self {
        Self {
            through: Arc::new(through),
            ..self
        }
    }
}

impl<G, S> Copies<G, S>
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
                Ok(Computed::new((), true))
            }
        };
        self.engine.get(key, compute).await?;
        Ok(())
    }
}

impl<G, S> LiveSession<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    /// Makes sure the changed content of the working tree on view is in the store (§4.1),
    /// without the token. The one way in for the queries and the copy ahead; joins a copy
    /// under way, and a pair already stored is answered at once. The diff is the kept one
    /// where there is one, for it read the texts already. A file that cannot be copied fails
    /// the call, and the queries of a file whose copy went missing answer from the disk.
    pub async fn kept_content(&self) -> Result<()> {
        let (base, target, status, kept) = {
            let st = self.published.read();
            if st.source != Source::WorkingTree {
                return Ok(());
            }
            let key = Retained {
                base: st.base.id,
                target: st.target.id,
            };
            if self.copies.engine.peek(&key).is_some() {
                return Ok(());
            }
            let kept = st.changes.as_ref().map(|c| Arc::clone(&c.diffed));
            (st.base.clone(), st.target.clone(), st.target_status.clone(), kept)
        };
        let diffed = match kept {
            Some(kept) => kept,
            None => self.facts.diff_of(&base, &target).await?,
        };
        self.copies
            .retained(&self.copies.through, (&base, &target, &status), diffed)
            .await
    }
}

impl<G, S> Retainer for LiveSession<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    fn retain(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let _ = self.kept_content().await;
        })
    }
}
