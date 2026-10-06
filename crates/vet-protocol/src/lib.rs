//! Shell <-> backend protocol (SPEC §4.1): one protocol over one loopback
//! WebSocket in every topology, JSON text frames.
//!
//! Ids and counts are `u32` so they map to plain TS `number`s.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 1;

/// The one line the backend prints to stdout once it is listening. The shell
/// reads it to learn the port and the per-launch random token.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Ready {
    pub port: u16,
    pub token: String,
}

/// First message on every connection must be `Hello` with the token from
/// [`Ready`]; there is no unauthenticated endpoint.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ClientMessage {
    Hello { token: String, protocol_version: u32 },
    Request { id: u32, request: Request },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ServerMessage {
    HelloAck { protocol_version: u32 },
    Response { id: u32, response: Response },
    /// `id` is `None` when the failure is not attributable to a request.
    Error { id: Option<u32>, message: String },
    /// Pushed without a request. A connection receives the events of the sessions
    /// it opened, until it closes them or disconnects; there is no other subscription.
    Event { event: Event },
}

/// Assigned by the backend in [`Response::Session`].
pub type SessionId = u32;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Request {
    /// Round-trip probe, used to measure transport latency.
    Ping { payload: String },
    /// Opens a review of the dirty tree of the repository at `repo` (`HEAD` to the
    /// working tree) and subscribes this connection to the session's events.
    OpenSession { repo: String },
    CloseSession { session: SessionId },
    /// The changeset as it is now, with review state.
    Manifest { session: SessionId },
    /// Both sides of one text file at the current revision.
    FileContents { session: SessionId, path: String },
    /// Marks hunks (indices into the manifest of `revision`) seen or unseen.
    /// Fails if `revision` is no longer current.
    SetSeen { session: SessionId, revision: u32, path: String, hunks: Vec<u32>, seen: bool },
    /// Acknowledges or un-acknowledges one non-text item. Fails if `revision` is
    /// no longer current.
    Acknowledge { session: SessionId, revision: u32, path: String, kind: NonTextKind, acknowledged: bool },
    /// Hover at a target-side position.
    Hover { session: SessionId, path: String, line: u32, col: u32 },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Response {
    Pong { payload: String },
    Session { session: SessionId },
    /// Acknowledges a request that has no result.
    Done,
    Manifest(Manifest),
    /// `None` is a side the file does not have (added or deleted). The text and
    /// `marks` belong to `revision`; the hunks are in that revision's manifest.
    FileContents { revision: u32, base: Option<String>, target: Option<String>, marks: Vec<Mark> },
    /// The file's review state after `SetSeen` or `Acknowledge`.
    Review(ReviewUpdate),
    Hover { text: String },
}

/// A file's new review state with the completion it implies, so a shell never
/// derives progress itself.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewUpdate {
    pub path: String,
    pub review: FileReview,
    /// The file's completion.
    pub completion: Completion,
    /// The changeset's completion after the change.
    pub changeset: Completion,
}

/// Progress against the review filter (§5.2): changed lines (counted per side)
/// and non-text items.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Completion {
    pub lines: u32,
    pub seen_lines: u32,
    /// Lines seen against an older base; they count as unseen in `seen_lines`.
    pub old_base_lines: u32,
    pub items: u32,
    pub acknowledged_items: u32,
}

/// Events are invalidations or small state updates; the client re-requests
/// whatever it displays.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Event {
    /// A snapshot settled and the changeset moved to a new `revision`. The
    /// manifest, with review state remapped onto it, is stale: fetch it again.
    SnapshotSettled { session: SessionId, revision: u32 },
    /// Review state of one file changed at the current `revision`.
    ReviewChanged { session: SessionId, revision: u32, update: ReviewUpdate },
}

/// The changeset as displayed through the session's review filter.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Manifest {
    pub session: SessionId,
    /// Increases whenever the changeset's target changes.
    pub revision: u32,
    pub name: String,
    pub files: Vec<FileChange>,
    /// Completion of the whole changeset.
    pub completion: Completion,
    /// Changed lines and non-text items the filter hides, so "complete" never
    /// implies every change was seen (§5.2).
    pub excluded_lines: u32,
    pub excluded_items: u32,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileChange {
    pub path: String,
    /// The base path of a renamed file.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub base_lines: u32,
    pub target_lines: u32,
    /// Text hunks in order; the sizes let a shell lay out views before it has the text.
    pub hunks: Vec<Hunk>,
    /// Each is one review item (§5.2).
    pub non_text: Vec<NonTextItem>,
    pub review: FileReview,
    pub completion: Completion,
}

/// A non-text change with both sides; `None` is a side that does not exist.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NonTextItem {
    pub kind: NonTextKind,
    pub before: Option<EntryDesc>,
    pub after: Option<EntryDesc>,
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
}

/// Review state derived from the backend's spans (§5.2); the spans stay there.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileReview {
    /// One per hunk of the file.
    pub hunks: Vec<HunkState>,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MarkKind {
    /// A changed symbol (derived, §7.3).
    Symbol,
    /// Reachability of the changed code (derived, §7.4).
    Reachability,
    /// An executor-relevant or whitespace note.
    Note,
}

/// An inline decoration on the target side. `from`/`to` are UTF-16 columns, as
/// CodeMirror counts them.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Mark {
    pub line: u32,
    pub from: u32,
    pub to: u32,
    pub kind: MarkKind,
}
