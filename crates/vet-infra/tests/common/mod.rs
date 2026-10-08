// Each test binary uses a part of this module.
#![allow(dead_code)]

use vet_app::acquisition::Acquisition;
use vet_app::changeset::CommitId;
use vet_app::ports::{Dirty, GitRepo, Store};
use vet_core::content::{Snapshot, SnapshotStatus};

/// `HEAD`, its tree and the working tree as a review started there would
/// record them, with the working tree's dirty content retained.
pub struct Started {
    pub head: CommitId,
    pub base: Snapshot,
    pub target: Snapshot,
    pub status: SnapshotStatus,
}

pub async fn start_at_head<G: GitRepo, S: Store>(acq: &Acquisition<G, S>) -> Started {
    let head = acq.git.head().await.unwrap();
    let base = acq.git.commit_snapshot(&head).await.unwrap();
    let scan = acq.reconcile(Dirty::All).await.unwrap();
    acq.retain(&[&base], &scan.snapshot, &scan.status, &Default::default())
        .await
        .unwrap();
    acq.record_commit(base.id, head.clone());
    acq.record_commit(scan.snapshot.id, head.clone());
    Started {
        head,
        base,
        target: scan.snapshot,
        status: scan.status,
    }
}
