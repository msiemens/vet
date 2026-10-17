//! What the backend answers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Account, AppStorage, BackendSettings, BaseDiff, BlobContent, Calls, ChangesetKey, ChangesetRow, CheckLog,
    CommitChecks, CommitUnit, Diagnostic, DirEntry, FileOutline, FileSymbols, Highlights, Lens, LineDelta,
    LocationRows, Locations, Manifest, MappedLine, PatternMatch, PipelineOutcome, PipelineStep, Preference, ProjectId,
    ProjectStorage, PullRequestInfo, PullRequestRow, PullRequestSource, Reachability, RecentProject, RemovalImpact,
    ReviewSubmitted, ReviewUpdate, SearchFile, ServerInfo, ServerRuntimes, Serving, Side, SymbolAt, SymbolSignature,
    Task, TextChange, TextContent, Thread, ThreadTarget, Unavailable, Usages, VersionInfo, WorkspaceSymbols,
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
    /// The folder of the backend's log files.
    LogDirectory {
        path: String,
    },
    /// The bytes the semantic cache's file takes on disk.
    SemanticCacheUsage {
        #[ts(type = "number")]
        bytes: u64,
    },
    /// The projects' disk use, in root order, and what is kept for none.
    ProjectsStorage {
        projects: Vec<ProjectStorage>,
        app: AppStorage,
    },
    /// One project's disk use.
    ProjectStorage {
        project: ProjectStorage,
    },
    /// After `RemoveProject`: the bytes the project took, which are freed apart from content that
    /// is still young enough to be kept for a while.
    ProjectRemoved {
        #[ts(type = "number")]
        freed: u64,
    },
    /// The backend's own settings.
    Settings {
        settings: BackendSettings,
    },
    /// The values the window remembered in the scope asked for, by key.
    Preferences {
        values: Vec<Preference>,
    },
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
    /// The answer to `ListPullRequests`: one source per hosted repository the project has, in the
    /// order of its remotes.
    PullRequests {
        rows: Vec<PullRequestRow>,
        sources: Vec<PullRequestSource>,
    },
    /// The answer to `ChangesetLines`, one per changeset of the request, in its order.
    ChangesetLines {
        lines: Vec<Option<LineDelta>>,
    },
    /// The answer to `Checks`, in the order of the commits asked for.
    Checks {
        checks: Vec<CommitChecks>,
        /// Why the commits that are left out could not be answered for, fit to show.
        error: Option<String>,
        /// Unix seconds: the platform limits requests until then, which is why `error` is set.
        limited_until: Option<u32>,
    },
    /// The answer to `CheckLog`.
    CheckLog(CheckLog),
    /// The answer to `PipelineSteps`.
    PipelineSteps {
        steps: Vec<PipelineStep>,
    },
    /// The answer to `ChangePipeline`.
    PipelineChanged {
        outcome: PipelineOutcome,
    },
    /// The versions of the open review, oldest first.
    Versions {
        versions: Vec<VersionInfo>,
    },
    /// The answer to `ChangedSince`: the files of the changeset that differ between version `from`
    /// and the current version `to`, compared as the interdiff of the two compares them.
    ChangedFiles {
        from: u32,
        to: u32,
        files: Vec<String>,
    },
    /// The answer to `Commits`, at `revision`.
    Commits {
        revision: u32,
        lens: Lens,
        commits: Vec<CommitUnit>,
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
    /// The file's review state after `ToggleHunk`, `ToggleItem`, `ToggleFileSeen` or
    /// `ReconfirmFile`.
    Review(ReviewUpdate),
    /// The files' review states after `SetFilesSeen`, in the order of its paths.
    Reviews {
        updates: Vec<ReviewUpdate>,
    },
    /// The answer to `BaseDiff`.
    BaseDiff(BaseDiff),
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
    References(LocationRows),
    Implementations(LocationRows),
    Diagnostics {
        revision: u32,
        path: String,
        diagnostics: Vec<Diagnostic>,
        /// How many of `diagnostics` to tell the viewer about: hints are not counted.
        count: u32,
        unavailable: Option<Unavailable>,
        /// Why the origins are unknown, when the base side could not be compared.
        origin_unavailable: Option<Unavailable>,
        /// Set (to `Incomplete`) when the server was at work on its own while it gave
        /// `diagnostics`, so some may be missing; they are not kept, and asking again once the
        /// server has finished gives the rest.
        incomplete: Option<Unavailable>,
    },
    FileSymbols(FileSymbols),
    FileOutline(FileOutline),
    WorkspaceSymbols(WorkspaceSymbols),
    SymbolAt(SymbolAt),
    /// `None` with `unavailable` unset means the server has nothing to say.
    Signature {
        revision: u32,
        signature: Option<SymbolSignature>,
        unavailable: Option<Unavailable>,
    },
    Calls(Calls),
    Usages(Usages),
    RemovalImpact(RemovalImpact),
    Reachability(Reachability),
    Servers {
        servers: Vec<ServerInfo>,
        /// What the servers of the changeset on view are doing as the answer is made.
        runtimes: ServerRuntimes,
    },
    /// The answer to `ServedBy`: which server serves each path, in order.
    ServedBy {
        serving: Vec<Serving>,
    },
    /// After `FinishReview`: whether every change was seen when it ended.
    ReviewFinished {
        complete: bool,
    },
    /// Directories first, then the others, each by name in byte order. `revision`
    /// is the one the listing belongs to.
    Directory {
        revision: u32,
        entries: Vec<DirEntry>,
    },
    /// The answer to `ListFiles`: the paths, which belong to `revision`.
    Files {
        revision: u32,
        paths: Vec<String>,
    },
    /// The answer to `PatternMatches`, at `revision`: one entry per root and glob of the draft
    /// filter, in the order it lists them. `roots` holds the changed files at or below each root.
    PatternMatches {
        revision: u32,
        roots: Vec<u32>,
        include: Vec<PatternMatch>,
        exclude: Vec<PatternMatch>,
    },
    /// The answer to `FileText`, at `revision`.
    FileText {
        revision: u32,
        content: TextContent,
    },
    /// The answer to `Search`, at `revision`.
    SearchResults {
        revision: u32,
        /// The files with matches, in path order.
        files: Vec<SearchFile>,
        /// The matches in `files`.
        hits: u32,
        /// The search stopped at the limit; more matches may follow.
        truncated: bool,
        /// How many files had text to look in.
        searched: u32,
    },
    /// The answer to `ItemBlob`, at `revision`.
    ItemBlob {
        revision: u32,
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
    /// The answer to `MapLines`, at `revision`.
    MappedLines {
        revision: u32,
        lines: Vec<MappedLine>,
    },
    /// The answer to `CommentTarget`, at `revision`. For a range target, `line` is the line (0-based) of
    /// the target text that a unified diff shows the range's last line at, where a form for
    /// the comment goes (the rule of `Thread::target_line`).
    CommentTarget {
        revision: u32,
        target: ThreadTarget,
        line: Option<u32>,
    },
    /// One thread after a change, resolved at the current revision.
    Thread(Thread),
    /// The answer to `PullRequestInfo`. Boxed like the manifest, which it is rarely near in size
    /// to but need not widen every response for.
    PullRequestInfo(Box<PullRequestInfo>),
    /// The answer to `SubmitReview`.
    ReviewSubmitted(ReviewSubmitted),
}
