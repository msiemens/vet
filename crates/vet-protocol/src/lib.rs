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
    HelloAck {
        protocol_version: u32,
    },
    Response {
        id: u32,
        response: Response,
    },
    /// `id` is `None` when the failure is not attributable to a request.
    Error {
        id: Option<u32>,
        message: String,
    },
    /// Pushed without a request. A connection receives the events of the sessions
    /// it opened, until it closes them or disconnects; there is no other subscription.
    /// Opening a repository that is already open subscribes to the existing session.
    Event {
        event: Event,
    },
}

/// Assigned by the backend in [`Response::Session`].
pub type SessionId = u32;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Request {
    /// Round-trip probe, used to measure transport latency.
    Ping {
        payload: String,
    },
    /// Opens a review of the dirty tree (`HEAD` to the working tree) of the
    /// repository containing `repo`, which may be any folder inside it, and
    /// subscribes this connection to the session's events. Fails if `repo` is
    /// not inside a non-bare repository.
    OpenSession {
        repo: String,
    },
    CloseSession {
        session: SessionId,
    },
    /// The changeset as it is now, with review state.
    Manifest {
        session: SessionId,
    },
    /// Both sides of one text file at the current revision.
    FileContents {
        session: SessionId,
        path: String,
    },
    /// Marks hunks (indices into the manifest of `revision`) seen or unseen.
    /// Fails if `revision` is no longer current.
    SetSeen {
        session: SessionId,
        revision: u32,
        path: String,
        hunks: Vec<u32>,
        seen: bool,
    },
    /// Acknowledges or un-acknowledges one non-text item. Fails if `revision` is
    /// no longer current.
    Acknowledge {
        session: SessionId,
        revision: u32,
        path: String,
        kind: NonTextKind,
        acknowledged: bool,
    },
    /// Hover at a target-side position.
    Hover {
        session: SessionId,
        path: String,
        line: u32,
        col: u32,
    },
    /// Changes what the live tree is compared against (§6.5). Review state is
    /// remapped onto the new base. The revision increases and every subscriber,
    /// including the requester, gets `SnapshotSettled`; fetch the manifest again.
    /// Fails with a message fit to show if the choice does not resolve.
    SetBase {
        session: SessionId,
        base: BaseChoice,
    },
    /// What a base can be picked from.
    ListBaseCandidates {
        session: SessionId,
    },
    /// The direct children of a directory of the current target snapshot.
    /// `path` is `""` for the root, else `a/b` without a trailing slash. Fails if
    /// the snapshot has nothing there.
    ListDirectory {
        session: SessionId,
        path: String,
    },
    /// The target-side text of any path of the current target snapshot, changed
    /// or not.
    FileText {
        session: SessionId,
        path: String,
    },
    /// Every thread with its anchor resolved at the current revision. Resolution
    /// is lazy: a settled snapshot only invalidates it, and this request computes
    /// it. Kept apart from `Manifest` so the manifest does not wait for it.
    Threads {
        session: SessionId,
    },
    /// Opens a thread with its first message. The lines of a range target are in
    /// the text of `revision`. Fails if `revision` is no longer current, or if the
    /// target does not exist at it. Answers with `Response::Thread`.
    CreateThread {
        session: SessionId,
        revision: u32,
        target: ThreadTarget,
        body: String,
    },
    /// Answers with `Response::Thread`.
    Reply {
        session: SessionId,
        thread: u32,
        body: String,
    },
    /// Resolves or reopens a thread. Answers with `Response::Thread`.
    SetThreadStatus {
        session: SessionId,
        thread: u32,
        status: ThreadStatus,
    },
    /// Deletes a thread; this is also how a detached one is discarded.
    DeleteThread {
        session: SessionId,
        thread: u32,
    },
    /// Re-anchors a thread whose anchor is `Inferred` at its current location,
    /// which makes it `Exact`. Fails if `revision` is no longer current or the
    /// anchor is not `Inferred`. Answers with `Response::Thread`.
    ConfirmAnchor {
        session: SessionId,
        revision: u32,
        thread: u32,
    },
    /// Re-anchors a thread whose anchor is `Ambiguous` at one of its candidates
    /// (an index into the candidates of `revision`). Fails like `ConfirmAnchor`.
    /// Answers with `Response::Thread`.
    AttachAnchor {
        session: SessionId,
        revision: u32,
        thread: u32,
        candidate: u32,
    },
}

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

