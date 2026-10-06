//! Acquisition (§6): maps git and filesystem state into immutable snapshots.
//! Provenance lives here, keyed by `SnapshotId`, and nowhere else (I1).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use vet_core::content::{EntryKind, Snapshot, SnapshotId, SnapshotStatus};
use vet_semantic::ports::{Materialized, Materializer, NotMaterializable};

use crate::pin::{CommitId, Pin, PinId};
use crate::ports::{CheckoutError, Dirty, GitError, GitRepo, Scan, Store};

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
    /// The commit whose objects back each snapshot's committed blobs.
    commits: Mutex<HashMap<SnapshotId, CommitId>>,
}

/// The initial changeset of a dirty-tree review: `HEAD` to dirty-at-open.
pub struct Opened {
    pub pin: Pin,
    pub base: Snapshot,
    pub target: Snapshot,
    pub status: SnapshotStatus,
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

impl<G: GitRepo, S: Store> Acquisition<G, S> {
    /// Pin on review open (§6.1): record `{head, dirty-at-open}` and copy every
    /// blob git cannot be assumed to keep (anything not already in `HEAD`)
    /// into the store.
    pub async fn open_pin(&self, id: PinId) -> Result<Opened, GitError> {
        let head = self.git.head().await?;
        let base = self.git.commit_snapshot(&head).await?;
        let Scan {
            snapshot: target,
            status,
        } = self.git.scan(Dirty::All).await?;
        for (path, entry) in &target.entries {
            let in_head = base.entries.get(path).is_some_and(|b| b.blob == entry.blob);
            if in_head || entry.kind == EntryKind::Submodule || status.content_unavailable.contains(path) {
                continue;
            }
            let bytes = self
                .git
                .read_blob(path, entry.blob)
                .await?
                .ok_or_else(|| GitError("worktree changed while pinning".into()))?;
            self.store.put_blob(bytes).await.map_err(|e| GitError(e.0))?;
        }
        self.record_commit(base.id, head.clone());
        self.record_commit(target.id, head.clone());
        let pin = Pin {
            id,
            head,
            tree: target.id,
        };
        Ok(Opened {
            pin,
            base,
            target,
            status,
        })
    }

    /// Authoritative reconciliation (§6.2): before declaring a snapshot settled,
    /// at review open, on rescan signals, and before every launch.
    pub async fn reconcile(&self, dirty: Dirty) -> Result<Snapshot, GitError> {
        Ok(self.git.scan(dirty).await?.snapshot)
    }
}

impl<G: GitRepo + Sync, S: Store + Sync> Materializer for Acquisition<G, S> {
    type Guard = DirGuard;

    /// Builds the snapshot in a fresh temporary directory. Refuses unless every
    /// blob is available (§6.1); the semantic layer only ever sees the directory (I1).
    async fn materialize(&self, id: SnapshotId) -> Result<Materialized<DirGuard>, NotMaterializable> {
        let other = |why: String| NotMaterializable::Other(why);
        let commit = self
            .commits
            .lock()
            .expect("not poisoned")
            .get(&id)
            .cloned()
            .ok_or_else(|| other("the snapshot has no recorded origin".into()))?;
        let stored = self.store.snapshot(id).await.map_err(|e| other(e.0))?;
        let (snapshot, status) = match stored {
            Some(found) => found,
            None => {
                // A commit's own tree is not necessarily in the store.
                let snapshot = self.git.commit_snapshot(&commit).await.map_err(|e| other(e.0))?;
                if snapshot.id != id {
                    return Err(other("the snapshot is not in the store".into()));
                }
                (snapshot, SnapshotStatus::default())
            }
        };
        if !status.path_collisions.is_empty() && cfg!(any(windows, target_os = "macos")) {
            return Err(other(
                "paths that differ only in case would merge on this filesystem".into(),
            ));
        }

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join("vet-materialized").join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).map_err(|e| other(format!("{}: {e}", root.display())))?;
        let guard = DirGuard(root.clone());
        match self.git.checkout(&self.store, &commit, &snapshot, &root).await {
            Ok(()) => Ok(Materialized { root, guard }),
            Err(CheckoutError::MissingBlobs(blobs)) => Err(NotMaterializable::MissingBlobs(blobs)),
            Err(CheckoutError::Git(e)) => Err(other(e.0)),
        }
    }
}
