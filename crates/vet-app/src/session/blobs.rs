//! Reading the content a session shows: texts, and what a card says of a
//! non-text file.

use std::collections::BTreeMap;

use vet_core::content::{BlobId, EntryKind, FileEntry, NonTextKind, RepoPath, decode_text};

use super::views::EntryInfo;
use super::{LiveSession, Result};
use crate::ports::{BlobRead, GitRepo, LiveStore};

/// Why a file has no text to show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoText {
    Binary,
    /// Over the capture cap (also for git-tracked content), or not readable any more.
    Unavailable,
    /// A symlink or a submodule.
    NotAFile,
}

/// The media type of a common image format, from the file extension (any case).
pub(super) fn image_media_type(path: &RepoPath) -> Option<&'static str> {
    Some(match path.extension()?.to_ascii_lowercase().as_slice() {
        b"png" => "image/png",
        b"jpg" | b"jpeg" => "image/jpeg",
        b"gif" => "image/gif",
        b"webp" => "image/webp",
        b"bmp" => "image/bmp",
        b"ico" => "image/x-icon",
        b"svg" => "image/svg+xml",
        _ => return None,
    })
}

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// Unavailable content leaves `target` and `size` empty.
    pub(super) async fn describe(
        &self,
        path: &RepoPath,
        entry: Option<&FileEntry>,
        kind: NonTextKind,
    ) -> Option<EntryInfo> {
        let entry = entry?;
        if entry.kind == EntryKind::Submodule {
            return Some(EntryInfo {
                entry: entry.clone(),
                target: self.acquisition.git.submodule_commit(entry.blob).await.ok().flatten(),
                size: None,
            });
        }
        let wanted = match entry.kind {
            EntryKind::Symlink => true,
            EntryKind::File => matches!(
                kind,
                NonTextKind::Binary | NonTextKind::Mode | NonTextKind::Rename | NonTextKind::Empty
            ),
            EntryKind::Submodule => false,
        };
        let mut blobs = BTreeMap::new();
        let loaded = wanted && self.load(&mut blobs, path, entry.blob).await.unwrap_or(false);
        let bytes = blobs.get(&entry.blob).filter(|_| loaded);
        Some(EntryInfo {
            entry: entry.clone(),
            target: bytes
                .filter(|_| entry.kind == EntryKind::Symlink)
                .map(|b| String::from_utf8_lossy(b).into_owned()),
            size: bytes.filter(|_| entry.kind == EntryKind::File).map(|b| b.len() as u64),
        })
    }

    /// Content from the store, else from git or the worktree. `false` if unavailable.
    pub(super) async fn load(
        &self,
        blobs: &mut BTreeMap<BlobId, Vec<u8>>,
        path: &RepoPath,
        blob: BlobId,
    ) -> Result<bool> {
        self.fetch(blobs, path, blob, false).await
    }

    /// Like `load`; with `capped`, git content over the capture cap is not read.
    pub(super) async fn fetch(
        &self,
        blobs: &mut BTreeMap<BlobId, Vec<u8>>,
        path: &RepoPath,
        blob: BlobId,
        capped: bool,
    ) -> Result<bool> {
        if blobs.contains_key(&blob) {
            return Ok(true);
        }
        Ok(match self.acquisition.blob(path, blob, capped).await? {
            BlobRead::Found(bytes) => {
                blobs.insert(blob, bytes);
                true
            }
            BlobRead::TooLarge | BlobRead::Missing => false,
        })
    }

    /// The text of a blob, or why there is none. Content over the capture cap is
    /// not read.
    pub(super) async fn text_of(
        &self,
        blobs: &mut BTreeMap<BlobId, Vec<u8>>,
        path: &RepoPath,
        blob: BlobId,
    ) -> Result<std::result::Result<String, NoText>> {
        Ok(if !self.fetch(blobs, path, blob, true).await? {
            Err(NoText::Unavailable)
        } else {
            decode_text(&blobs[&blob]).ok_or(NoText::Binary)
        })
    }
}
