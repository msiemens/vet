//! `vet_semantic::ports::Materializer`: a snapshot written out as a directory
//! under Vet's data directory, which is all the semantic layer ever sees of it
//! (I1, §7.2). The directory is kept between launches: see `kept`.

pub mod kept;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use vet_app::acquisition::Acquisition;
use vet_app::ports::{Checkout, CheckoutError, History, Store};
use vet_core::content::SnapshotId;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Materialized, Materializer, NotMaterializable};

use kept::{Error, KeptTrees, Lease};

/// Serves the snapshots an acquisition has recorded.
pub struct Materialize<G, S> {
    acquisition: Arc<Acquisition<G, S>>,
    trees: Arc<KeptTrees>,
}

impl<G, S> Materialize<G, S> {
    pub fn new(acquisition: Arc<Acquisition<G, S>>, trees: Arc<KeptTrees>) -> Self {
        Self { acquisition, trees }
    }
}

impl<G: History + Checkout + Send + Sync, S: Store + Send + Sync> Materialize<G, S> {
    /// Writes the snapshot into `dest`. Refuses unless every blob is available (§6.1).
    async fn build(&self, id: SnapshotId, dest: PathBuf) -> Result<(), NotMaterializable> {
        let other = |why: String| NotMaterializable::Other(why);
        let (commit, snapshot, status) = self.acquisition.recorded(id).await.map_err(other)?;
        if !status.path_collisions.is_empty() && cfg!(any(windows, target_os = "macos")) {
            return Err(other(
                "paths that differ only in case would merge on this filesystem".into(),
            ));
        }
        match self
            .acquisition
            .git
            .checkout(&self.acquisition.store, &commit, &snapshot, &dest)
            .await
        {
            Ok(()) => Ok(()),
            Err(CheckoutError::MissingBlobs(blobs)) => Err(NotMaterializable::MissingBlobs(blobs)),
            Err(CheckoutError::Git(e)) => Err(other(e.0)),
        }
    }
}

impl<G: History + Checkout + Send + Sync, S: Store + Send + Sync> Materializer for Materialize<G, S> {
    type Guard = Lease;

    /// The kept tree of the snapshot, or a new one if there is none. The tree is
    /// locked for as long as the guard lives, and kept after that.
    async fn materialize(
        &self,
        id: SnapshotId,
        policy: AnalysisPolicy,
    ) -> Result<Materialized<Lease>, NotMaterializable> {
        let started = Instant::now();
        let acquired = self
            .trees
            .acquire(id, policy, |dest| self.build(id, dest))
            .await
            .map_err(|e| match e {
                Error::Build(why) => why,
                Error::Io(why) => NotMaterializable::Other(format!("materialize directory: {why}")),
            })
            .inspect_err(|why| tracing::warn!(snapshot = %&id.0.to_hex()[..12], reason = %why, "cannot materialize the snapshot"))?;
        vet_semantic::trace::log(
            || {
                format!(
                    "materialize {} ({})",
                    &id.0.to_hex()[..12],
                    if acquired.built { "built" } else { "kept" }
                )
            },
            started,
        );
        Ok(Materialized {
            root: acquired.lease.tree().to_owned(),
            guard: acquired.lease,
        })
    }

    async fn kept(&self, id: SnapshotId, policy: AnalysisPolicy) -> Option<PathBuf> {
        self.trees.kept(id, policy).await
    }
}
