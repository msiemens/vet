//! `vet_app::ports::GitRepo` over gitoxide. No git subprocesses, hooks,
//! fsmonitor, or filter and diff drivers (§6.3).

mod diff;
mod merkle;
mod scan;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gix::bstr::ByteSlice;
use gix::objs::Find;
use vet_app::acquisition::CommitId;
use vet_app::ports::{Dirty, GitError, GitRepo, Scan, Store};
use vet_core::content::{BlobId, RepoPath, Snapshot, SnapshotDiff};

pub use diff::line_hunks;
pub use merkle::snapshot_id;

/// Default capture size cap (§6.1).
pub const DEFAULT_CAP: u64 = 50 * 1024 * 1024;

pub struct GixRepo {
    root: PathBuf,
    cap: u64,
    blobs: BlobCache,
}

impl GixRepo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), cap: DEFAULT_CAP, blobs: BlobCache::default() }
    }

    pub fn with_cap(mut self, cap: u64) -> Self {
        self.cap = cap;
        self
    }

    fn open(&self) -> Result<gix::Repository, GitError> {
        gix::open(&self.root).map_err(err)
    }

    fn commit_snapshot_sync(&self, commit: &CommitId) -> Result<Snapshot, GitError> {
        let repo = self.open()?;
        let id = gix::ObjectId::from_hex(commit.0.as_bytes()).map_err(err)?;
        let tree = repo.find_commit(id).map_err(err)?.tree_id().map_err(err)?;
        let index = repo.index_from_tree(&tree).map_err(err)?;
        let mut entries = BTreeMap::new();
        for e in index.entries() {
            let entry = scan::tree_entry(&repo, &self.blobs, e.id, e.mode.bits())?;
            entries.insert(RepoPath(e.path(&index).to_vec()), entry);
        }
        Ok(Snapshot { id: snapshot_id(&entries), entries })
    }

    fn read_blob_sync(&self, path: &RepoPath, blob: BlobId) -> Result<Option<Vec<u8>>, GitError> {
        let repo = self.open()?;
        if let Some(bytes) = self.blobs.odb_content(&repo, blob, u64::MAX)? {
            return Ok(Some(bytes));
        }
        let item = scan::Worktree::new(&repo, &self.blobs, self.cap)?.item_by_path(path.0.as_bstr(), true)?;
        Ok(item.filter(|i| i.entry.blob == blob).and_then(|i| i.bytes))
    }

    fn diff_sync(&self, stored: BTreeMap<BlobId, Vec<u8>>, old: &Snapshot, new: &Snapshot) -> Result<SnapshotDiff, GitError> {
        let repo = self.open()?;
        let mut wt = scan::Worktree::new(&repo, &self.blobs, self.cap)?;
        diff::diff(&repo, &mut wt, &self.blobs, self.cap, stored, old, new)
    }
}

impl GitRepo for GixRepo {
    async fn head(&self) -> Result<CommitId, GitError> {
        Ok(CommitId(self.open()?.head_id().map_err(err)?.to_string()))
    }

    /// Always a full reconciliation: unchanged tracked files cost one stat each
    /// plus a cache lookup.
    async fn scan(&self, _dirty: Dirty) -> Result<Scan, GitError> {
        scan::scan(&self.open()?, &self.blobs, self.cap)
    }

    async fn checkout(&self, _commit: &CommitId, _snapshot: &Snapshot, _root: &Path) -> Result<(), GitError> {
        Err(GitError("checkout is not implemented yet".into()))
    }

    async fn commit_snapshot(&self, commit: &CommitId) -> Result<Snapshot, GitError> {
        self.commit_snapshot_sync(commit)
    }

    async fn read_blob(&self, path: &RepoPath, blob: BlobId) -> Result<Option<Vec<u8>>, GitError> {
        self.read_blob_sync(path, blob)
    }

    async fn diff<S: Store + Sync>(&self, store: &S, old: &Snapshot, new: &Snapshot) -> Result<SnapshotDiff, GitError> {
        let mut stored = BTreeMap::new();
        for (_, blob) in diff::text_blobs(&diff::changes(old, new)) {
            if let Some(bytes) = store.blob(blob).await.map_err(|e| GitError(e.0))? {
                stored.insert(blob, bytes);
            }
        }
        self.diff_sync(stored, old, new)
    }
}

/// Git object id to `BlobId`, so unchanged tracked files and tree entries are
/// hashed once. In memory for now.
#[derive(Default)]
pub(crate) struct BlobCache {
    ids: Mutex<HashMap<gix::ObjectId, BlobId>>,
}

impl BlobCache {
    /// The `BlobId` of the git blob `oid`, reading it from the object database
    /// on a miss.
    pub fn blob_id(&self, repo: &gix::Repository, oid: gix::ObjectId) -> Result<BlobId, GitError> {
        if let Some(id) = self.ids.lock().expect("not poisoned").get(&oid) {
            return Ok(*id);
        }
        let mut buf = Vec::new();
        repo.objects.try_find(&oid, &mut buf).map_err(err)?.ok_or_else(|| GitError(format!("missing object {oid}")))?;
        let id = BlobId::of(&buf);
        self.ids.lock().expect("not poisoned").insert(oid, id);
        Ok(id)
    }

    /// Content of `blob` if git has it and it is within `cap`.
    pub fn odb_content(&self, repo: &gix::Repository, blob: BlobId, cap: u64) -> Result<Option<Vec<u8>>, GitError> {
        let oid = self.ids.lock().expect("not poisoned").iter().find(|(_, b)| **b == blob).map(|(o, _)| *o);
        let Some(oid) = oid else { return Ok(None) };
        let mut buf = Vec::new();
        let found = repo.objects.try_find(&oid, &mut buf).map_err(err)?.is_some();
        Ok((found && buf.len() as u64 <= cap).then_some(buf))
    }
}

fn err(e: impl Display) -> GitError {
    GitError(e.to_string())
}
