//! `vet_app::ports::GitRepo` over gitoxide. No git subprocesses, hooks,
//! fsmonitor, or filter and diff drivers (§6.3). All filesystem, hashing and
//! gix work runs on the blocking pool.

mod base;
mod checkout;
mod diff;
mod ignore;
mod scan;
mod stat_cache;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gix::bstr::ByteSlice;
use gix::objs::{Find, FindHeader};
use vet_app::pin::CommitId;
use vet_app::ports::{
    BaseCandidates, BlobRead, BranchInfo, CheckoutError, CommitInfo, Dirty, GitError, GitRepo, IgnoreRules, Scan, Store,
};
use vet_app::review::SessionId;
use vet_core::content::{BlobId, EntryKind, RepoPath, Snapshot, SnapshotDiff};
use vet_core::merkle;

use checkout::Pending;
use stat_cache::StatCache;

pub use diff::line_hunks;

/// Default capture size cap (§6.1).
pub const DEFAULT_CAP: u64 = 50 * 1024 * 1024;

/// The root of the working tree containing `path`, found the way git does and
/// canonicalized. The error is fit to show to a user.
pub fn find_root(path: &Path) -> Result<PathBuf, String> {
    let not_a_repo = || format!("Not a git repository: {}", path.display());
    let repo = gix::discover(path).map_err(|_| not_a_repo())?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| "Bare repositories are not supported".to_string())?;
    workdir.canonicalize().map_err(|_| not_a_repo())
}

/// Cheap to clone: clones share the caches.
#[derive(Clone)]
pub struct GixRepo(Arc<Inner>);

struct Inner {
    root: PathBuf,
    cap: u64,
    blobs: BlobCache,
    stats: StatCache,
}

impl GixRepo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self(Arc::new(Inner {
            root: root.into(),
            cap: DEFAULT_CAP,
            blobs: BlobCache::default(),
            stats: StatCache::default(),
        }))
    }

    pub fn with_cap(self, cap: u64) -> Self {
        Self(Arc::new(Inner {
            root: self.0.root.clone(),
            cap,
            blobs: BlobCache::default(),
            stats: StatCache::default(),
        }))
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Inner) -> Result<T, GitError> + Send + 'static,
    ) -> Result<T, GitError> {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || f(&inner)).await.map_err(err)?
    }

    async fn write_batch(&self, root: &Path, batch: Vec<Pending>) -> Result<Vec<BlobId>, GitError> {
        let root = root.to_owned();
        self.blocking(move |i| i.write_entries(&root, batch)).await
    }
}

impl Inner {
    fn open(&self) -> Result<gix::Repository, GitError> {
        gix::open(&self.root).map_err(err)
    }

    fn commit_snapshot(&self, commit: &CommitId) -> Result<Snapshot, GitError> {
        let repo = self.open()?;
        let id = gix::ObjectId::from_hex(commit.0.as_bytes()).map_err(err)?;
        let tree = repo.find_commit(id).map_err(err)?.tree_id().map_err(err)?;
        let index = repo.index_from_tree(&tree).map_err(err)?;
        let mut entries = BTreeMap::new();
        for e in index.entries() {
            let entry = scan::tree_entry(&repo, &self.blobs, e.id, e.mode.bits())?;
            entries.insert(RepoPath(e.path(&index).to_vec()), entry);
        }
        Ok(Snapshot {
            id: merkle::snapshot_id(&entries),
            entries,
        })
    }

    fn read_blob(&self, path: &RepoPath, blob: BlobId) -> Result<Option<Vec<u8>>, GitError> {
        let repo = self.open()?;
        if let Some(bytes) = self.blobs.odb_content(&repo, blob, u64::MAX)? {
            return Ok(Some(bytes));
        }
        let item =
            scan::Worktree::new(&repo, &self.blobs, &self.stats, self.cap)?.item_by_path(path.0.as_bstr(), true)?;
        Ok(item.filter(|i| i.entry.blob == blob).and_then(|i| i.bytes))
    }

    /// The object's size is checked before its content is read.
    fn read_blob_capped(&self, path: &RepoPath, blob: BlobId) -> Result<BlobRead, GitError> {
        let repo = self.open()?;
        if self.blobs.odb_size(&repo, blob)?.is_some_and(|size| size > self.cap) {
            return Ok(BlobRead::TooLarge);
        }
        // A blob not in git is read from the worktree, which applies the cap itself.
        Ok(self.read_blob(path, blob)?.map_or(BlobRead::Missing, BlobRead::Found))
    }

    fn diff(
        &self,
        stored: BTreeMap<BlobId, Vec<u8>>,
        old: &Snapshot,
        new: &Snapshot,
    ) -> Result<SnapshotDiff, GitError> {
        let repo = self.open()?;
        let mut wt = scan::Worktree::new(&repo, &self.blobs, &self.stats, self.cap)?;
        diff::diff(&repo, &mut wt, &self.blobs, self.cap, stored, old, new)
    }
}

