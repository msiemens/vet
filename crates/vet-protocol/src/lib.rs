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
    /// Opens the working tree of the repository containing `repo`, which may be
    /// any folder inside it, against its current branch, and subscribes this
    /// connection to the session's events. The repository's open review is
    /// resumed; with none, the session is browsing (`ReviewInfo`). Fails if
    /// `repo` is not inside a non-bare repository.
    OpenSession {
        repo: String,
    },
    CloseSession {
        session: SessionId,
    },
    /// Ends the open review: its marks, acknowledgements and threads are kept
    /// in the store but leave the session, which goes back to browsing. The
    /// revision increases and every subscriber, including the requester, gets
    /// `SnapshotSettled`; fetch the manifest again. Fails if no review is open.
    /// Answers with `Response::ReviewFinished`.
    FinishReview {
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
    /// Hover at a position of one side of the changeset at `revision`. Positions
    /// are 0-based lines and UTF-16 columns, in the text of that side. Fails if
    /// `revision` is no longer current; a language without a usable server
    /// answers with `unavailable` set instead. Answers with `Response::Hover`.
    Hover {
        session: SessionId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
    },
    /// Where the symbol at the position is defined. Answers with `Response::Definition`;
    /// otherwise like `Hover`.
    Definition {
        session: SessionId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
    },
    /// The references to the symbol at the position, without its declaration.
    /// Answers with `Response::References`; otherwise like `Hover`.
    References {
        session: SessionId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
    },
    /// The diagnostics of a file of the target side. The language server is
    /// asked and may take seconds on a cold start: there is no push.
    /// Answers with `Response::Diagnostics`; otherwise like `Hover`.
    Diagnostics {
        session: SessionId,
        revision: u32,
        path: String,
    },
    /// The symbols the changeset touches (derived, §7.3): both sides of every
    /// changed file, intersected with the diff hunks. Fails if `revision` is no
    /// longer current. Answers with `Response::ChangedSymbols`.
    ChangedSymbols {
        session: SessionId,
        revision: u32,
    },
    /// References and callers of an added or modified changed symbol, on the
    /// target side (derived). Fails if the symbol is not one of
    /// `ChangedSymbols` at `revision`, or was removed. Answers with `Response::Impact`.
    Impact {
        session: SessionId,
        revision: u32,
        symbol: SymbolId,
    },
    /// What became of the references to a removed symbol (derived). Fails if the
    /// symbol is not a removed one of `ChangedSymbols` at `revision`. Answers
    /// with `Response::RemovalImpact`.
    RemovalImpact {
        session: SessionId,
        revision: u32,
        symbol: SymbolId,
    },
    /// Whether an added or modified changed symbol is reached from an entry
    /// point (derived, §7.4). Fails like `Impact`. Answers with `Response::Reachability`.
    Reachability {
        session: SessionId,
        revision: u32,
        symbol: SymbolId,
    },
    /// Each language's server as it resolves for the session's repository.
    /// Answers with `Response::Servers`.
    ListServers {
        session: SessionId,
    },
    /// Installs the pinned server of a language into Vet's data directory and
    /// restarts that language's running servers. Output lines arrive as
    /// `ServerInstallOutput` events until the answer, which is `Response::Servers`
    /// as after the install. Fails with the tool's output tail if it does not work.
    InstallServer {
        session: SessionId,
        language: String,
    },
    /// Stores the path of the server to use for a language, or removes it with
    /// `None`. The path is not checked here: `ListServers` says whether it works.
    /// Restarts that language's running servers. Answers with `Response::Servers`.
    SetServerPath {
        session: SessionId,
        language: String,
        path: Option<String>,
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
    /// Replaces the review filter of the repository `session` has open and
    /// persists it: a project setting, not part of the session (`session` is
    /// only the handle to its repository). The revision increases and every
    /// subscriber, including the requester, gets `SnapshotSettled`; fetch the
    /// manifest again. Fails with a message fit to show if a glob is invalid, and
    /// then changes nothing. Answers with `Response::Done`.
    SetProjectFilter {
        session: SessionId,
        filter: ReviewFilter,
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
    /// The bytes of one side of a non-text item's file, to preview an image. The
    /// item is found by `path` and `kind` as in `Acknowledge`; `Base` is the file
    /// before (at its old path for a rename), `Target` the file after. Fails if
    /// `revision` is no longer current or there is no such item. Answers with
    /// `Response::ItemBlob`.
    ItemBlob {
        session: SessionId,
        revision: u32,
        path: String,
        kind: NonTextKind,
        side: Side,
    },
    /// The run targets of the current target snapshot (§10): discovered from its
    /// manifests, with the user's edits applied, then the targets the user added.
    /// Answers with `Response::RunTargets`.
    ListRunTargets {
        session: SessionId,
    },
    /// Stores an edited discovered target (same `id`) or a new one, per repository.
    /// For a new target the client chooses a non-empty `id` no discovered target
    /// uses. Fails with a message fit to show if the id or the command is empty
    /// or `cwd` is not inside the repository. Answers with `Response::Done`.
    SaveRunTarget {
        session: SessionId,
        target: RunTarget,
    },
    /// Drops the stored target: an edited discovered target reverts, an added one
    /// disappears. Answers with `Response::Done`.
    ResetRunTarget {
        session: SessionId,
        id: String,
    },
    /// Starts a target in place (§9): the backend reconciles the tree, applies the
    /// run check (§7.5) and either starts the process or asks for approval first.
    /// Answers with `Response::RunNeedsApproval`, which starts nothing, or
    /// `Response::RunStarted`. After the former, ask again with its `hash` as
    /// `approved_hash` to approve and run; the hash is honored only while the
    /// files about to run still hash to it, else the answer is a new
    /// `RunNeedsApproval`.
    Run {
        session: SessionId,
        target_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        approved_hash: Option<String>,
    },
    /// The files behind a pending `RunNeedsApproval`, with their text. Fails if
    /// no approval with this hash is pending. Answers with `Response::RunApprovalDiff`.
    RunApprovalDiff {
        session: SessionId,
        hash: String,
    },
    /// Kills a running process; `RunExited` follows. Answers with `Response::Done`,
    /// also for a run that already exited.
    StopRun {
        session: SessionId,
        run_id: u32,
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

/// What the review shows and counts (§5.2). A path passes if it is under one of
/// `roots` (or there are none), matches `include` (or there is none) and does not
/// match `exclude`. A glob without a `/` matches the file name at any depth, a
/// leading `/` anchors it at the root; `*` and `?` stop at `/`, `**` does not.
#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewFilter {
    /// Directories, `a/b` without a trailing slash.
    pub roots: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Binary changes are neither shown nor counted.
    pub hide_binary: bool,
}

/// How a session's base is chosen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum BaseChoice {
    /// The tip of a local branch as it is when chosen; the branch moving on
    /// does not move the base (`Base::behind`), choosing it again does.
    Branch { name: String },
    /// Where `HEAD` branched off `reference` (a branch or any revision): their
    /// merge base.
    MergeBase { reference: String },
    /// A commit: a full or abbreviated id, or any revision.
    Commit { id: String },
}

/// A base as resolved, with what to show of it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Base {
    /// A commit is reported by its full id.
    pub choice: BaseChoice,
    /// The base commit.
    pub commit: CommitInfo,
    /// What to call the commit: the branch if it is a branch tip, else
    /// `branch~N` on a local branch's first-parent history (the chosen branch,
    /// the current one, the default one, then the rest), else its short id.
    pub name: String,
    /// How many commits a chosen branch has gained since the base was chosen;
    /// absent unless the choice is a branch that has moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub behind: Option<u32>,
}

/// What the changes lead to: the live working tree.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Target {
    /// "Working tree".
    pub name: String,
    /// The checked-out branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
}

