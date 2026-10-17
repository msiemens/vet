//! `vet_app::ports::GitRepo` over gitoxide. No git subprocesses, hooks,
//! fsmonitor, or filter and diff drivers (§4.3); a fetch runs the credential
//! helpers the user configured and, over SSH, the system `ssh`. All filesystem,
//! hashing and gix work runs on the blocking pool.

mod attributes;
mod base;
mod blob_cache;
mod checkout;
mod diff;
mod fetch;
mod ignore;
mod remerge;
mod scan;
mod stat_cache;
mod tree;
mod vet_refs;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gix::bstr::ByteSlice;
use vet_app::changeset::{ChangesetId, CommitId};
use vet_app::ports::{
    Attributes, BlobJob, BlobRead, BlobReader, BlobSession, BranchInfo, ChangesetRef, ChangesetRefs, Checkout,
    CheckoutError, CommitDetail, CommitInfo, Differ, Dirty, Fetcher, GitError, History, IgnoreRules, RefTip, RefUpdate,
    Remerged, RemoteInfo, Scan, Scanner, Store, StoredBlob, TextDiff, Via, split_stored,
};
use vet_core::classify::{changes, text_blobs};
use vet_core::content::{BlobId, EntryKind, RepoPath, Snapshot};

use crate::budget::{Budget, REVIEW_IO};
use crate::paths;
use checkout::Pending;
use stat_cache::StatCache;

pub(crate) use blob_cache::BlobCache;
pub use vet_refs::RepoRefs;

/// Default capture size cap (§4.1).
pub const DEFAULT_CAP: u64 = 50 * 1024 * 1024;

/// The root of the working tree containing `path`, found the way git does and
/// canonicalized (in the plain form on Windows wherever that names it). The
/// error is fit to show to a user.
pub fn find_root(path: &Path) -> Result<PathBuf, String> {
    let not_a_repo = || format!("Not a git repository: {}", path.display());
    let repo = gix::discover(path).map_err(|_| not_a_repo())?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| "Bare repositories are not supported".to_string())?;
    paths::canonical(workdir).map_err(|_| not_a_repo())
}

/// The files of the tree at `root`, relative to it, without reading any: in a
/// worktree what git sees (the scan's candidates, so ignored files are left
/// out); in a directory that is not one, every file, since it holds nothing but
/// its own. A name the platform cannot represent is left out.
/// The work runs on `io`.
pub async fn list_files(io: &Budget, root: &Path) -> Result<Vec<PathBuf>, String> {
    let root = root.to_owned();
    // `.git` is how git marks a worktree's root, as a directory or a file.
    let is_worktree = {
        let root = root.clone();
        io.blocking(move || root.join(".git").exists())
            .await
            .map_err(|e| e.to_string())?
    };
    if is_worktree {
        let files = GixRepo::new(&root, io.clone())
            .list_files()
            .await
            .map_err(|e| e.to_string())?;
        return Ok(files
            .iter()
            .filter_map(|f| paths::to_path(&f.0).ok())
            .map(Path::to_owned)
            .collect());
    }
    io.blocking(move || walk(&root)).await.map_err(|e| e.to_string())?
}

fn walk(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                found.push(relative.to_owned());
            }
        }
    }
    Ok(found)
}

/// Cheap to clone: clones share the caches. Its blocking work draws on a budget; clones
/// made with [`GixRepo::budgeted`] draw on another.
#[derive(Clone)]
pub struct GixRepo(Arc<Inner>, Budget);

struct Inner {
    root: PathBuf,
    cap: u64,
    blobs: BlobCache,
    stats: StatCache,
    /// Opened on the first read of a blob.
    opened: OnceLock<gix::ThreadSafeRepository>,
    /// Whether the user's and the system's git configuration and the environment are ignored.
    isolated: bool,
    /// Whether a platform fetch is served from the directory `vet.servedPlatforms` names.
    served: bool,
}

/// Who a reflog entry is written as when no git identity is configured. Git itself guesses
/// `user@host` there; a fixed one keeps the entries alike on every machine.
const FALLBACK_NAME: &str = "Vet";
const FALLBACK_EMAIL: &str = "vet@localhost";

