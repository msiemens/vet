//! Reading the content a session shows: texts, and what a card says of a
//! non-text file.

use std::collections::BTreeMap;

use vet_core::content::{BlobId, BlobSource, EntryKind, FileEntry, NonTextKind, RepoPath, decode_text};

use super::Result;
use super::views::EntryInfo;
use crate::acquisition::Acquisition;
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

/// Texts looked up in two maps: those of a diff, which the engine holds, and others read for a
/// purpose of their own.
pub(super) struct Texts<'a> {
    pub(super) diffed: &'a BTreeMap<BlobId, Vec<u8>>,
    pub(super) other: &'a BTreeMap<BlobId, Vec<u8>>,
}

impl BlobSource for Texts<'_> {
    fn blob(&self, id: BlobId) -> Option<&[u8]> {
        self.diffed.blob(id).or_else(|| self.other.blob(id))
    }
}

impl<G: GitRepo + Sync, S: LiveStore + Sync> Acquisition<G, S> {
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
                target: self.git.submodule_commit(entry.blob).await.ok().flatten(),
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
        Ok(match self.blob(path, blob, capped).await? {
            BlobRead::Found(bytes) => {
                blobs.insert(blob, bytes);
                true
            }
            BlobRead::TooLarge | BlobRead::Missing => false,
        })
    }

    /// The text of a blob, or why there is none. Content over the capture cap is
    /// not read, unless `known` has it.
    pub(super) async fn text_of(
        &self,
        known: &BTreeMap<BlobId, Vec<u8>>,
        path: &RepoPath,
        blob: BlobId,
    ) -> Result<std::result::Result<String, NoText>> {
        let mut read = BTreeMap::new();
        let bytes = match known.get(&blob) {
            Some(bytes) => bytes,
            None if self.fetch(&mut read, path, blob, true).await? => &read[&blob],
            None => return Ok(Err(NoText::Unavailable)),
        };
        Ok(decode_text(bytes).ok_or(NoText::Binary))
    }
}