/// Whether a review has started. It starts with the first review action (a
/// mark, an acknowledgement, a new thread) and ends with `FinishReview`;
/// before and after, the session is browsing: the live diff, nothing kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ReviewInfo {
    Browsing,
    Open {
        /// Of the current snapshot: 1 is the review's start snapshot, each
        /// later settled snapshot adds one.
        version: u32,
        /// Unix seconds the version settled.
        updated_at: u32,
    },
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
    /// Boxed: it would make every response as large as the manifest.
    Manifest(Box<Manifest>),
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
    /// `text` is Markdown; `None` with `unavailable` unset means the server has
    /// nothing to say there.
    Hover {
        revision: u32,
        side: Side,
        text: Option<String>,
        unavailable: Option<Unavailable>,
    },
    Definition(Locations),
    References(Locations),
    Diagnostics {
        revision: u32,
        path: String,
        diagnostics: Vec<Diagnostic>,
        unavailable: Option<Unavailable>,
    },
    ChangedSymbols(ChangedSymbols),
    Impact(Impact),
    RemovalImpact(RemovalImpact),
    Reachability(Reachability),
    Servers {
        servers: Vec<ServerInfo>,
    },
    /// The base after `SetBase`.
    Base(Base),
    /// After `FinishReview`: whether every change was seen when it ended.
    ReviewFinished {
        complete: bool,
    },
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
    /// The answer to `ItemBlob`.
    ItemBlob {
        content: BlobContent,
    },
    /// The answer to `Threads`, at `revision`, in creation order.
    Threads {
        revision: u32,
        threads: Vec<Thread>,
    },
    /// One thread after a change, resolved at the current revision.
    Thread(Thread),
    RunTargets {
        targets: Vec<RunTargetEntry>,
    },
    /// Nothing was started. Show `changed` (and `RunApprovalDiff` for the text)
    /// and ask "run anyway?"; approving is `Run` with `hash` as `approved_hash`.
    RunNeedsApproval {
        hash: String,
        /// No run was ever approved in this repository.
        first_run: bool,
        changed: Vec<ApprovalChange>,
    },
    /// The process runs in place. Its output arrives as `RunOutput` and its end as
    /// `RunExited`, which can reach the connection before this response.
    RunStarted {
        run_id: u32,
        /// The reconciled tree that runs, as a snapshot id.
        snapshot: String,
        /// Files that differ between that tree and the target snapshot the
        /// review last showed; the run panel says "N files changed since review"
        /// if not zero.
        changed_since_review: u32,
    },
    /// The answer to `RunApprovalDiff`, in the order of `RunNeedsApproval::changed`,
    /// except that a renamed file takes the place of the added path and the
    /// removed one is left out.
    RunApprovalDiff {
        files: Vec<ApprovalFile>,
    },
}

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

