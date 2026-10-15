//! Comment threads and where they are anchored.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Side;

/// What a new thread is attached to.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ThreadTarget {
    /// `len` lines from line `start` (0-based) of one side of a file.
    Range {
        side: Side,
        path: String,
        start: u32,
        len: u32,
    },
    File {
        side: Side,
        path: String,
    },
    /// The changeset as a whole.
    Changeset,
}

/// What a comment is about, in the terms of a changeset file's view.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CommentAt {
    /// The file as a whole.
    File,
    /// The hunk with this index in the file's manifest entry.
    Hunk { index: u32 },
    /// `len` lines from line `start` (0-based) of `side`'s text of the file.
    Lines { side: Side, start: u32, len: u32 },
}

/// Lines `start..start + len` (0-based) of a file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LineLocation {
    pub path: String,
    pub start: u32,
    pub len: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ThreadStatus {
    Open,
    Resolved,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ThreadKind {
    Range,
    File,
    Changeset,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadMessage {
    /// What `EditMessage`, `RemoveMessage` and `KeepEdit` take.
    pub id: u32,
    pub author: String,
    pub body: String,
    /// Unix seconds.
    pub at: u32,
    /// The viewer wrote it, here or on the platform: it can be edited and removed.
    pub mine: bool,
    /// The platform's picture of the author; without one the UI shows the author's initials.
    pub avatar: Option<String>,
    /// Whether the platform has the comment; on a thread without a `platform` always `draft`.
    pub state: CommentState,
    /// Unix seconds of the viewer's last edit of the text.
    pub edited_at: Option<u32>,
    /// Set when the text was edited here and on the platform since the last sync: both are kept
    /// until `KeepEdit` chooses one, and nothing is posted from the thread meanwhile.
    pub conflict: Option<EditConflict>,
    /// Someone else wrote it after the viewer last looked at the pull request's comments (D96).
    /// Vet's conclusion from the time they last looked, not the shell's.
    pub new: bool,
}

/// Where one of the viewer's comments is on its way to the platform.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CommentState {
    /// Only in Vet; `SubmitReview` or `PostDrafts` posts it.
    Draft,
    /// Handed to the platform without an answer yet; the next sync finds out whether it took it.
    Posting,
    /// The platform has it.
    Posted,
    /// The platform refused it, for `error`; it is posted with the next post.
    Failed { error: String },
}

/// The platform's text of a comment that was also edited here.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditConflict {
    pub remote_body: String,
    /// Unix seconds the platform's edit was made.
    pub remote_at: u32,
}

/// What a thread is on a platform.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadPlatform {
    /// The platform has the thread; otherwise it is a draft.
    pub posted: bool,
    /// A flat comment on the whole pull request cannot be resolved.
    pub resolvable: bool,
    /// Who resolved it on the platform.
    pub resolved_by: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AnchorStrategy {
    ContextWindow,
    CrossFileMove,
    /// The commented lines were deleted; the thread is where they were.
    Deleted,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AnchorConfidence {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StaleReason {
    ContextChanged,
    FileDeleted,
}

/// Where a thread's anchor is now (§5.1).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum AnchorState {
    /// At `Thread::location`; `whitespace_changed` if only whitespace differs.
    Exact { whitespace_changed: bool },
    /// At `Thread::location`, found by evidence: show a "moved" badge, offer
    /// `ConfirmAnchor`. `Thread::original` is where it was recorded.
    Inferred {
        strategy: AnchorStrategy,
        confidence: AnchorConfidence,
    },
    /// Detached: no location. Offer `DeleteThread`.
    Stale { reason: StaleReason },
    /// Detached: several plausible locations, offered to `AttachAnchor` by index.
    Ambiguous { candidates: Vec<LineLocation> },
}

/// A comment thread. `anchor` and `location` belong to `revision`; they are re-resolved
/// from the recorded anchor each revision.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Thread {
    pub id: u32,
    /// The revision `anchor` and `location` were resolved at.
    pub revision: u32,
    pub kind: ThreadKind,
    /// `None` for a changeset thread.
    pub side: Option<Side>,
    /// The file the thread is shown at: for a range thread where its anchor is
    /// now, else where it was recorded; for a file thread where the file is now
    /// (it follows a git-detected rename). `None` for a changeset thread.
    pub path: Option<String>,
    /// The recorded range of a range thread.
    pub original: Option<LineLocation>,
    /// Where a range thread is now; `None` when it is detached, and for file
    /// and changeset threads.
    pub location: Option<LineLocation>,
    /// The line (0-based) of the target text that a unified diff shows a range thread's
    /// card below: where `location` ends, mapped through the hunks if the thread is on the
    /// base side, and not past the file's last line. Set exactly when `location` is.
    pub target_line: Option<u32>,
    /// `None` for a changeset thread, which never detaches. A file thread is
    /// `Exact`, or `Stale` once its file is deleted.
    pub anchor: Option<AnchorState>,
    /// The first one opened the thread.
    pub messages: Vec<ThreadMessage>,
    pub status: ThreadStatus,
    /// `None` for a thread of a changeset that is not on a platform.
    pub platform: Option<ThreadPlatform>,
}
