//! Acquisition (§6): maps git and filesystem state into immutable snapshots.
//! Provenance lives here, keyed by `SnapshotId`, and nowhere else (I1).

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};

use vet_core::content::{BlobId, EntryKind, RepoPath, Snapshot, SnapshotId, SnapshotStatus};

use crate::changeset::CommitId;
use crate::ports::{BlobRead, BlobReader, Dirty, GitError, History, Scan, Scanner, Store, StoreError};
use crate::progress::{TaskKind, Tasks};

pub struct Acquisition<G, S> {
    pub git: G,
    pub store: S,
    /// The commit whose objects back each snapshot's committed blobs.
    commits: Arc<Mutex<HashMap<SnapshotId, CommitId>>>,
    /// Where scans and diffs report.
    pub tasks: Tasks,
}

impl<G, S> Acquisition<G, S> {
    pub fn new(git: G, store: S) -> Self {
        Self {
            git,
            store,
            commits: Arc::default(),
            tasks: Tasks::default(),
        }
    }

    /// The same acquisition over other handles on the same repository and store: what
    /// it knows of the snapshots is shared, so one lane records a commit and another
    /// serves its snapshot. The handles tell the lanes apart by what they draw on.
    pub fn through<G2, S2>(&self, git: G2, store: S2) -> Acquisition<G2, S2> {
        Acquisition {
            git,
            store,
            commits: self.commits.clone(),
            tasks: self.tasks.clone(),
        }
    }

    /// Reports scans and diffs to `tasks`.
    pub fn reporting(mut self, tasks: Tasks) -> Self {
        self.tasks = tasks;
        self
    }

    /// Remember that git can reconstruct `snapshot`'s committed blobs from
    /// `commit`; everything else must be in the content store. Needed before
    /// `materialize` can serve the snapshot (§7.2).
    pub fn record_commit(&self, snapshot: SnapshotId, commit: CommitId) {
        self.commits.lock().expect("not poisoned").insert(snapshot, commit);
    }
}

/// How many bytes of content [`Acquisition::retain`] holds before it puts them: a batch syncs
/// each of its directories once, and a large changeset is not read into memory at once.
pub const RETAIN_BATCH: usize = 32 << 20;

/// Why content could not be read or kept.
#[derive(Debug)]
pub enum BlobError {
    Git(GitError),
    Store(StoreError),
}

impl fmt::Display for BlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => write!(f, "git: {e}"),
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
        if self.store.missing_blobs(vec![blob]).await?.is_empty() {
            return Ok(true);
        }
        match self.git.read_blob(path, blob).await? {
            Some(bytes) => {
                self.store.put_blobs(vec![bytes]).await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Copies the blobs of `target` that `known` snapshots do not already have
    /// (they are in git or were retained before) and the store does not hold
    /// into the store, so the snapshot stays reconstructible (§6.1). They are
    /// taken from `read` if it has them, else read from git or the worktree, and
    /// put in batches of about [`RETAIN_BATCH`] bytes. Fails if a blob's content
    /// is not what `target` says it is: the file changed while it was copied.
    /// `report` is told how many of how many missing blobs were taken.
    pub async fn retain(
        &self,
        known: &[&Snapshot],
        target: &Snapshot,
        status: &SnapshotStatus,
        read: &BTreeMap<BlobId, Vec<u8>>,
        report: impl Fn(u32, u32),
    ) -> Result<(), BlobError> {
        let mut wanted: BTreeMap<BlobId, &RepoPath> = BTreeMap::new();
        for (path, entry) in &target.entries {
            let is_known = known
                .iter()
                .any(|s| s.entries.get(path).is_some_and(|k| k.blob == entry.blob));
            if is_known || entry.kind == EntryKind::Submodule || status.content_unavailable.contains(path) {
                continue;
            }
            wanted.entry(entry.blob).or_insert(path);
        }
        let changed =
            |path: &RepoPath| GitError::failed(format!("{} changed while being copied", path.to_string_lossy()));
        let put = async |batch: Vec<(BlobId, Vec<u8>)>| -> Result<(), BlobError> {
            let (ids, contents): (Vec<BlobId>, Vec<Vec<u8>>) = batch.into_iter().unzip();
            let put = self.store.put_blobs(contents).await?;
            match ids.iter().zip(&put).find(|(id, put)| id != put) {
                Some((id, _)) => Err(changed(wanted[id]).into()),
                None => Ok(()),
            }
        };
        let missing = self.store.missing_blobs(wanted.keys().copied().collect()).await?;
        let (mut batch, mut bytes) = (Vec::new(), 0);
        let total = missing.len() as u32;
        for (done, blob) in missing.into_iter().enumerate() {
            report(done as u32, total);
            let path = wanted[&blob];
            let content = match read.get(&blob) {
                Some(content) => content.clone(),
                None => self.git.read_blob(path, blob).await?.ok_or_else(|| changed(path))?,
            };
            bytes += content.len();
            batch.push((blob, content));
            if bytes >= RETAIN_BATCH {
                put(std::mem::take(&mut batch)).await?;
                bytes = 0;
            }
        }
        put(batch).await?;
        report(total, total);
        Ok(())
    }
}

impl<G: Scanner, S> Acquisition<G, S> {
    /// Authoritative reconciliation (§6.2): before declaring a snapshot settled,
    /// at open, and on rescan signals.
    pub async fn reconcile(&self, dirty: Dirty) -> Result<Scan, GitError> {
        self.tasks
            .track(TaskKind::Scan, "Scanning the working tree", self.git.scan(dirty))
            .await
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
        let snapshot = self.git.commit_snapshot(&commit).await.map_err(|e| e.to_string())?;
        if snapshot.id != id {
            return Err("the snapshot is not in the store".into());
        }
        let status = SnapshotStatus::of(&snapshot);
        Ok((commit, snapshot, status))
    }
}
