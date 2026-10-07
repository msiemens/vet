//! Acquisition (§6): maps git and filesystem state into immutable snapshots.
//! Provenance lives here, keyed by `SnapshotId`, and nowhere else (I1).

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use vet_core::content::{BlobId, EntryKind, RepoPath, Snapshot, SnapshotId, SnapshotStatus};

use crate::pin::CommitId;
use crate::ports::{BlobRead, BlobReader, Dirty, GitError, History, Scan, Scanner, Store, StoreError};

pub struct Acquisition<G, S> {
    pub git: G,
    pub store: S,
    /// The commit whose objects back each snapshot's committed blobs.
    commits: Mutex<HashMap<SnapshotId, CommitId>>,
}

impl<G, S> Acquisition<G, S> {
    pub fn new(git: G, store: S) -> Self {
        Self {
            git,
            store,
            commits: Mutex::default(),
        }
    }

    /// Remember that git can reconstruct `snapshot`'s committed blobs from
    /// `commit`; everything else must be in the content store. Needed before
    /// `materialize` can serve the snapshot (§7.2).
    pub fn record_commit(&self, snapshot: SnapshotId, commit: CommitId) {
        self.commits.lock().expect("not poisoned").insert(snapshot, commit);
    }
}

/// Why content could not be read or kept.
#[derive(Debug)]
pub enum BlobError {
    Git(GitError),
    Store(StoreError),
}

impl fmt::Display for BlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => write!(f, "git: {}", e.0),
            Self::Store(e) => write!(f, "store: {}", e.0),
        }
    }
}

impl From<GitError> for BlobError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

impl From<StoreError> for BlobError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl<G: BlobReader, S: Store> Acquisition<G, S> {
    /// The content of `blob`, the file at `path`: the store's copy, else git's
    /// (or the worktree's, if the file still hashes to `blob`). With `capped`,
    /// git content over the capture cap is not read.
    pub async fn blob(&self, path: &RepoPath, blob: BlobId, capped: bool) -> Result<BlobRead, BlobError> {
        if let Some(bytes) = self.store.blob(blob).await? {
            return Ok(BlobRead::Found(bytes));
        }
        Ok(if capped {
            self.git.read_blob_capped(path, blob).await?
        } else {
            match self.git.read_blob(path, blob).await? {
                Some(bytes) => BlobRead::Found(bytes),
                None => BlobRead::Missing,
            }
        })
    }

    /// Makes sure the store holds `blob`, copying it from git or the worktree.
    /// `false` if there is nothing to copy.
    pub async fn keep(&self, path: &RepoPath, blob: BlobId) -> Result<bool, BlobError> {
        if self.store.blob(blob).await?.is_some() {
            return Ok(true);
        }
        match self.git.read_blob(path, blob).await? {
            Some(bytes) => {
                self.store.put_blob(bytes).await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Copies the blobs of `target` that `known` snapshots do not already have
    /// (they are in git or were retained before) into the store, so the
    /// snapshot stays reconstructible (§6.1).
    pub async fn retain(
        &self,
        known: &[&Snapshot],
        target: &Snapshot,
        status: &SnapshotStatus,
    ) -> Result<(), BlobError> {
        for (path, entry) in &target.entries {
            let is_known = known
                .iter()
                .any(|s| s.entries.get(path).is_some_and(|k| k.blob == entry.blob));
            if is_known || entry.kind == EntryKind::Submodule || status.content_unavailable.contains(path) {
                continue;
            }
            if !self.keep(path, entry.blob).await? {
                return Err(GitError(format!("{} changed while being copied", path.to_string_lossy())).into());
            }
        }
        Ok(())
    }
}

impl<G: Scanner, S> Acquisition<G, S> {
    /// Authoritative reconciliation (§6.2): before declaring a snapshot settled,
    /// at open, on rescan signals, and before every launch.
    pub async fn reconcile(&self, dirty: Dirty) -> Result<Scan, GitError> {
        self.git.scan(dirty).await
    }
}

impl<G: History, S: Store> Acquisition<G, S> {
    /// A snapshot whose origin was recorded, with the commit that backs its
    /// committed blobs and what is known about it. The error is for a person to
    /// read.
    pub async fn recorded(&self, id: SnapshotId) -> Result<(CommitId, Snapshot, SnapshotStatus), String> {
        let commit = (self.commits.lock().expect("not poisoned"))
            .get(&id)
            .cloned()
            .ok_or("the snapshot has no recorded origin")?;
        if let Some((snapshot, status)) = self.store.snapshot(id).await.map_err(|e| e.0)? {
            return Ok((commit, snapshot, status));
        }
        // A commit's own tree is not necessarily in the store.
        let snapshot = self.git.commit_snapshot(&commit).await.map_err(|e| e.0)?;
        if snapshot.id != id {
            return Err("the snapshot is not in the store".into());
        }
        let status = SnapshotStatus::of(&snapshot);
        Ok((commit, snapshot, status))
    }
}
