//! Writing a snapshot into a directory (§6.2). Content is written as stored:
//! nothing here runs hooks, filters or smudging.

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use vet_app::ports::{GitError, StoredBlob};
use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath};

use super::{Inner, chained};
use crate::paths;

/// An entry with what the content store had for it, if anything.
pub(super) type Pending = (RepoPath, FileEntry, Option<StoredBlob>);

impl Inner {
    /// Writes `batch` under `root`, filling in missing bytes from the object database. A file the
    /// store holds by hash only is read from the worktree too, if it still hashes to the entry's
    /// blob. Returns the blobs none of them had.
    pub(super) fn write_entries(&self, root: &Path, batch: Vec<Pending>) -> Result<Vec<BlobId>, GitError> {
        let repo = self.open()?;
        let mut missing = Vec::new();
        for (path, entry, stored) in batch {
            let dest = root.join(checked(&path)?);
            if entry.kind == EntryKind::Submodule {
                // Descent is opt-in (§4.1): the pinned commit has no content here.
                std::fs::create_dir_all(&dest).map_err(chained)?;
                continue;
            }
            let found = match stored {
                Some(StoredBlob::Bytes(bytes)) => Some(bytes),
                Some(StoredBlob::Binary { .. }) => self.read_blob(&repo, &path, entry.blob)?,
                None => self.blobs.odb_content(&repo, entry.blob, u64::MAX)?,
            };
            let Some(bytes) = found else {
                missing.push(entry.blob);
                continue;
            };
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(chained)?;
            }
            if entry.kind == EntryKind::Symlink {
                symlink(&bytes, &dest)?;
            } else {
                std::fs::write(&dest, &bytes).map_err(chained)?;
                set_mode(&dest, entry.mode)?;
            }
        }
        Ok(missing)
    }
}

/// The path as a relative filesystem path, refusing anything that could leave
/// the root or write into `.git`.
fn checked(path: &RepoPath) -> Result<&Path, GitError> {
    let bad = path
        .0
        .split(|b| *b == b'/')
        .any(|part| part.is_empty() || part == b"." || part == b".." || part.eq_ignore_ascii_case(b".git"));
    if bad {
        return Err(GitError::failed(format!(
            "refusing to write unsafe path {:?}",
            path.to_string_lossy()
        )));
    }
    paths::to_path(&path.0).map_err(chained)
}

#[cfg(unix)]
fn symlink(target: &[u8], dest: &Path) -> Result<(), GitError> {
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), dest).map_err(chained)
}

/// Windows without symlink rights: the link target as file content.
#[cfg(not(unix))]
fn symlink(target: &[u8], dest: &Path) -> Result<(), GitError> {
    std::fs::write(dest, target).map_err(chained)
}

#[cfg(unix)]
fn set_mode(dest: &Path, mode: u32) -> Result<(), GitError> {
    let bits = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(bits)).map_err(chained)
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<(), GitError> {
    Ok(())
}

#[cfg(test)]
mod tests;