/// A side of the changeset.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Side {
    Base,
    Target,
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
    /// `None` for a changeset thread, which never detaches. A file thread is
    /// `Exact`, or `Stale` once its file is deleted.
    pub anchor: Option<AnchorState>,
    /// The first one opened the thread.
    pub messages: Vec<ThreadMessage>,
    pub status: ThreadStatus,
}

/// How a session's base is chosen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum BaseChoice {
    /// `HEAD` at the time the session was opened.
    Pin,
    /// The merge base of `reference` (a branch or any revision) and `HEAD`.
    MergeBase { reference: String },
    /// A commit: a full or abbreviated id, or any revision.
    Commit { id: String },
}

/// A base as resolved.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Base {
    /// A commit is reported by its full id.
    pub choice: BaseChoice,
    /// The full id of the base commit.
    pub commit: String,
    /// What the choice is, without the commit: `pin`, `merge-base main`, `commit`.
    pub label: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchInfo {
    pub name: String,
    /// Full id of the tip.
    pub commit: String,
    /// The checked-out branch.
    pub current: bool,
    /// The remote's default branch (`origin/HEAD`); never set without a remote.
    pub default: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub time: u32,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Response {
    Pong {
        payload: String,
    },
    Session {
        session: SessionId,
        /// The root of the repository the opened path belongs to.
        root: String,
    },
    /// Acknowledges a request that has no result.
    Done,
    Manifest(Manifest),
    /// `None` is a side the file does not have (added or deleted). The text and
    /// `marks` belong to `revision`; the hunks are in that revision's manifest.
    FileContents {
        revision: u32,
        base: Option<String>,
        target: Option<String>,
        marks: Vec<Mark>,
    },
    /// The file's review state after `SetSeen` or `Acknowledge`.
    Review(ReviewUpdate),
    Hover {
        text: String,
    },
    /// The base after `SetBase`.
    Base(Base),
    /// Local branches by name, and up to about 200 commits reachable from
    /// `HEAD`, newest first.
    BaseCandidates {
        branches: Vec<BranchInfo>,
        commits: Vec<CommitInfo>,
    },
    /// Directories first, then the others, each by name in byte order. `revision`
    /// is the one the listing belongs to.
    Directory {
        revision: u32,
        entries: Vec<DirEntry>,
    },
    /// The answer to `FileText`, at `revision`.
    FileText {
        revision: u32,
        content: TextContent,
    },
    /// The answer to `Threads`, at `revision`, in creation order.
    Threads {
        revision: u32,
        threads: Vec<Thread>,
    },
    /// One thread after a change, resolved at the current revision.
    Thread(Thread),
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
    Text { text: String },
    NoText { reason: NoTextReason },
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
    ReviewChanged {
        session: SessionId,
        revision: u32,
        update: ReviewUpdate,
    },
    /// A thread was created, replied to, resolved, re-anchored or deleted by
    /// another connection; fetch the threads again. Not sent for a settled
    /// snapshot, which is `SnapshotSettled`: that also invalidates the threads.
    ThreadsChanged { session: SessionId, revision: u32 },
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
    /// What the files are compared against.
    pub base: Base,
    pub files: Vec<FileChange>,
    /// Completion of the whole changeset.
    pub completion: Completion,
    /// Changed lines and non-text items the filter hides, so "complete" never
    /// implies every change was seen (§5.2).
    pub excluded_lines: u32,
    pub excluded_items: u32,
    /// Set when the backend polls the tree instead of watching it, so changes
    /// show up late (§4.1, §6.4); the shell shows a banner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub degraded: Option<WatchDegraded>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WatchDegraded {
    /// A WSL worktree under `/mnt/*`: file events do not cross 9P.
    MountedWindowsDrive,
    /// The OS watch limit is exhausted.
    WatchLimit,
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
