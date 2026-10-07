//! Files of the changeset and of the target snapshot: their changes, texts and review state.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Completion, Highlights};

/// One side of a non-text item's file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum BlobContent {
    Bytes {
        /// Set for png, jpeg, gif, webp, bmp, ico and svg files, by extension.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        media_type: Option<String>,
        /// Base64 (standard alphabet, padded) of the file.
        bytes: String,
    },
    /// The item has no file on this side.
    Absent,
    /// Over the size the backend returns (8 MiB), or over the capture cap.
    TooLarge,
    /// A symlink or submodule, or content that is not readable any more.
    Unavailable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DirEntryKind {
    File,
    Dir,
    Symlink,
    Submodule,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirEntry {
    pub name: String,
    pub kind: DirEntryKind,
    /// The path, or for a directory anything below it, is changed in the
    /// changeset (whatever the review filter says).
    pub changed: bool,
    /// The review filter excludes the path, or for a directory everything below it.
    pub excluded: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum TextContent {
    /// `highlights` is empty for a file without a known language (see [`Highlights`]).
    Text {
        text: String,
        highlights: Highlights,
    },
    NoText {
        reason: NoTextReason,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NoTextReason {
    Binary,
    /// Over the capture cap, or its content could not be read.
    Unavailable,
    /// A symlink or a submodule.
    NotAFile,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileChange {
    pub path: String,
    /// The base path of a renamed file.
    pub old_path: Option<String>,
    pub status: FileStatus,
    /// Lines of the base text as an editor numbers them (a final line break is followed by one
    /// more, empty line, and an empty text has one); 0 for a file without a text diff.
    pub base_lines: u32,
    /// Lines of the target text, counted like `base_lines`.
    pub target_lines: u32,
    /// Text hunks in order; the sizes let a shell lay out views before it has the text.
    pub hunks: Vec<Hunk>,
    /// The lines `hunks` add and remove.
    pub delta: LineDelta,
    /// Each is one review item (§5.2).
    pub non_text: Vec<NonTextItem>,
    pub review: FileReview,
    pub completion: Completion,
}

/// Lines a diff adds (the new side's lines its hunks cover) and removes (the old side's).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LineDelta {
    pub added: u32,
    pub removed: u32,
}

/// A non-text change with both sides; `None` is a side that does not exist.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NonTextItem {
    pub kind: NonTextKind,
    pub before: Option<EntryDesc>,
    pub after: Option<EntryDesc>,
    /// The item is an image that can be previewed: `ItemBlob` answers with its bytes and
    /// media type for a side that has a file.
    pub has_preview: bool,
    /// What a `Mode` item changes; none for the other kinds.
    pub mode_change: Option<ModeChange>,
}

/// How a file's mode changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ModeChange {
    /// An executable bit was set that no bit was before.
    GainedExecutable,
    /// The last executable bit was cleared.
    LostExecutable,
    /// Any other change of the mode.
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NonTextKind {
    Mode,
    Symlink,
    Submodule,
    Binary,
    ContentUnavailable,
    /// A rename without text changes.
    Rename,
    /// An empty file added or deleted.
    Empty,
}

/// Review state derived from the backend's spans (§5.2); the spans stay there.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileReview {
    /// One per hunk of the file.
    pub hunks: Vec<HunkState>,
    /// One per hunk: its changed lines covered only by spans seen against an old base.
    pub old_base_lines: Vec<u32>,
    pub acknowledged: Vec<NonTextKind>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HunkState {
    Unseen,
    Seen,
    /// Target lines unchanged since reviewed, base lines changed. Counts as unseen (§5.2).
    SeenAgainstOldBase,
    /// Interdiff only: nothing is left to review in the hunk (the author took a change back to
    /// the base content). It cannot be marked and is not counted.
    Informational,
    /// Interdiff only: unseen, and the old version's patch did not apply to the new base here,
    /// so the old side shows its lines unrebased.
    Unapplied,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntryKind {
    File,
    Symlink,
    Submodule,
}

/// One side of a file's non-text change.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntryDesc {
    pub kind: EntryKind,
    /// Git file mode as a number, e.g. 33261 for `100755`.
    pub mode: u32,
    /// Symlink target or submodule commit.
    pub target: Option<String>,
    /// Content size in bytes, saturating at `u32::MAX`; `None` for symlinks and submodules.
    pub size: Option<u32>,
}

/// A changed text region. Line numbers are 0-based; a pure insertion has
/// `base_len == 0`, a pure deletion `target_len == 0`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Hunk {
    pub base_start: u32,
    pub base_len: u32,
    pub target_start: u32,
    pub target_len: u32,
}