impl Inner {
    /// A repository with caches of its own.
    fn new(root: PathBuf, cap: u64, isolated: bool, served: bool) -> Self {
        Self {
            root,
            cap,
            blobs: BlobCache::default(),
            stats: StatCache::default(),
            opened: OnceLock::new(),
            isolated,
            served,
        }
    }
}

impl GixRepo {
    /// A repository whose blocking work draws on `io`. The budget is required, so a caller
    /// cannot end up on a private pool of its own by forgetting [`GixRepo::budgeted`].
    pub fn new(root: impl Into<PathBuf>, io: Budget) -> Self {
        Self(Arc::new(Inner::new(root.into(), DEFAULT_CAP, false, false)), io)
    }

    /// A repository with a budget of its own, for tests and one-off tools.
    #[doc(hidden)]
    pub fn for_test(root: impl Into<PathBuf>) -> Self {
        Self::new(root, Budget::new(REVIEW_IO))
    }

    pub fn with_cap(self, cap: u64) -> Self {
        let inner = Inner::new(self.0.root.clone(), cap, self.0.isolated, self.0.served);
        Self(Arc::new(inner), self.1)
    }

    /// The same repository read without the user's and the system's git configuration and
    /// without git's environment variables: only the repository's own configuration counts.
    pub fn isolated(self) -> Self {
        let inner = Inner::new(self.0.root.clone(), self.0.cap, true, self.0.served);
        Self(Arc::new(inner), self.1)
    }

    /// For tests: a platform fetch is served from the directory the repository's
    /// `vet.servedPlatforms` names.
    pub fn serving_platforms(self) -> Self {
        let inner = Inner::new(self.0.root.clone(), self.0.cap, self.0.isolated, true);
        Self(Arc::new(inner), self.1)
    }

    /// The same repository, with its caches, drawing on `io`.
    pub fn budgeted(&self, io: Budget) -> Self {
        Self(self.0.clone(), io)
    }

    /// Every file of the tree as git sees it, read from none: the index plus
    /// what is untracked and not ignored.
    pub async fn list_files(&self) -> Result<Vec<RepoPath>, GitError> {
        self.blocking(|i| scan::list(&i.open()?)).await
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Inner) -> Result<T, GitError> + Send + 'static,
    ) -> Result<T, GitError> {
        let inner = self.0.clone();
        self.1.blocking(move || f(&inner)).await.map_err(chained)?
    }

    async fn write_batch(&self, root: &Path, batch: Vec<Pending>) -> Result<Vec<BlobId>, GitError> {
        let root = root.to_owned();
        self.blocking(move |i| i.write_entries(&root, batch)).await
    }
}

impl Inner {
    fn open(&self) -> Result<gix::Repository, GitError> {
        let mut repo = if self.isolated {
            gix::open_opts(&self.root, gix::open::Options::isolated())
        } else {
            gix::open(&self.root)
        }
        .map_err(chained)?;
        // Ref updates write reflog entries, which need a committer; this only fills in a missing one.
        repo.committer_or_set_fallback(FALLBACK_NAME, FALLBACK_EMAIL)
            .map_err(chained)?;
        Ok(repo)
    }

    /// A handle on the repository opened once and kept for the reads of blobs, which
    /// come by the hundreds when the changed symbols are found.
    fn open_shared(&self) -> Result<gix::Repository, GitError> {
        if let Some(opened) = self.opened.get() {
            return Ok(opened.to_thread_local());
        }
        let opened = gix::ThreadSafeRepository::open(&self.root).map_err(chained)?;
        Ok(self.opened.get_or_init(|| opened).to_thread_local())
    }

    fn read_blob(&self, repo: &gix::Repository, path: &RepoPath, blob: BlobId) -> Result<Option<Vec<u8>>, GitError> {
        if let Some(bytes) = self.blobs.odb_content(repo, blob, u64::MAX)? {
            return Ok(Some(bytes));
        }
        let item =
            scan::Worktree::new(repo, &self.blobs, &self.stats, self.cap)?.item_by_path(path.0.as_bstr(), true)?;
        Ok(item.filter(|i| i.entry.blob == blob).and_then(|i| i.bytes))
    }

