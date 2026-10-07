//! Snapshots of what git recorded: the tree of a commit, and entries as a tree
//! or the index holds them.

use std::collections::BTreeMap;

use vet_app::changeset::CommitId;
use vet_app::ports::GitError;
use vet_core::content::{EntryKind, FileEntry, RepoPath, Snapshot};
use vet_core::merkle;

use super::{BlobCache, Inner, err, object_id};

impl Inner {
    pub(super) fn commit_snapshot(&self, commit: &CommitId) -> Result<Snapshot, GitError> {
        let repo = self.open()?;
        let tree = repo
            .find_commit(object_id(commit)?)
            .map_err(err)?
            .tree_id()
            .map_err(err)?;
        let index = repo.index_from_tree(&tree).map_err(err)?;
        let mut entries = BTreeMap::new();
        for e in index.entries() {
            let entry = tree_entry(&repo, &self.blobs, e.id, e.mode.bits())?;
            entries.insert(RepoPath(e.path(&index).to_vec()), entry);
        }
        Ok(Snapshot {
            id: merkle::snapshot_id(&entries),
            entries,
        })
    }
}

/// The entry for `oid` as recorded in a tree or index, with no worktree
/// involved. A submodule's blob is the hash of its pinned commit id.
pub(crate) fn tree_entry(
    repo: &gix::Repository,
    blobs: &BlobCache,
    oid: gix::ObjectId,
    mode: u32,
) -> Result<FileEntry, GitError> {
    let (kind, blob) = match mode & 0o170000 {
        0o160000 => (EntryKind::Submodule, blobs.submodule_id(oid)),
        0o120000 => (EntryKind::Symlink, blobs.blob_id(repo, oid)?),
        _ => (EntryKind::File, blobs.blob_id(repo, oid)?),
    };
    Ok(FileEntry {
        blob,
        mode,
        kind,
        unfiltered: false,
    })
}
