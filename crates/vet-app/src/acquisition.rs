//! Acquisition (§6): maps git and filesystem state into immutable snapshots.
//! Provenance lives here, keyed by `SnapshotId`, and nowhere else (I1).

use std::path::PathBuf;

use vet_core::content::{Snapshot, SnapshotId};
use vet_semantic::ports::{Materialized, Materializer, NotMaterializable};

use crate::ports::{Dirty, GitError, GitRepo, Store};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CommitId(pub String);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PinId(pub u64);

/// Recorded at review open (§6.1). Dirty content is copied into the store, never
/// into git's object database.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pin {
    pub id: PinId,
    pub head: CommitId,
    pub tree: SnapshotId,
}

/// Where a snapshot came from. Never visible to `vet-semantic`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Origin {
    DirtyTree { base_head: CommitId },
    Commit(CommitId),
    PullRequest { head: CommitId, from_fork: bool },
}

/// An evolving tip: its settled snapshots, in order, are the edit timeline.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LogicalTip {
    pub timeline: Vec<SnapshotId>,
}

/// Deletes the materialized directory when the pool drops its server (§7.2).
pub struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Acquisition<G, S> {
    pub git: G,
    pub store: S,
}

impl<G: GitRepo, S: Store> Acquisition<G, S> {
    /// Authoritative reconciliation (§6.2): before declaring a snapshot settled,
    /// at review open, on rescan signals, and before every launch.
    pub async fn reconcile(&self, dirty: Dirty) -> Result<Snapshot, GitError> {
        Ok(self.git.scan(dirty).await?.snapshot)
    }
}

impl<G: GitRepo + Sync, S: Store + Sync> Materializer for Acquisition<G, S> {
    type Guard = DirGuard;

    async fn materialize(&self, _snapshot: SnapshotId) -> Result<Materialized<DirGuard>, NotMaterializable> {
        todo!("look up the commit from provenance; refuse unless every blob is available; git.checkout")
    }
}