    /// The object's size is checked before its content is read.
    fn read_blob_capped(&self, repo: &gix::Repository, path: &RepoPath, blob: BlobId) -> Result<BlobRead, GitError> {
        if self.blobs.odb_size(repo, blob)?.is_some_and(|size| size > self.cap) {
            return Ok(BlobRead::TooLarge);
        }
        // A blob not in git is read from the worktree, which applies the cap itself.
        Ok(self
            .read_blob(repo, path, blob)?
            .map_or(BlobRead::Missing, BlobRead::Found))
    }

    fn diff(
        &self,
        stored: BTreeMap<BlobId, Vec<u8>>,
        binary: &BTreeSet<BlobId>,
        old: &Snapshot,
        new: &Snapshot,
    ) -> Result<TextDiff, GitError> {
        let repo = self.open()?;
        let mut wt = scan::Worktree::new(&repo, &self.blobs, &self.stats, self.cap)?;
        diff::diff(&repo, &mut wt, &self.blobs, self.cap, stored, binary, old, new)
    }
}

impl Scanner for GixRepo {
    /// Always a full reconciliation, but only entries whose stat changed are
    /// read and hashed.
    async fn scan(&self, _dirty: Dirty) -> Result<Scan, GitError> {
        self.blocking(|i| scan::scan(&i.open()?, &i.blobs, &i.stats, i.cap))
            .await
    }
}

impl Checkout for GixRepo {
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
                store.blob(entry.blob).await.map_err(|e| GitError::failed(e.0))?
            };
            if let Some(StoredBlob::Bytes(held)) = &stored {
                bytes += held.len();
            }
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
}

impl BlobReader for GixRepo {
    async fn read_blob(&self, path: &RepoPath, blob: BlobId) -> Result<Option<Vec<u8>>, GitError> {
        let path = path.clone();
        self.blocking(move |i| i.read_blob(&i.open_shared()?, &path, blob))
            .await
    }

    async fn read_blob_capped(&self, path: &RepoPath, blob: BlobId) -> Result<BlobRead, GitError> {
        let path = path.clone();
        self.blocking(move |i| i.read_blob_capped(&i.open_shared()?, &path, blob))
            .await
    }

    async fn submodule_commit(&self, blob: BlobId) -> Result<Option<String>, GitError> {
        Ok(self.0.blobs.submodule_commit(blob))
    }

    async fn with_blobs<J: BlobJob>(&self, job: J) -> Result<J::Output, GitError> {
        self.blocking(move |i| {
            let mut session = Session {
                inner: i,
                repo: i.open_shared()?,
            };
            Ok(job.run(&mut session))
        })
        .await
    }
}

/// One open repository for many reads.
struct Session<'a> {
    inner: &'a Inner,
    repo: gix::Repository,
}

impl BlobSession for Session<'_> {
    fn read_blob_capped(&mut self, path: &RepoPath, blob: BlobId) -> Result<BlobRead, GitError> {
        self.inner.read_blob_capped(&self.repo, path, blob)
    }
}

