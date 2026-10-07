//! What the backend answers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Account, Base, BlobContent, BranchInfo, ChangedSymbols, ChangesetKey, ChangesetRow, CommitInfo, Diagnostic,
    DirEntry, Highlights, Impact, Locations, Manifest, ProjectId, Reachability, RecentProject, RemovalImpact,
    ReviewUpdate, ServerInfo, Side, SymbolSignature, Task, TextChange, TextContent, Thread, ThreadTarget, Unavailable,
};

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Response {
    Pong {
        payload: String,
    },
    Project {
        project: ProjectId,
        /// The root of the repository the opened path belongs to.
        root: String,
    },
    /// Acknowledges a request that has no result.
    Done,
    /// The recent list after the request, most recent first.
    RecentProjects {
        projects: Vec<RecentProject>,
    },
    /// The accounts after `Accounts` or `RemoveCredentials`: every credential the backend
    /// holds or can get, hosts in name order, a host's CLI credential before its personal
    /// access token.
    Accounts {
        accounts: Vec<Account>,
    },
    /// The personal access token `SetCredentials` kept, as an account.
    Account(Account),
    /// The running tasks, oldest first.
    Tasks {
        tasks: Vec<Task>,
    },
    /// The answer to `ListChangesets`; `on_view` is the changeset the manifest shows.
    Changesets {
        rows: Vec<ChangesetRow>,
        on_view: ChangesetKey,
    },
    /// Boxed: it would make every response as large as the manifest.
    Manifest(Box<Manifest>),
    /// `None` is a side the file does not have (added or deleted). The text and
    /// `changes` belong to `revision`; the hunks are in that revision's manifest.
    FileContents {
        revision: u32,
        base: Option<String>,
        target: Option<String>,
        /// The changed ranges of the hunks in order, down to the words that differ within a
        /// replaced region: line-snapped, one block per hunk (see [`TextChange`]).
        changes: Vec<TextChange>,
        /// The syntax highlighting of `base`, empty for a side without one or without a
        /// known language (see [`Highlights`]).
        base_highlights: Highlights,
        /// The same for `target`.
        target_highlights: Highlights,
    },
    /// The file's review state after `ToggleHunk`, `ToggleItem` or `ToggleFileSeen`.
    Review(ReviewUpdate),
    /// `text` is Markdown; `None` with `unavailable` unset means the server has
    /// nothing to say there. `code_highlights` has one entry for each fenced code block of
    /// `text`, in order: the highlighting of the block's code (see [`Highlights`]).
    Hover {
        revision: u32,
        side: Side,
        text: Option<String>,
        code_highlights: Vec<Highlights>,
        unavailable: Option<Unavailable>,
    },
    Definition(Locations),
    References(Locations),
    Diagnostics {
        revision: u32,
        path: String,
        diagnostics: Vec<Diagnostic>,
        /// How many of `diagnostics` to tell the viewer about: hints are not counted.
        count: u32,
        unavailable: Option<Unavailable>,
    },
    ChangedSymbols(ChangedSymbols),
    /// `None` with `unavailable` unset means the server has nothing to say.
    Signature {
        revision: u32,
        signature: Option<SymbolSignature>,
        unavailable: Option<Unavailable>,
    },
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
    /// The answer to `TargetLine`: the line (0-based) of the target text, at `revision`.
    TargetLine {
        revision: u32,
        line: u32,
    },
    /// The answer to `CommentTarget`. For a range target, `line` is the line (0-based) of
    /// the target text that a unified diff shows the range's last line at, where a form for
    /// the comment goes (the rule of `Thread::target_line`).
    CommentTarget {
        target: ThreadTarget,
        line: Option<u32>,
    },
    /// One thread after a change, resolved at the current revision.
    Thread(Thread),
}