impl GitRepo for GixRepo {
    async fn head(&self) -> Result<CommitId, GitError> {
        self.blocking(|i| Ok(CommitId(i.open()?.head_id().map_err(err)?.to_string())))
            .await
    }

    async fn user_name(&self) -> Option<String> {
        self.blocking(|i| {
            let repo = i.open()?;
            let name = repo.config_snapshot().string("user.name");
            Ok(name.map(|n| n.to_string()))
        })
        .await
        .ok()
        .flatten()
    }

    /// Always a full reconciliation, but only entries whose stat changed are
    /// read and hashed.
    async fn scan(&self, _dirty: Dirty) -> Result<Scan, GitError> {
        self.blocking(|i| scan::scan(&i.open()?, &i.blobs, &i.stats, i.cap))
            .await
    }

    /// Content-store bytes are read here, a bounded batch at a time; the rest
    /// comes from the object database on the blocking pool.
    async fn checkout<S: Store + Sync>(
        &self,
        store: &S,
        commit: &CommitId,
        snapshot: &Snapshot,
        root: &Path,
    ) -> Result<(), CheckoutError> {
        const BATCH_FILES: usize = 256;
        const BATCH_BYTES: usize = 64 * 1024 * 1024;

        // Learns which git blob holds each of the commit's contents.
        let commit = commit.clone();
        self.blocking(move |i| i.commit_snapshot(&commit).map(drop)).await?;

        let mut missing = Vec::new();
        let mut batch: Vec<Pending> = Vec::new();
        let mut bytes = 0;
        for (path, entry) in &snapshot.entries {
            let stored = if entry.kind == EntryKind::Submodule {
                None
            } else {
                store.blob(entry.blob).await.map_err(|e| GitError(e.0))?
            };
            bytes += stored.as_ref().map_or(0, Vec::len);
            batch.push((path.clone(), entry.clone(), stored));
            if batch.len() >= BATCH_FILES || bytes >= BATCH_BYTES {
                missing.extend(self.write_batch(root, std::mem::take(&mut batch)).await?);
                bytes = 0;
            }
        }
        missing.extend(self.write_batch(root, batch).await?);
        if missing.is_empty() {
            Ok(())
        } else {
            Err(CheckoutError::MissingBlobs(missing))
        }
    }

    async fn commit_snapshot(&self, commit: &CommitId) -> Result<Snapshot, GitError> {
        let commit = commit.clone();
        self.blocking(move |i| i.commit_snapshot(&commit)).await
    }

    async fn read_blob(&self, path: &RepoPath, blob: BlobId) -> Result<Option<Vec<u8>>, GitError> {
        let path = path.clone();
        self.blocking(move |i| i.read_blob(&path, blob)).await
    }

    async fn read_blob_capped(&self, path: &RepoPath, blob: BlobId) -> Result<BlobRead, GitError> {
        let path = path.clone();
        self.blocking(move |i| i.read_blob_capped(&path, blob)).await
    }

    async fn submodule_commit(&self, blob: BlobId) -> Result<Option<String>, GitError> {
        Ok(self.0.blobs.submodule_commit(blob))
    }

    async fn merge_base(&self, reference: &str) -> Result<CommitId, GitError> {
        let reference = reference.to_owned();
        self.blocking(move |i| base::merge_base(&i.open()?, &reference)).await
    }

    async fn resolve_commit(&self, spec: &str) -> Result<CommitId, GitError> {
        let spec = spec.to_owned();
        self.blocking(move |i| base::resolve_commit(&i.open()?, &spec)).await
    }

    async fn base_candidates(&self, max_commits: usize) -> Result<BaseCandidates, GitError> {
        self.blocking(move |i| base::candidates(&i.open()?, max_commits)).await
    }

    async fn branches(&self) -> Result<Vec<BranchInfo>, GitError> {
        self.blocking(|i| base::branches(&i.open()?)).await
    }

    async fn commit_info(&self, commit: &CommitId) -> Result<CommitInfo, GitError> {
        let commit = commit.clone();
        self.blocking(move |i| base::commit_info(&i.open()?, &commit)).await
    }

    async fn first_parent_distance(
        &self,
        tip: &CommitId,
        commit: &CommitId,
        max: usize,
    ) -> Result<Option<usize>, GitError> {
        let (tip, commit) = (tip.clone(), commit.clone());
        self.blocking(move |i| base::first_parent_distance(&i.open()?, &tip, &commit, max))
            .await
    }

    async fn commits_behind(&self, tip: &CommitId, base: &CommitId, max: usize) -> Result<usize, GitError> {
        let (tip, base) = (tip.clone(), base.clone());
        self.blocking(move |i| base::commits_behind(&i.open()?, &tip, &base, max))
            .await
    }

    async fn set_session_ref(&self, session: SessionId, name: &str, commit: &CommitId) -> Result<(), GitError> {
        let (name, commit) = (format!("{}{name}", session_ref_prefix(session)), commit.clone());
        self.blocking(move |i| {
            let id = gix::ObjectId::from_hex(commit.0.as_bytes()).map_err(err)?;
            i.open()?
                .reference(name, id, gix::refs::transaction::PreviousValue::Any, "vet session")
                .map_err(err)?;
            Ok(())
        })
        .await
    }

