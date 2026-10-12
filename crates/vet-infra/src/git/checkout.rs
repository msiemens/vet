//! Writing a snapshot into a directory (§7.2). Content is written as stored:
//! nothing here runs hooks, filters or smudging.

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use vet_app::ports::GitError;
use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath};

use super::{Inner, err};
use crate::paths;

/// An entry with the bytes the content store had for it, if any.
pub(super) type Pending = (RepoPath, FileEntry, Option<Vec<u8>>);

impl Inner {
    /// Writes `batch` under `root`, filling in missing bytes from the object
    /// database. Returns the blobs neither had.
    pub(super) fn write_entries(&self, root: &Path, batch: Vec<Pending>) -> Result<Vec<BlobId>, GitError> {
        let repo = self.open()?;
        let mut missing = Vec::new();
        for (path, entry, stored) in batch {
            let dest = root.join(checked(&path)?);
            if entry.kind == EntryKind::Submodule {
                // Descent is opt-in (§6.1): the pinned commit has no content here.
                std::fs::create_dir_all(&dest).map_err(err)?;
                continue;
            }
            let bytes = match stored {
                Some(bytes) => bytes,
                None => match self.blobs.odb_content(&repo, entry.blob, u64::MAX)? {
                    Some(bytes) => bytes,
                    None => {
                        missing.push(entry.blob);
                        continue;
                    }
                },
            };
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(err)?;
            }
            if entry.kind == EntryKind::Symlink {
                symlink(&bytes, &dest)?;
            } else {
                std::fs::write(&dest, &bytes).map_err(err)?;
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
        return Err(GitError(format!(
            "refusing to write unsafe path {:?}",
            path.to_string_lossy()
        )));
    }
    paths::to_path(&path.0).map_err(err)
}

#[cfg(unix)]
fn symlink(target: &[u8], dest: &Path) -> Result<(), GitError> {
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), dest).map_err(err)
}

/// Windows without symlink rights: the link target as file content.
#[cfg(not(unix))]
fn symlink(target: &[u8], dest: &Path) -> Result<(), GitError> {
    std::fs::write(dest, target).map_err(err)
}

#[cfg(unix)]
fn set_mode(dest: &Path, mode: u32) -> Result<(), GitError> {
    let bits = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(bits)).map_err(err)
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<(), GitError> {
    Ok(())
}
