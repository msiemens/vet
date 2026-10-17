// Each test module uses a part of this one.
#![allow(dead_code, clippy::disallowed_methods)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use vet_app::acquisition::Acquisition;
use vet_app::changeset::CommitId;
use vet_app::ports::{Dirty, GitRepo, Store, StoreError, StoredBlob};
use vet_core::content::{BlobId, Snapshot, SnapshotId, SnapshotStatus, keeps_bytes};

/// An in-memory content store that keeps what `SqliteStore` keeps.
#[derive(Default)]
pub struct MemStore {
    blobs: Mutex<HashMap<BlobId, StoredBlob>>,
    snapshots: Mutex<HashMap<SnapshotId, (Snapshot, SnapshotStatus)>>,
}

impl Store for MemStore {
    async fn put_blobs(&self, blobs: Vec<Vec<u8>>) -> Result<Vec<BlobId>, StoreError> {
        let mut held = self.blobs.lock().unwrap();
        Ok(blobs
            .into_iter()
            .map(|bytes| {
                let id = BlobId::of(&bytes);
                let kept = if keeps_bytes(&bytes) {
                    StoredBlob::Bytes(bytes)
                } else {
                    StoredBlob::Binary {
                        size: bytes.len() as u64,
                    }
                };
                held.insert(id, kept);
                id
            })
            .collect())
    }
    async fn missing_blobs(&self, ids: Vec<BlobId>) -> Result<Vec<BlobId>, StoreError> {
        let held = self.blobs.lock().unwrap();
        Ok(ids.into_iter().filter(|id| !held.contains_key(id)).collect())
    }
    async fn blob(&self, id: BlobId) -> Result<Option<StoredBlob>, StoreError> {
        Ok(self.blobs.lock().unwrap().get(&id).cloned())
    }
    async fn blobs(&self, ids: Vec<BlobId>) -> Result<BTreeMap<BlobId, StoredBlob>, StoreError> {
        let held = self.blobs.lock().unwrap();
        Ok(ids
            .into_iter()
            .filter_map(|id| Some((id, held.get(&id)?.clone())))
            .collect())
    }
    async fn snapshot(&self, id: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
        Ok(self.snapshots.lock().unwrap().get(&id).cloned())
    }
}

impl MemStore {
    /// What a review does to the store when it keeps a snapshot.
    pub fn put_snapshot(&self, snapshot: Snapshot, status: SnapshotStatus) {
        self.snapshots.lock().unwrap().insert(snapshot.id, (snapshot, status));
    }
}

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
    acq.retain(&[&base], &scan.snapshot, &scan.status, &Default::default(), |_, _| {})
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
