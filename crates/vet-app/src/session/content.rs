//! Serving file content: both sides of a changed file, the target text of any
//! file, and the bytes of a non-text item.

use std::collections::BTreeMap;

use vet_core::content::NonTextKind;
use vet_core::content::{BlobId, EntryKind, RepoPath};
use vet_core::inline::TextChange;
use vet_core::linemap::{LineMap, MappedLine};
use vet_core::review::Side;

use super::blobs::{NoText, image_media_type};
use super::state::{View, check_revision, find_diff, find_non_text};
use super::{LiveSession, Result, SessionError};
use crate::editor;
use crate::ports::{BlobRead, GitRepo, LiveStore};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileText {
    pub revision: u32,
    pub base: Option<String>,
    pub target: Option<String>,
    /// The diff's changed ranges within the texts.
    pub changes: Vec<TextChange>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetText {
    pub revision: u32,
    pub content: std::result::Result<String, NoText>,
}

/// Largest file `item_blob` returns.
pub const MAX_ITEM_BLOB: usize = 8 * 1024 * 1024;

/// One side of a non-text item's file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ItemBlob {
    Bytes {
        /// Only for common image formats.
        media_type: Option<&'static str>,
        bytes: Vec<u8>,
    },
    /// The item has no file on this side.
    Absent,
    /// Over [`MAX_ITEM_BLOB`] or the capture cap.
    TooLarge,
    /// A symlink or submodule, or content that is not readable any more.
    Unavailable,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Both sides of a file with text, at the current revision.
    pub async fn file_text(&self, view: View, path: &str) -> Result<FileText> {
        let (st, shown) = self.viewing(view).await?;
        let changes = shown.diffed(&st);
        type Side = Option<(RepoPath, BlobId)>;
        let diff = find_diff(&changes.diff, path);
        let hunks = diff.map(|d| d.hunks.clone()).unwrap_or_default();
        let (old, new): (Side, Side) = if let Some(d) = diff {
            let side = |p: &Option<RepoPath>, b: Option<BlobId>| p.clone().zip(b);
            (side(&d.old_path, d.old_blob), side(&d.new_path, d.new_blob))
        } else if let Some(n) = changes.diff.non_text.iter().find(|n| n.path.to_string_lossy() == path) {
            let side = |p: RepoPath, e: &Option<vet_core::content::FileEntry>| {
                e.as_ref().filter(|e| e.kind == EntryKind::File).map(|e| (p, e.blob))
            };
            (
                side(n.from.clone().unwrap_or_else(|| n.path.clone()), &n.before),
                side(n.path.clone(), &n.after),
            )
        } else {
            return Err(SessionError::NotFound(format!("file {path}")));
        };
        let mut texts = [None, None];
        for (text, side) in texts.iter_mut().zip([old, new]) {
            let Some((p, b)) = side else { continue };
            match self.acquisition.text_of(&changes.blobs, &p, b).await? {
                Ok(t) => *text = Some(t),
                Err(_) => return Err(SessionError::NotText(p.to_string_lossy())),
            }
        }
        let [base, target] = texts;
        let changes = editor::text_changes(
            base.as_deref().unwrap_or_default(),
            target.as_deref().unwrap_or_default(),
            &hunks,
        );
        Ok(FileText {
            revision: st.revision,
            base,
            target,
            changes,
        })
    }

    /// The text of any file on the target side of `view`, changed or not: the changeset's target
    /// snapshot, or for a commit the tree it leaves (an interdiff reads the changeset's). A path
    /// the changeset's target lacks is an error; a path a commit lacks is an ordinary answer
    /// ([`NoText::Absent`]), since a tab stays at its commit while the file comes and goes.
    pub async fn target_text(&self, view: View, path: &str) -> Result<TargetText> {
        let st = match view {
            View::Commit { .. } => self.viewing(view).await?.0,
            _ => self.published.read(),
        };
        let target = match view {
            View::Commit { index } => &st.units()[index as usize].new,
            _ => &*st.target,
        };
        let key = RepoPath::from(path);
        let Some(entry) = target.entries.get(&key) else {
            if matches!(view, View::Commit { .. }) {
                return Ok(TargetText {
                    revision: st.revision,
                    content: Err(NoText::Absent),
                });
            }
            return Err(SessionError::NotFound(format!("file {path}")));
        };
        let content = match entry.kind {
            EntryKind::File => self.acquisition.text_of(&BTreeMap::new(), &key, entry.blob).await?,
            EntryKind::Symlink | EntryKind::Submodule => Err(NoText::NotAFile),
        };
        Ok(TargetText {
            revision: st.revision,
            content,
        })
    }

    /// Where the lines of the target text of `path` in version `from` of the open review are in the
    /// target text at `revision`. Fails if `revision` is no longer current, no review is open,
    /// there is no such version, or the file has no text in the version or now.
    pub async fn map_lines(&self, revision: u32, from: u32, path: &str, lines: &[u32]) -> Result<Vec<MappedLine>> {
        let st = self.published.read();
        check_revision(&st, revision)?;
        let Some(changeset) = st.changeset.filter(|_| st.review.is_some()) else {
            return Err(SessionError::Invalid(
                "versions are recorded once a review has started".into(),
            ));
        };
        let version = (self.acquisition.store.versions(changeset).await?)
            .into_iter()
            .find(|v| v.n == from)
            .ok_or_else(|| SessionError::NotFound(format!("version {from}")))?;
        let Some((earlier, _)) = self.acquisition.store.snapshot(version.target).await? else {
            return Err(SessionError::NotFound("snapshot of the version".into()));
        };
        let key = RepoPath::from(path);
        let mut texts = Vec::new();
        for snapshot in [&earlier, &*st.target] {
            let entry = (snapshot.entries.get(&key))
                .filter(|e| e.kind == EntryKind::File)
                .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
            match self.acquisition.text_of(&BTreeMap::new(), &key, entry.blob).await? {
                Ok(text) => texts.push(text),
                Err(_) => return Err(SessionError::NotText(path.to_owned())),
            }
        }
        let map = LineMap::between(texts[0].as_bytes(), texts[1].as_bytes());
        Ok(lines.iter().map(|&line| map.map_line(line)).collect())
    }

    /// The bytes of one side of a non-text item's file at `revision`.
    pub async fn item_blob(
        &self,
        revision: u32,
        view: View,
        path: &str,
        kind: NonTextKind,
        side: Side,
    ) -> Result<ItemBlob> {
        let (st, shown) = self.viewing(view).await?;
        check_revision(&st, revision)?;
        let change = find_non_text(&shown.diffed(&st).diff, kind, path)
            .ok_or_else(|| SessionError::NotFound(format!("change of {path}")))?;
        let (file, entry) = match side {
            Side::Base => (change.from.as_ref().unwrap_or(&change.path), &change.before),
            Side::Target => (&change.path, &change.after),
        };
        let Some(entry) = entry else {
            return Ok(ItemBlob::Absent);
        };
        if entry.kind != EntryKind::File {
            return Ok(ItemBlob::Unavailable);
        }
        Ok(match self.acquisition.blob(file, entry.blob, true).await? {
            BlobRead::Found(bytes) if bytes.len() <= MAX_ITEM_BLOB => ItemBlob::Bytes {
                media_type: image_media_type(file),
                bytes,
            },
            BlobRead::Found(_) | BlobRead::TooLarge => ItemBlob::TooLarge,
            BlobRead::Missing => ItemBlob::Unavailable,
        })
    }
}