/// 0-based line and UTF-16 column.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Position {
    pub line: u32,
    pub col: u32,
}

/// End exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextSpan {
    pub start: Position,
    pub end: Position,
}

/// A symbol is identified by its file and container chain (outermost first),
/// as in §7.3: moves and renames change the identity.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolId {
    pub path: String,
    pub name: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolInfo {
    pub id: SymbolId,
    /// The whole definition, including its body.
    pub span: TextSpan,
    /// Where its name is: the position to ask `References` about.
    pub at: Position,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SemanticLocation {
    /// Repository-relative; an absolute path if `external`.
    pub path: String,
    pub span: TextSpan,
    /// Outside the repository: a dependency or toolchain source, which Vet has
    /// no snapshot of.
    pub external: bool,
}

/// `revision` and `side` are those of the request.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Locations {
    pub revision: u32,
    pub side: Side,
    pub locations: Vec<SemanticLocation>,
    pub unavailable: Option<Unavailable>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Diagnostic {
    pub span: TextSpan,
    pub severity: Severity,
    pub message: String,
    /// The tool that reported it, e.g. `rustc`.
    pub source: Option<String>,
    pub code: Option<String>,
}

/// Why a semantic answer is missing, so the shell can say so instead of
/// showing nothing (§7.1: a language without a usable server degrades).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Unavailable {
    pub kind: UnavailableKind,
    /// Fit to show.
    pub message: String,
    /// The language whose server is the problem, as `ListServers` names it;
    /// absent for `Unsupported`.
    pub language: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UnavailableKind {
    /// No language is configured for this kind of file.
    Unsupported,
    /// No server was found for the language: see `ListServers`.
    ServerMissing,
    /// The server does not start, lacks a required operation or broke the protocol.
    ServerFailed,
    /// The server would execute project code, which the review does not allow.
    AnalysisLimited,
    /// The base snapshot cannot be rebuilt (§6.1).
    SnapshotUnavailable,
    /// The server failed this query.
    QueryFailed,
}

/// What a derived result was computed from (I6: derived results show their inputs).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DerivedInputs {
    pub analysis: String,
    pub bounds: Option<DerivedBounds>,
    /// The primitive queries consulted, e.g. `incoming_calls(target:src/a.rs:3:4)`.
    pub primitives: Vec<String>,
    /// The diff hunks consulted, e.g. `src/a.rs: old 3..5 new 3..6`.
    pub hunks: Vec<String>,
    /// Configuration that shaped the result, e.g. the entry points.
    pub config: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DerivedBounds {
    pub max_depth: u32,
    pub max_nodes: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SymbolChange {
    Added,
    Modified,
    /// Present on the base side only.
    Removed,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangedSymbol {
    /// The target-side symbol, or the base-side one if `Removed`.
    pub symbol: SymbolInfo,
    pub change: SymbolChange,
}

/// A file whose symbols could not be determined: unknown, not unchanged.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileGap {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangedSymbols {
    pub revision: u32,
    pub symbols: Vec<ChangedSymbol>,
    /// Changed files no language is configured for.
    pub unsupported: Vec<String>,
    pub gaps: Vec<FileGap>,
    pub inputs: DerivedInputs,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Impact {
    pub revision: u32,
    pub symbol: SymbolInfo,
    pub references: Vec<SemanticLocation>,
    pub callers: Vec<SymbolInfo>,
    /// Queries that failed; the matching list is unknown, not empty.
    pub gaps: Vec<String>,
    pub inputs: DerivedInputs,
}

/// What became of one reference to a removed symbol, judged in the target.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CallFate {
    /// Its lines were deleted.
    Gone,
    /// Its lines were edited.
    Modified,
    /// Unchanged text that no longer resolves.
    LikelyBroken,
    /// Unchanged text that now resolves elsewhere.
    BindingChanged {
        now: Vec<SemanticLocation>,
    },
    Unknown {
        reason: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CallSite {
    /// Where the reference is on the base side.
    pub base: SemanticLocation,
    pub fate: CallFate,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemovalImpact {
    pub revision: u32,
    pub symbol: SymbolInfo,
    pub call_sites: Vec<CallSite>,
    /// Set when the references could not be listed: the call sites are unknown, not absent.
    pub gap: Option<String>,
    pub inputs: DerivedInputs,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Reach {
    /// The path from the changed symbol to the entry point.
    Reached { path: Vec<SymbolId> },
    /// No entry point within the bounds and entry points in `inputs`.
    UnreachedWithinBound,
    /// Backend gaps (dependency injection, reflection, serialization).
    Unknown { gaps: Vec<String> },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Reachability {
    pub revision: u32,
    pub symbol: SymbolInfo,
    pub reach: Reach,
    pub inputs: DerivedInputs,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServerOrigin {
    /// The path the user set.
    Custom,
    /// Part of the project: its Rust toolchain.
    Project,
    /// Installed by Vet into its data directory.
    Installed,
    /// On `PATH`.
    Path,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ServerState {
    Found {
        path: String,
        origin: ServerOrigin,
        /// If it is cheap to learn.
        version: Option<String>,
        /// Facts that shape its answers, e.g. which `tsserver` it uses.
        notes: Vec<String>,
    },
    Missing {
        /// What was looked for and where, and what to do. Fit to show.
        reason: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerInfo {
    /// `rust`, `typescript`, `python` or `csharp`.
    pub language: String,
    /// The server's name as a person knows it, e.g. "Pyright".
    pub server: String,
    pub state: ServerState,
    /// The path the user set; it is what `state` was resolved from if present.
    pub custom_path: Option<String>,
    /// The version `InstallServer` installs; `None` if Vet does not install this server.
    pub pinned_version: Option<String>,
    /// Why `InstallServer` cannot run now.
    pub install_blocker: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// What to run and how (§10).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunTarget {
    /// Stable across snapshots, so an edit made to a discovered target keeps applying.
    pub id: String,
    /// The manifest it was discovered from, relative to the repository root;
    /// `None` for a target the user added. The backend ignores it in
    /// `SaveRunTarget`: discovery decides.
    pub source: Option<String>,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    /// Relative to the repository root; empty for the root.
    pub cwd: String,
    pub env: Vec<EnvVar>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunTargetEntry {
    pub target: RunTarget,
    /// The user edited or added it: it is stored, and `ResetRunTarget` applies.
    pub stored: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunChange {
    Added,
    Modified,
    Removed,
}

/// An executor-relevant file that changed since the last approved run.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApprovalChange {
    pub path: String,
    pub change: RunChange,
}

/// One changed file of an approval prompt. A side is `None` if the file did not
/// exist there, has no text (binary) or is over the capture cap.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApprovalFile {
    pub path: String,
    /// The old path if git detected a rename; `base` is then the text there, and
    /// the removed file has no entry of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,
    pub change: RunChange,
    /// The last approved version.
    pub base: Option<String>,
    /// The version about to run.
    pub target: Option<String>,
    /// Line hunks between the two, as in `FileChange`.
    pub hunks: Vec<Hunk>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunStream {
    Stdout,
    Stderr,
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
    /// The first review action started a review. Sent to every subscriber,
    /// the requester included; a finished review is `SnapshotSettled`.
    ReviewStarted { session: SessionId, review: ReviewInfo },
    /// A thread was created, replied to, resolved, re-anchored or deleted by
    /// another connection; fetch the threads again. Not sent for a settled
    /// snapshot, which is `SnapshotSettled`: that also invalidates the threads.
    ThreadsChanged { session: SessionId, revision: u32 },
    /// A line of output of an `InstallServer` in progress, to the connections of
    /// the session it was asked on.
    ServerInstallOutput {
        session: SessionId,
        language: String,
        line: String,
    },
    /// Another connection installed a server or changed a server path: what
    /// `ListServers` says may have changed.
    ServersChanged { session: SessionId },
    /// Output of a run, in order per stream. `bytes` is base64 (standard alphabet,
    /// padded) of the raw bytes: process output is not necessarily UTF-8 and a
    /// chunk can end inside a multi-byte character, so a client decodes and joins
    /// the chunks of a stream before decoding text.
    RunOutput {
        session: SessionId,
        run_id: u32,
        stream: RunStream,
        bytes: String,
    },
    /// The last event of a run. `code` is `None` if a signal ended the process,
    /// as `StopRun` does. A run sends a bounded amount of output; the process is
    /// not slowed, but output past the bound is dropped and `dropped_bytes` counts
    /// it (saturating).
    RunExited {
        session: SessionId,
        run_id: u32,
        code: Option<i32>,
        dropped_bytes: u32,
    },
    /// A dependency restore failed, so analysis is limited. `failures` is every
    /// failure of the session so far, as in `Manifest::restore_failures`.
    RestoreFailed {
        session: SessionId,
        failures: Vec<RestoreFailure>,
    },
}

/// A dependency restore step that failed (§7.2): what that project's analysis
/// cannot resolve is missing, not wrong.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreFailure {
    /// The project directory, relative to the repository with `/`; `.` is the root.
    pub project: String,
    /// The tool that failed, e.g. `pnpm`.
    pub tool: String,
    /// Fit to show.
    pub message: String,
}

/// The changeset as displayed through the repository's review filter.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Manifest {
    pub session: SessionId,
    /// Increases whenever the changeset's target changes.
    pub revision: u32,
    pub name: String,
    pub review: ReviewInfo,
    /// What the files are compared against.
    pub base: Base,
    /// What the files lead to.
    pub target: Target,
    pub filter: ReviewFilter,
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
    /// Dependency restores that failed so far; the shell says analysis is limited
    /// (§7.5). `Event::RestoreFailed` updates it.
    #[serde(default)]
    pub restore_failures: Vec<RestoreFailure>,
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