    async fn delete_session_refs(&self, session: SessionId) -> Result<(), GitError> {
        self.blocking(move |i| {
            let repo = i.open()?;
            let platform = repo.references().map_err(err)?;
            let prefix = session_ref_prefix(session);
            for reference in platform.prefixed(prefix.as_str()).map_err(err)? {
                reference.map_err(err)?.delete().map_err(err)?;
            }
            Ok(())
        })
        .await
    }

    async fn diff<S: Store + Sync>(&self, store: &S, old: &Snapshot, new: &Snapshot) -> Result<SnapshotDiff, GitError> {
        let mut stored = BTreeMap::new();
        for (_, blob) in diff::text_blobs(&diff::changes(old, new)) {
            if let Some(bytes) = store.blob(blob).await.map_err(|e| GitError(e.0))? {
                stored.insert(blob, bytes);
            }
        }
        let (old, new) = (old.clone(), new.clone());
        self.blocking(move |i| i.diff(stored, &old, &new)).await
    }
}

impl IgnoreRules for GixRepo {
    async fn unignored(&self, paths: Vec<RepoPath>) -> Result<Vec<RepoPath>, GitError> {
        self.blocking(move |i| ignore::unignored(&i.open()?, paths)).await
    }
}

/// Git object id to `BlobId` and back, so unchanged tracked files and tree
/// entries are hashed once. In memory for now.
#[derive(Default)]
pub(crate) struct BlobCache {
    ids: Mutex<Ids>,
}

#[derive(Default)]
struct Ids {
    by_oid: HashMap<gix::ObjectId, BlobId>,
    by_blob: HashMap<BlobId, gix::ObjectId>,
    /// Pinned commit of each submodule entry's `BlobId`, which only hashes it.
    submodules: HashMap<BlobId, gix::ObjectId>,
}

impl BlobCache {
    /// The `BlobId` of a submodule entry pinned at `commit`.
    pub fn submodule_id(&self, commit: gix::ObjectId) -> BlobId {
        let id = BlobId::of(commit.as_bytes());
        self.ids.lock().expect("not poisoned").submodules.insert(id, commit);
        id
    }

    pub fn submodule_commit(&self, blob: BlobId) -> Option<String> {
        let ids = self.ids.lock().expect("not poisoned");
        ids.submodules.get(&blob).map(|c| c.to_string())
    }

    /// The `BlobId` of the git blob `oid`, reading it from the object database
    /// on a miss.
    pub fn blob_id(&self, repo: &gix::Repository, oid: gix::ObjectId) -> Result<BlobId, GitError> {
        if let Some(id) = self.ids.lock().expect("not poisoned").by_oid.get(&oid) {
            return Ok(*id);
        }
        let mut buf = Vec::new();
        repo.objects
            .try_find(&oid, &mut buf)
            .map_err(err)?
            .ok_or_else(|| GitError(format!("missing object {oid}")))?;
        let id = BlobId::of(&buf);
        let mut ids = self.ids.lock().expect("not poisoned");
        ids.by_oid.insert(oid, id);
        ids.by_blob.insert(id, oid);
        Ok(id)
    }

    /// Size of `blob` if git has it, from the object header.
    pub fn odb_size(&self, repo: &gix::Repository, blob: BlobId) -> Result<Option<u64>, GitError> {
        let oid = self.ids.lock().expect("not poisoned").by_blob.get(&blob).copied();
        let Some(oid) = oid else { return Ok(None) };
        Ok(repo.objects.try_header(&oid).map_err(err)?.map(|h| h.size))
    }

    /// Content of `blob` if git has it and it is within `cap`.
    pub fn odb_content(&self, repo: &gix::Repository, blob: BlobId, cap: u64) -> Result<Option<Vec<u8>>, GitError> {
        let oid = self.ids.lock().expect("not poisoned").by_blob.get(&blob).copied();
        let Some(oid) = oid else { return Ok(None) };
        let mut buf = Vec::new();
        let found = repo.objects.try_find(&oid, &mut buf).map_err(err)?.is_some();
        Ok((found && buf.len() as u64 <= cap).then_some(buf))
    }
}

fn session_ref_prefix(session: SessionId) -> String {
    format!("refs/vet/sessions/{}/", session.0)
}

fn err(e: impl Display) -> GitError {
    GitError(e.to_string())
}

/// Folders whose name a tool defines as its own store; their untracked contents are never
/// part of a snapshot (SPEC §6.1).
const TOOL_STORES: &[&[u8]] = &[b".pnpm-store"];

/// Whether `path` (repository-relative, `/`-separated) is a tool store or inside one.
fn in_tool_store(path: &[u8]) -> bool {
    path.split(|&b| b == b'/').any(|part| TOOL_STORES.contains(&part))
}
