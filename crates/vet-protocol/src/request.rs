//! What a client asks for.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    BackendSettings, ChangesetKey, CommentAt, EditChoice, Lens, NonTextKind, PipelineAction, ProjectId, ReviewFilter,
    SearchPattern, SearchScope, SearchSide, Secret, Side, SymbolId, ThreadStatus, ThreadTarget, View,
};

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Request {
    /// Round-trip probe, used to measure transport latency.
    Ping {
        payload: String,
    },
    /// The recently opened repositories, as the paths they were recorded with and
    /// when, most recent first. They belong to the backend, not to a project. Subscribes
    /// this connection to the events of the backend (`RecentProjectsChanged`,
    /// `AccountsChanged`, `CredentialsNeeded`). Answers with `Response::RecentProjects`.
    RecentProjects,
    /// Puts `path` at the top of the recent list, opened now, dropping its earlier
    /// entry and the oldest ones beyond ten. Every connection subscribed to the events of
    /// the backend gets `RecentProjectsChanged`. Answers with `Response::RecentProjects`.
    RecordRecentProject {
        path: String,
    },
    /// Removes `path` from the recent list, like `RecordRecentProject`. Answers with
    /// `Response::RecentProjects`.
    ForgetRecentProject {
        path: String,
    },
    /// The credentials the backend holds or can get for hosting platforms, per host
    /// and not tied to a project, and subscribes this connection to the events of the
    /// backend, as `RecentProjects` does.
    /// Asking also looks up what the platform says about each credential, so it may
    /// take a moment. Answers with `Response::Accounts`.
    Accounts,
    /// Gives the backend a personal access token for `host` (`github.com`, `dev.azure.com`,
    /// a GitHub Enterprise host); it keeps it in memory only. With `validate` the platform
    /// is asked first, and a token it refuses, or a platform that cannot be reached,
    /// fails the request with a message fit to show and nothing is kept. Without it the
    /// token is taken as is: this is how the shell hands back what its keychain holds.
    /// Unless the token is the one held already and was not to be checked, every connection
    /// subscribed to the events of the backend gets `AccountsChanged`, and the open pull
    /// requests of the host are synced again. Answers with `Response::Account`, whose `host`
    /// is the normalized one (lower case, without a scheme or a path) and what the shell
    /// stores the token under.
    SetCredentials {
        host: String,
        token: Secret,
        validate: bool,
    },
    /// Forgets the personal access token of `host`. A credential of a CLI tool is not
    /// the backend's to remove. Announced and synced like `SetCredentials`. Answers with
    /// `Response::Accounts`.
    RemoveCredentials {
        host: String,
    },
    /// "Clear semantic caches": removes the answers of the language servers that the
    /// backend keeps (in memory and between launches) and the kept copies of snapshots
    /// that no running Vet uses. Only speed is lost; review state and results stay as
    /// they are. They belong to the backend, not to a project. Answers with
    /// `Response::SemanticCacheUsage` as after clearing.
    ClearSemanticCaches,
    /// How much disk the answers of the language servers kept between launches take. They
    /// belong to the backend, not to a project. Answers with `Response::SemanticCacheUsage`.
    SemanticCacheUsage,
    /// The backend's own settings. Answers with `Response::Settings`.
    Settings,
    /// Replaces the backend's own settings and keeps them across launches. A smaller
    /// semantic cache budget removes the answers used least recently at once. Answers with
    /// `Response::Settings`.
    SetSettings {
        settings: BackendSettings,
    },
    /// Where the backend writes its log files, as a path of the machine it runs on. They belong to
    /// the backend, not to a project. Answers with `Response::LogDirectory`.
    LogDirectory,
    /// Opens the working tree of the repository containing `repo`, which may be
    /// any folder inside it, against its current branch, and subscribes this
    /// connection to the project's events. The repository's open review is
    /// resumed; with none, the project is browsing (`ReviewInfo`). Fails if
    /// `repo` is not inside a non-bare repository.
    OpenProject {
        repo: String,
    },
    CloseProject {
        project: ProjectId,
    },
    /// The changesets the project offers: the working tree first, then every local branch, then
    /// the remote-tracking branches that have commits the default branch lacks and were committed
    /// to in the last 30 days (and always the one on view and any with a review open), each group
    /// most recently committed first. Answers with `Response::Changesets`.
    ListChangesets {
        project: ProjectId,
    },
    /// The open pull requests of the project's GitHub and Azure DevOps remotes, most recently
    /// updated first. Asks the platform, so it takes a moment and is
    /// separate from `ListChangesets`; a repository the platform cannot answer for is named in
    /// `sources` with the reason, a missing credential also as `CredentialsNeeded`, and the other
    /// repositories still list. Answers with `Response::PullRequests`.
    ListPullRequests {
        project: ProjectId,
    },
    /// The lines the changeset changes against its base, for a row of the list: the delta between
    /// the commits of its base and its tip, kept per pair of commits. Absent for the working tree,
    /// whose manifest is live, and for a changeset whose tip is not here (a pull request that was
    /// not fetched). May take a moment for a large changeset, so ask per row. Answers with
    /// `Response::ChangesetLines`.
    ChangesetLines {
        project: ProjectId,
        changeset: ChangesetKey,
    },
    /// The checks (CI runs and statuses) the platform has of each commit, in the repository of the
    /// pull request on view; none for any other changeset. Asks the platform, which keeps what
    /// it has learnt of a commit whose checks are all done and asks again for one that has some
    /// running, so call it again while a commit's state is `Pending`. A commit the platform
    /// cannot answer for is left out, and why is in `error` beside the others; a missing
    /// credential is also announced as `CredentialsNeeded`. Answers with `Response::Checks`.
    Checks {
        project: ProjectId,
        commits: Vec<String>,
    },
    /// The log of a run of `Checks` that has one, as lines of text without the platform's
    /// timestamps, and where it failed. Fails with a message fit to show. Answers with
    /// `Response::CheckLog`.
    CheckLog {
        project: ProjectId,
        run: String,
    },
    /// The steps of a job of `Checks` that belongs to a `Pipeline`, in the order they run. Fails
    /// with a message fit to show. Answers with `Response::PipelineSteps`.
    PipelineSteps {
        project: ProjectId,
        check: String,
    },
    /// Asks the platform to re-run or stop a run of `Checks`; none for a changeset that is no
    /// pull request. The platform does its work later, so ask for the checks again. A refusal of
    /// the token, and a limit the platform asks to wait out, are in the answer; any other
    /// failure is a message fit to show. Answers with `Response::PipelineChanged`.
    ChangePipeline {
        project: ProjectId,
        action: PipelineAction,
    },
    /// Puts a changeset on view in place of the one that is: one is on view per project.
    /// Every subscriber, including the requester, gets `ChangesetOpened`; fetch the
    /// manifest again. A branch whose tip or merge base moved since it was last on view
    /// gets a new version if its review is open. Opening the changeset on view looks at it
    /// again and announces only if it moved. Fails with a message fit to show if the
    /// branch does not exist. A pull request is fetched first (head and target branch, as a
    /// task), so the request waits for the network; it fails with a message fit to show if the
    /// platform or the fetch does, and a missing credential is also announced as
    /// `CredentialsNeeded`. Opening the pull request on view again announces `TargetChanged` if
    /// the platform names it differently now. A push to a pull request on view that a later
    /// fetch brings is a new version, announced as `SnapshotSettled`. Answers with
    /// `Response::Done`.
    OpenChangeset {
        project: ProjectId,
        changeset: ChangesetKey,
    },
    /// Ends the open review: its marks, acknowledgements and threads are kept
    /// in the store but leave the project, which goes back to browsing. The
    /// revision increases and every subscriber, including the requester, gets
    /// `SnapshotSettled`; fetch the manifest again. `ChangesetsChanged` follows, as the
    /// changeset's row no longer has a review open. Fails if no review is open.
    /// Answers with `Response::ReviewFinished`.
    FinishReview {
        project: ProjectId,
    },
    /// The background work of the project that is running now, oldest first, for
    /// a connection that joined after it started. Answers with `Response::Tasks`,
    /// also while the project is still scanning for the first time.
    Tasks {
        project: ProjectId,
    },
    /// The versions of the open review, oldest first; none while browsing, as versions are
    /// recorded only while a review is open. Answers with `Response::Versions`.
    Versions {
        project: ProjectId,
    },
    /// The commits of the changeset on view, oldest first, each with its own diff and progress,
    /// and the lens the changeset is reviewed in. The working tree has none. Answers with
    /// `Response::Commits`.
    Commits {
        project: ProjectId,
    },
    /// Remembers the lens a branch changeset is reviewed in; every subscriber, including the
    /// requester, gets `LensChanged`. Fails for the working tree. Answers with `Response::Done`.
    SetLens {
        project: ProjectId,
        lens: Lens,
    },
    /// The changeset as it is now, with review state, an interdiff of two of its versions, or
    /// one of its commits (see [`View`]). Waits for the project's first scan if it has not
    /// finished.
    Manifest {
        project: ProjectId,
        view: View,
    },
    /// Both sides of one text file at the current revision.
    FileContents {
        project: ProjectId,
        view: View,
        path: String,
    },
    /// Flips one hunk (an index into the manifest of `revision`): seen becomes unseen, an
    /// unseen one or one seen against an old base becomes seen. Fails if `revision` is no
    /// longer current. In an interdiff the hunk is marked in the changeset of the newer
    /// version (which must be the latest), and a hunk with nothing left to review cannot be
    /// marked. Answers with `Response::Review`, in the terms of `view`; every subscriber,
    /// the requester included, gets `ReviewChanged` of the changeset's file.
    ToggleHunk {
        project: ProjectId,
        view: View,
        revision: u32,
        path: String,
        hunk: u32,
    },
    /// Flips the flag of one hunk for later review (an index into the manifest of `revision`).
    /// A flag is independent of the hunk being seen and only goes away with this request.
    /// Fails like `ToggleHunk`. Answers with `Response::Review`, and every subscriber gets
    /// `ReviewChanged`.
    ToggleFlag {
        project: ProjectId,
        view: View,
        revision: u32,
        path: String,
        hunk: u32,
    },
    /// Flips the acknowledgement of one non-text item. Fails if `revision` is no longer
    /// current. Answers with `Response::Review`.
    ToggleItem {
        project: ProjectId,
        view: View,
        revision: u32,
        path: String,
        kind: NonTextKind,
    },
    /// Marks everything of a file seen: all its hunks and non-text items. If it all
    /// is seen already, takes every mark back instead. Fails if `revision` is no
    /// longer current. Answers with `Response::Review`.
    ToggleFileSeen {
        project: ProjectId,
        view: View,
        revision: u32,
        path: String,
    },
    /// Marks everything of each of the files seen (`seen`), or takes every mark back, whatever
    /// their states are: all their hunks and non-text items, recorded together. Fails if
    /// `revision` is no longer current or a path is not a file of `view`. Answers with
    /// `Response::Reviews`, one update for each path in order; every subscriber gets
    /// `ReviewChanged` of each changeset file.
    SetFilesSeen {
        project: ProjectId,
        view: View,
        revision: u32,
        paths: Vec<String>,
        seen: bool,
    },
    /// Marks the hunks of a file that are seen against an old base seen again, against the
    /// base now; its other hunks stay as they are. Fails if `revision` is no longer current
    /// or the file has no such hunk. Answers with `Response::Review`.
    ReconfirmFile {
        project: ProjectId,
        revision: u32,
        path: String,
    },
    /// The base-side lines of a hunk seen against an old base (an index into the manifest of
    /// `revision`) as they were when it was marked, as a diff to what the base has there now.
    /// Fails if `revision` is no longer current or the hunk is in another state. Only the
    /// changeset has such hunks to show. Answers with `Response::BaseDiff`.
    BaseDiff {
        project: ProjectId,
        revision: u32,
        path: String,
        hunk: u32,
    },
    /// Hover at a position of one side of the changeset at `revision`. Positions
    /// are 0-based lines and UTF-16 columns, in the text of that side. Fails if
    /// `revision` is no longer current; a language without a usable server
    /// answers with `unavailable` set instead. Answers with `Response::Hover`.
    Hover {
        project: ProjectId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
    },
    /// Where the symbol at the position is defined. The locations are ordered so that
    /// the first is the one to go to: those inside the repository, which can be opened,
    /// come before the others. Answers with `Response::Definition`; otherwise like `Hover`.
    Definition {
        project: ProjectId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
    },
    /// The references to the symbol at the position, without its declaration, as rows to list
    /// (see `LocationRows`). `limit` is the most rows to answer with, a default one if absent;
    /// `LocationRows::total` says how many there were. Answers with `Response::References`;
    /// otherwise like `Hover`.
    References {
        project: ProjectId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        limit: Option<u32>,
    },
    /// The implementations of the interface, trait or abstract member at the position, as rows to
    /// list. A server that cannot answer makes `unavailable` set. Answers with
    /// `Response::Implementations`; otherwise like `Hover`.
    Implementations {
        project: ProjectId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
    },
    /// The diagnostics of a file of the target side. The language server is
    /// asked and may take seconds on a cold start: there is no push.
    /// With `against_base` each carries its origin from comparing with the base side's, which
    /// may take far longer; without it every origin is pending.
    /// Answers with `Response::Diagnostics`; otherwise like `Hover`.
    Diagnostics {
        project: ProjectId,
        revision: u32,
        path: String,
        against_base: bool,
    },
    /// What the changeset did to the symbols of one changed file (derived, §7.3): the file's
    /// symbols on both sides, from their texts, intersected with its diff hunks. A file no
    /// language server handles has none. Fails if `revision` is no longer current or `path` is
    /// not a changed file. Answers with `Response::FileSymbols`.
    FileSymbols {
        project: ProjectId,
        revision: u32,
        path: String,
    },
    /// Every symbol of the file `path` as `view` shows it (derived): in the changeset, a changed
    /// file's symbols of both sides with what the change did to each (a removed symbol is listed
    /// where it stood), any other file's from its target text; in a commit or an interdiff, the
    /// symbols of the file as the commit leaves it (the changeset's target for an interdiff),
    /// with no marks. A language without a usable server answers with `unavailable` set and no
    /// symbols. Fails if `revision` is no longer current. Answers with `Response::FileOutline`.
    FileOutline {
        project: ProjectId,
        view: View,
        revision: u32,
        path: String,
    },
    /// The symbols of the target side whose names match `query`, from the language server of each
    /// language that has a file in the project (`workspace/symbol`; a server matches and caps its
    /// answer in its own way, rust-analyzer at 128). A language whose server cannot answer is
    /// reported with its reason and the others still answer. Cancellable, like every semantic
    /// request: a new query overtakes the one before. Fails if `revision` is no longer current.
    /// Answers with `Response::WorkspaceSymbols`.
    WorkspaceSymbols {
        project: ProjectId,
        revision: u32,
        query: String,
    },
    /// The innermost symbol (a function, type, module, field or constant) that holds a position of
    /// one side of the changeset, and what became of it (derived). The position is in the text of that side,
    /// 0-based lines and UTF-16 columns. A language without a usable server answers with
    /// `unavailable` set and no symbol. Fails if `revision` is no longer current. Answers with
    /// `Response::SymbolAt`.
    SymbolAt {
        project: ProjectId,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
        col: u32,
    },
    /// The declaration of a symbol, from the language server, on the side the symbol exists on
    /// (the base for a removed one). Fails if `revision` is no longer current or the symbol is
    /// in no outline of its file. Answers with `Response::Signature`.
    Signature {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// The changed callers of a symbol of the target side, followed through changed symbols
    /// only, its changed callees and, if it changed, its unchanged direct callers (derived).
    /// Fails like `Signature`, and for a removed symbol. Answers with `Response::Calls`.
    Calls {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// The references to a symbol of the target side (derived). Fails like `Calls`. Answers
    /// with `Response::Usages`.
    Usages {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// What became of the references to a removed symbol (derived). Fails if the
    /// symbol is not a removed one of its file at `revision`. Answers
    /// with `Response::RemovalImpact`.
    RemovalImpact {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// Whether a symbol of the target side is reached from an entry
    /// point (derived, §7.4). Fails like `Calls`. Answers with `Response::Reachability`.
    Reachability {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// Each language's server as it resolves for the project's repository.
    /// Answers with `Response::Servers`.
    ListServers {
        project: ProjectId,
    },
    /// Installs the pinned server of a language into Vet's data directory and
    /// restarts that language's running servers. Output lines arrive as
    /// `ServerInstallOutput` events until the answer, which is `Response::Servers`
    /// as after the install. Fails with the tool's output tail if it does not work.
    InstallServer {
        project: ProjectId,
        language: String,
    },
    /// Stores the path of the server to use for a language, or removes it with
    /// `None`. The path is not checked here: `ListServers` says whether it works.
    /// Restarts that language's running servers. Answers with `Response::Servers`.
    SetServerPath {
        project: ProjectId,
        language: String,
        path: Option<String>,
    },
    /// Stops the project's servers of a language at once, requests in flight on them
    /// included (those fail), and starts them anew, as the project's languages need.
    /// A server that was missing or failed to start is tried again. The project's
    /// subscribers get `ServersChanged`. Answers with `Response::Servers` once the new
    /// servers are running; fails with why a server did not start.
    RestartServer {
        project: ProjectId,
        language: String,
    },
    /// Says whether this connection shows the review screen of the project, which is what the
    /// language servers of the changeset on view run for. While no connection does, a changeset
    /// that comes on view, a revision that settles, a fetch or a change of trust starts no
    /// server and no restore; showing the screen starts the servers the changeset on view
    /// needs, and later changes of the revision start theirs. A connection that closes the
    /// project or goes away has left the screen. Answers with `Response::Done`.
    SetReviewShown {
        project: ProjectId,
        shown: bool,
    },
    /// Fetches the repository's remotes now, as a `Fetch` task, and answers with
    /// `Response::Done` when it ended. A remote that fails (offline, refused) fails the
    /// task with its name, not this request, and the others are still fetched. If any
    /// remote was fetched, every subscriber then gets `ChangesetsChanged`,
    /// a pull request on view whose head moved is its next version (`SnapshotSettled`), and
    /// its threads are synced.
    Fetch {
        project: ProjectId,
    },
    /// Replaces the review filter of the repository `project` has open and
    /// persists it: a project setting, not part of the review (`project` is
    /// only the handle to its repository). The revision increases and every
    /// subscriber, including the requester, gets `SnapshotSettled`; fetch the
    /// manifest again. Fails with a message fit to show if a glob is invalid, and
    /// then changes nothing. Answers with `Response::Done`.
    SetProjectFilter {
        project: ProjectId,
        filter: ReviewFilter,
    },
    /// What each include and exclude glob of the draft `filter` matches among the changed files of
    /// the changeset on view: files and lines, whatever else the filter does. A glob that is
    /// invalid is answered, not failed on. Changes nothing. Answers with
    /// `Response::PatternMatches`.
    PatternMatches {
        project: ProjectId,
        filter: ReviewFilter,
    },
    /// Narrows the review on view to `filter`, which replaces the project's filter for this
    /// review only (`None` ends the narrowing). Nothing is stored: the narrowing ends with a
    /// change to the project's filter and when another changeset comes on view. Announces like
    /// `SetProjectFilter`. Fails with a message fit to show if a glob is invalid, and then
    /// changes nothing. Answers with `Response::Done`.
    NarrowReview {
        project: ProjectId,
        /// The changeset the viewer narrows; a request made after another came on view is ignored.
        changeset: ChangesetKey,
        filter: Option<ReviewFilter>,
    },
    /// Marks the repository `project` has open as trusted, or not (§7.5): a project setting, as
    /// the review filter is. A trusted project's changesets are analysed with restores and
    /// language servers that execute project code (pull requests from a fork excepted).
    /// Servers restart under the new policy: every subscriber, including the requester, gets
    /// `TargetChanged` (fetch the manifest again, whose `trust` says) and `ServersChanged`.
    /// Answers with `Response::Done`.
    SetProjectTrusted {
        project: ProjectId,
        trusted: bool,
    },
    /// Trusts the pull request on view at its current head commit, so its analysis may execute
    /// project code until it is pushed to. Announces like `SetProjectTrusted`. Fails if the
    /// changeset on view is no pull request. Answers with `Response::Done`.
    TrustPullRequest {
        project: ProjectId,
    },
    /// Adds the file at `path`, or with `directory` everything below the folder at `path`
    /// (`a/b` without a trailing slash), to the excluded globs of the repository's review
    /// filter and persists it, like `SetProjectFilter`. Answers with `Response::Done`.
    ExcludePath {
        project: ProjectId,
        path: String,
        directory: bool,
    },
    /// The direct children of a directory of the current target snapshot.
    /// `path` is `""` for the root, else `a/b` without a trailing slash. Fails if
    /// the snapshot has nothing there.
    ListDirectory {
        project: ProjectId,
        path: String,
    },
    /// The path of every file of the current target snapshot (not a symlink or a submodule; no
    /// file a `.gitignore` leaves out), in byte order, to search them by name. Answers with
    /// `Response::Files`.
    ListFiles {
        project: ProjectId,
    },
    /// The text of any path on the target side of `view`, changed or not: the file as the
    /// changeset ends with it, or as the commit leaves it for a commit's view (an interdiff
    /// reads the changeset's). Fails if the file is not there.
    FileText {
        project: ProjectId,
        view: View,
        path: String,
    },
    /// The lines of the files in `scope` that `pattern` matches, grouped by file in path order.
    /// `mask` limits the files: globs separated by commas, a leading `!` excludes (`*.rs, !tests/**`),
    /// a glob without `/` matching the file name at any depth; empty takes every file. `side`,
    /// `only_changes` (leave out unchanged lines) and `not_seen` (only changed lines of hunks that
    /// are not seen; it implies `only_changes`) narrow the lines. The search stops after `limit`
    /// matches (`truncated`). Fails with a message fit to show if the pattern or the mask is
    /// invalid. Answers with `Response::SearchResults`.
    Search {
        project: ProjectId,
        scope: SearchScope,
        pattern: SearchPattern,
        mask: String,
        side: SearchSide,
        only_changes: bool,
        not_seen: bool,
        limit: u32,
    },
    /// The bytes of one side of a non-text item's file, to preview an image. The
    /// item is found by `path` and `kind` as in `ToggleItem`; `Base` is the file
    /// before (at its old path for a rename), `Target` the file after. Fails if
    /// `revision` is no longer current or there is no such item. Answers with
    /// `Response::ItemBlob`.
    ItemBlob {
        project: ProjectId,
        view: View,
        revision: u32,
        path: String,
        kind: NonTextKind,
        side: Side,
    },
    /// Every thread with its anchor resolved at the current revision. Resolution
    /// is lazy: a settled snapshot only invalidates it, and this request computes
    /// it. Kept apart from `Manifest` so the manifest does not wait for it.
    /// An interdiff shows the range threads on the new side of its newer version, and none if
    /// that is not the latest version.
    Threads {
        project: ProjectId,
        view: View,
    },
    /// The line of the target text that a unified diff shows what is on `line` of one side of
    /// the file at `path` (its old path on the base side) at: the same rule as
    /// `Thread::target_line`, for a comment that has no thread yet and for a base position to
    /// reveal. Fails if `revision` is no longer current. Answers with `Response::TargetLine`.
    TargetLine {
        project: ProjectId,
        view: View,
        revision: u32,
        side: Side,
        path: String,
        line: u32,
    },
    /// Where the lines (0-based) of the target text of `path` in version `from` of the open
    /// review are in the target text now, for rows found in an earlier text. Fails if no review
    /// is open, there is no such version, or the file has no text in the version or now.
    /// Answers with `Response::MappedLines`, one for each line, in order.
    MapLines {
        project: ProjectId,
        from: u32,
        path: String,
        lines: Vec<u32>,
    },
    /// What a comment on a part of the changeset file `path` (its current path) attaches
    /// to: the side and the path on that side. A file is commented on at its target
    /// side, a deleted one at its base; a hunk at its target lines, a pure deletion at
    /// its base lines; lines at the side the request names. Fails if `revision` is no
    /// longer current, or if the file or hunk does not exist. Answers with
    /// `Response::CommentTarget`, whose target goes to `CreateThread`.
    CommentTarget {
        project: ProjectId,
        view: View,
        revision: u32,
        path: String,
        at: CommentAt,
    },
    /// Opens a thread with its first message. The lines of a range target are in
    /// the text of `revision`. Fails if `revision` is no longer current, or if the
    /// target does not exist at it. In an interdiff a comment goes on the new side of the
    /// latest version only; anything else fails. Answers with `Response::Thread`.
    CreateThread {
        project: ProjectId,
        view: View,
        revision: u32,
        target: ThreadTarget,
        body: String,
    },
    /// Answers with `Response::Thread`.
    Reply {
        project: ProjectId,
        thread: u32,
        body: String,
    },
    /// Resolves or reopens a thread. Answers with `Response::Thread`.
    SetThreadStatus {
        project: ProjectId,
        thread: u32,
        status: ThreadStatus,
    },
    /// Deletes a thread; this is also how a detached one is discarded. A thread the platform
    /// has is removed there too, which only its author can; the request fails with a message
    /// fit to show for anyone else's, or when the platform cannot do it.
    DeleteThread {
        project: ProjectId,
        thread: u32,
    },
    /// Replaces the text of one of the viewer's own comments. A draft changes here. A comment
    /// the platform has is changed there at once; if its text there is not the one this edit
    /// started from, nothing is sent and the message carries both texts as a `conflict` until
    /// `KeepEdit` chooses one. A comment the platform cannot be told about now fails the request
    /// with the reason; the new text is kept and the next sync sends it. Answers with
    /// `Response::Thread`.
    EditMessage {
        project: ProjectId,
        message: u32,
        body: String,
    },
    /// Removes one of the viewer's own comments; removing a thread's first removes the thread.
    /// A comment the platform has is removed there first. A draft or a failed one goes locally;
    /// one that is being posted fails the request. Answers with `Response::Done`.
    RemoveMessage {
        project: ProjectId,
        message: u32,
    },
    /// Settles a comment that was edited here and on the platform by keeping one of the texts.
    /// Answers with `Response::Thread`.
    KeepEdit {
        project: ProjectId,
        message: u32,
        keep: EditChoice,
    },
    /// Replaces the text of one of the viewer's comments in the conversation of the pull request
    /// on view (`ConversationKind::Comment` with `mine`), on the platform at once. Fails with the
    /// reason if the platform does not take it, or for anyone else's comment. Answers with
    /// `Response::Done`.
    EditConversationComment {
        project: ProjectId,
        comment: String,
        body: String,
    },
    /// Deletes one of the viewer's comments in the conversation of the pull request on view, on
    /// the platform at once. Answers with `Response::Done`.
    DeleteConversationComment {
        project: ProjectId,
        comment: String,
    },
    /// Brings the threads of the pull request on view up to date with the platform's, both ways
    /// (a status the viewer changed since the last sync is sent and wins), then announces
    /// `ThreadsChanged`, `PlatformStatus` and any `Notification` to every subscriber. The backend
    /// does this on its own when a pull request opens, on fetch and after `SubmitReview`; this is
    /// for after the viewer signed in again. Fails if the changeset on view is no pull request.
    /// A platform that cannot be asked is a `PlatformStatus`, not a failure. Answers with
    /// `Response::Done`.
    SyncThreads {
        project: ProjectId,
    },
    /// Posts the viewer's drafts of the pull request on view as one review with `verdict` (an id
    /// from `PlatformTraits::verdicts`) and an optional text, then syncs. Nothing of the review
    /// is posted if the platform refuses it, and the request fails with the reason. Answers with
    /// `Response::ReviewSubmitted`.
    SubmitReview {
        project: ProjectId,
        verdict: String,
        body: Option<String>,
    },
    /// Posts the viewer's drafts of the pull request on view one after the other, on a platform
    /// that takes comments one at a time (`PlatformTraits::batched_review` is false), then the
    /// pending vote, then syncs. Posting stops at the first draft the platform refuses: those
    /// before it stay posted, the rest stay drafts (and the vote is not sent), and the answer says
    /// why. A vote the platform refuses is marked failed with its reason. Answers with
    /// `Response::ReviewSubmitted`.
    PostDrafts {
        project: ProjectId,
    },
    /// Stages the viewer's vote on the pull request on view, on a platform that keeps the vote
    /// apart from posting (`PlatformTraits::verdict_separate`); `verdict` is an id from
    /// `PlatformTraits::verdicts`. The vote is stored with the pull request and sent by
    /// `PostDrafts`. It replaces a vote staged before; the viewer's current vote stages nothing and
    /// clears the staged one. Announces `ThreadsChanged`. Answers with `Response::Done`.
    StageVote {
        project: ProjectId,
        verdict: String,
    },
    /// Drops the vote staged on the pull request on view. Fails while it is being posted.
    /// Announces `ThreadsChanged`. Answers with `Response::Done`.
    DiscardVote {
        project: ProjectId,
    },
    /// What the Comments tool window shows of the pull request on view above its threads, as
    /// the last sync found it. Fails if the changeset on view is no pull request or was not
    /// synced yet. Answers with `Response::PullRequestInfo`.
    PullRequestInfo {
        project: ProjectId,
    },
    /// Re-anchors a thread whose anchor is `Inferred` at its current location,
    /// which makes it `Exact`. Fails if `revision` is no longer current or the
    /// anchor is not `Inferred`. Answers with `Response::Thread`.
    ConfirmAnchor {
        project: ProjectId,
        revision: u32,
        thread: u32,
    },
    /// Re-anchors a thread whose anchor is `Ambiguous` at one of its candidates
    /// (an index into the candidates of `revision`). Fails like `ConfirmAnchor`.
    /// Answers with `Response::Thread`.
    AttachAnchor {
        project: ProjectId,
        revision: u32,
        thread: u32,
        candidate: u32,
    },
}