impl History for GixRepo {
    async fn head(&self) -> Result<CommitId, GitError> {
        self.blocking(|i| Ok(CommitId(i.open()?.head_id().map_err(chained)?.to_string())))
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

    async fn commit_snapshot(&self, commit: &CommitId) -> Result<Snapshot, GitError> {
        let commit = commit.clone();
        self.blocking(move |i| i.commit_snapshot(&commit)).await
    }

    async fn merge_base(&self, reference: &str, tip: &CommitId) -> Result<CommitId, GitError> {
        let (reference, tip) = (reference.to_owned(), tip.clone());
        self.blocking(move |i| base::merge_base(&i.open()?, &reference, &tip))
            .await
    }

    async fn default_reference(&self) -> Result<Option<String>, GitError> {
        self.blocking(|i| base::default_reference(&i.open()?)).await
    }

    async fn default_branch(&self) -> Result<Option<String>, GitError> {
        self.blocking(|i| base::default_branch(&i.open()?)).await
    }

    async fn first_parent_range(&self, base: &CommitId, tip: &CommitId) -> Result<Vec<CommitId>, GitError> {
        let (base, tip) = (base.clone(), tip.clone());
        self.blocking(move |i| base::first_parent_range(&i.open()?, &base, &tip))
            .await
    }

    async fn resolve_commit(&self, spec: &str) -> Result<Option<CommitId>, GitError> {
        let spec = spec.to_owned();
        self.blocking(move |i| base::resolve_commit(&i.open()?, &spec)).await
    }

    async fn branch_tip(&self, name: &str) -> Result<Option<CommitId>, GitError> {
        let name = name.to_owned();
        self.blocking(move |i| base::branch_tip(&i.open()?, &name)).await
    }

    async fn branches(&self) -> Result<Vec<BranchInfo>, GitError> {
        self.blocking(|i| base::branches(&i.open()?)).await
    }

    async fn current_branch(&self) -> Result<Option<String>, GitError> {
        self.blocking(|i| base::current_branch(&i.open()?)).await
    }

    async fn commit_info(&self, commit: &CommitId) -> Result<CommitInfo, GitError> {
        let commit = commit.clone();
        self.blocking(move |i| base::commit_info(&i.open()?, &commit)).await
    }

    async fn commit_detail(&self, commit: &CommitId) -> Result<CommitDetail, GitError> {
        let commit = commit.clone();
        self.blocking(move |i| i.commit_detail(&commit)).await
    }

    async fn remerge(&self, merge: &CommitId) -> Result<Option<Remerged>, GitError> {
        let merge = merge.clone();
        self.blocking(move |i| i.remerge(&merge)).await
    }

    async fn commits_behind(&self, tip: &CommitId, base: &CommitId, max: usize) -> Result<usize, GitError> {
        let (tip, base) = (tip.clone(), base.clone());
        self.blocking(move |i| base::commits_behind(&i.open()?, &tip, &base, max))
            .await
    }
}

impl Fetcher for GixRepo {
    async fn remotes(&self) -> Result<Vec<RemoteInfo>, GitError> {
        self.blocking(|i| fetch::remotes(&i.open()?)).await
    }

    async fn fetch(&self, remote: &str, via: Via) -> Result<usize, GitError> {
        let remote = remote.to_owned();
        self.blocking(move |i| fetch::fetch(i.open()?, &remote, via, i.served))
            .await
    }

    async fn fetch_refs(&self, remote: &str, updates: Vec<RefUpdate>, via: Via) -> Result<(), GitError> {
        let remote = remote.to_owned();
        self.blocking(move |i| fetch::fetch_refs(i.open()?, &remote, updates, via, i.served))
            .await
    }

    async fn refs_below(&self, prefix: &str) -> Result<Vec<RefTip>, GitError> {
        let prefix = prefix.to_owned();
        self.blocking(move |i| fetch::refs_below(&i.open()?, &prefix)).await
    }

    async fn descends_from(&self, descendant: &str, ancestor: &str) -> Result<bool, GitError> {
        let (descendant, ancestor) = (descendant.to_owned(), ancestor.to_owned());
        self.blocking(move |i| fetch::descends_from(&i.open()?, &descendant, &ancestor))
            .await
    }

    async fn delete_refs(&self, names: Vec<String>) -> Result<(), GitError> {
        self.blocking(move |i| fetch::delete_refs(&i.open()?, &names)).await
    }
}

impl ChangesetRefs for GixRepo {
    async fn set_changeset_ref(
        &self,
        changeset: ChangesetId,
        version: u32,
        name: &str,
        commit: &CommitId,
    ) -> Result<(), GitError> {
        let name = format!("{}{version}/{name}", changeset_ref_prefix(changeset));
        let commit = commit.clone();
        self.blocking(move |i| {
            let id = object_id(&commit)?;
            i.open()?
                .reference(name, id, gix::refs::transaction::PreviousValue::Any, "vet changeset")
                .map_err(chained)?;
            Ok(())
        })
        .await
    }

