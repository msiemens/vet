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
    pub author: String,
    pub body: String,
    /// Unix seconds.
    pub at: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AnchorStrategy {
    ContextWindow,
    CrossFileMove,
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

/// A comment thread. `anchor` and `location` belong to the revision of the
/// response; they are re-resolved from the recorded anchor each revision.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Thread {
    pub id: u32,
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
}