    async fn changeset_refs(&self, changeset: ChangesetId) -> Result<Vec<ChangesetRef>, GitError> {
        let prefix = changeset_ref_prefix(changeset);
        let wanted = prefix.clone();
        let refs = self.blocking(move |i| fetch::refs_below(&i.open()?, &wanted)).await?;
        Ok(refs
            .into_iter()
            .filter_map(|r| {
                let (version, name) = r.name.strip_prefix(&prefix)?.split_once('/')?;
                Some(ChangesetRef {
                    version: version.parse().ok()?,
                    name: name.to_owned(),
                    commit: CommitId(r.commit),
                })
            })
            .collect())
    }

    async fn delete_changeset_refs(&self, changeset: ChangesetId) -> Result<(), GitError> {
        self.blocking(move |i| {
            let repo = i.open()?;
            let platform = repo.references().map_err(chained)?;
            let prefix = changeset_ref_prefix(changeset);
            for reference in platform.prefixed(prefix.as_str()).map_err(chained)? {
                reference.map_err(chained)?.delete().map_err(chained)?;
            }
            Ok(())
        })
        .await
    }
}

impl Attributes for GixRepo {
    async fn linguist_generated<S: Store + Sync>(
        &self,
        store: &S,
        snapshot: &Snapshot,
        paths: Vec<RepoPath>,
    ) -> Result<BTreeMap<RepoPath, bool>, GitError> {
        let files = attributes::files(snapshot);
        if files.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut stored = store
            .blobs(files.iter().map(|(_, blob)| *blob).collect())
            .await
            .map_err(|e| GitError::failed(e.0))?;
        let mut contents = Vec::new();
        for (path, blob) in files {
            let bytes = match stored.remove(&blob) {
                Some(StoredBlob::Bytes(bytes)) => Some(bytes),
                Some(StoredBlob::Binary { .. }) | None => self.read_blob(&path, blob).await?,
            };
            contents.extend(bytes.map(|bytes| (path, bytes)));
        }
        Ok(attributes::linguist_generated(&contents, paths))
    }
}

impl Differ for GixRepo {
    async fn diff<S: Store + Sync>(&self, store: &S, old: &Snapshot, new: &Snapshot) -> Result<TextDiff, GitError> {
        let wanted = text_blobs(&changes(old, new))
            .into_iter()
            .map(|(_, blob)| blob)
            .collect();
        let (stored, binary) = split_stored(store.blobs(wanted).await.map_err(|e| GitError::failed(e.0))?);
        let (old, new) = (old.clone(), new.clone());
        self.blocking(move |i| i.diff(stored, &binary, &old, &new)).await
    }
}

impl IgnoreRules for GixRepo {
    async fn unignored(&self, paths: Vec<RepoPath>) -> Result<Vec<RepoPath>, GitError> {
        self.blocking(move |i| ignore::unignored(&i.open()?, paths)).await
    }
}

fn changeset_ref_prefix(changeset: ChangesetId) -> String {
    format!("refs/vet/changesets/{}/", changeset.0)
}

/// A git error from a value that says only what failed, as a plain message.
fn err(message: impl Display) -> GitError {
    GitError::failed(vet_log::redact_urls(&message.to_string()).into_owned())
}

/// A git error from an error with a cause chain, so the innermost cause is kept: an
/// outer frame often names only the operation while the innermost holds what the OS said.
fn chained(error: impl std::error::Error + 'static) -> GitError {
    GitError::failed(redact(&error))
}

/// The text of `error` and of every cause below it, with URLs redacted.
pub(super) fn redact(error: &(dyn std::error::Error + 'static)) -> String {
    vet_log::redact_urls(&chain_text(error)).into_owned()
}

/// The text of `error` followed by the text of each cause, `: ` between them.
pub(super) fn chain_text(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(error) = cause {
        let part = error.to_string();
        if !part.is_empty() && !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        cause = error.source();
    }
    text
}

fn object_id(commit: &CommitId) -> Result<gix::ObjectId, GitError> {
    gix::ObjectId::from_hex(commit.0.as_bytes()).map_err(err)
}
