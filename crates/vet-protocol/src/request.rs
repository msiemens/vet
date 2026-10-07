//! What a client asks for.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    BaseChoice, ChangesetKey, CommentAt, NonTextKind, ProjectId, ReviewFilter, Secret, Side, SymbolId, ThreadStatus,
    ThreadTarget, View,
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
    /// when, most recent first. They belong to the backend, not to a project.
    /// Answers with `Response::RecentProjects`.
    RecentProjects,
    /// Puts `path` at the top of the recent list, opened now, dropping its earlier
    /// entry and the oldest ones beyond ten. Answers with `Response::RecentProjects`.
    RecordRecentProject {
        path: String,
    },
    /// Removes `path` from the recent list. Answers with `Response::RecentProjects`.
    ForgetRecentProject {
        path: String,
    },
    /// The credentials the backend holds or can get for hosting platforms, per host
    /// and not tied to a project, and subscribes this connection to `CredentialsNeeded`.
    /// Asking also looks up what the platform says about each credential, so it may
    /// take a moment. Answers with `Response::Accounts`.
    Accounts,
    /// Gives the backend a personal access token for `host` (`github.com`, `dev.azure.com`,
    /// a GitHub Enterprise host); it keeps it in memory only. With `validate` the platform
    /// is asked first, and a token it refuses, or a platform that cannot be reached,
    /// fails the request with a message fit to show and nothing is kept. Without it the
    /// token is taken as is: this is how the shell hands back what its keychain holds.
    /// Answers with `Response::Account`, whose `host` is the normalized one (lower case,
    /// without a scheme or a path) and what the shell stores the token under.
    SetCredentials {
        host: String,
        token: Secret,
        validate: bool,
    },
    /// Forgets the personal access token of `host`. A credential of a CLI tool is not
    /// the backend's to remove. Answers with `Response::Accounts`.
    RemoveCredentials {
        host: String,
    },
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
    /// The changesets the project offers: the working tree first, then the local branches
    /// that have commits the default branch lacks and were committed to in the last 30 days
    /// (and always the checked-out one, the one on view and any with a review open), the
    /// most recently committed first. Answers with `Response::Changesets`.
    ListChangesets {
        project: ProjectId,
    },
    /// The open pull requests of the project's GitHub remotes that wait for the viewer or are the
    /// viewer's own, most recently updated first. Asks the platform, so it takes a moment and is
    /// separate from `ListChangesets`; a repository the platform cannot answer for is named in
    /// `sources` with the reason, a missing credential also as `CredentialsNeeded`, and the other
    /// repositories still list. Answers with `Response::PullRequests`.
    ListPullRequests {
        project: ProjectId,
    },
    /// Puts a changeset on view in place of the one that is: one is on view per project.
    /// Every subscriber, including the requester, gets `ChangesetOpened`; fetch the
    /// manifest again. A branch whose tip or merge base moved since it was last on view
    /// gets a new version if its review is open. Opening the changeset on view looks at it
    /// again and announces only if it moved. Fails with a message fit to show if the
    /// branch does not exist. A pull request is fetched first (head and target branch, as a
    /// task), so the request waits for the network; it fails with a message fit to show if the
    /// platform or the fetch does, and a missing credential is also announced as
    /// `CredentialsNeeded`. A push to a pull request on view that a later fetch brings is a new
    /// version, announced as `SnapshotSettled`. Answers with `Response::Done`.
    OpenChangeset {
        project: ProjectId,
        changeset: ChangesetKey,
    },
    /// Ends the open review: its marks, acknowledgements and threads are kept
    /// in the store but leave the project, which goes back to browsing. The
    /// revision increases and every subscriber, including the requester, gets
    /// `SnapshotSettled`; fetch the manifest again. Fails if no review is open.
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
    /// The changeset as it is now, with review state, or an interdiff of two of its versions
    /// (see [`View`]). Waits for the project's first scan if it has not finished.
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
    /// marked. Answers with `Response::Review`, in the terms of `view`; a mark in an
    /// interdiff also reaches the other subscribers and the requester's own changeset as
    /// `ReviewChanged` of the changeset file.
    ToggleHunk {
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
    /// The references to the symbol at the position, without its declaration.
    /// Answers with `Response::References`; otherwise like `Hover`.
    References {
        project: ProjectId,
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
        project: ProjectId,
        revision: u32,
        path: String,
    },
    /// The symbols the changeset touches (derived, §7.3): both sides of every
    /// changed file, intersected with the diff hunks. Fails if `revision` is no
    /// longer current. Answers with `Response::ChangedSymbols`.
    ChangedSymbols {
        project: ProjectId,
        revision: u32,
    },
    /// The declaration of a changed symbol, from the language server, on the side
    /// the symbol exists on. Fails like `Impact` but also for a removed symbol.
    /// Answers with `Response::Signature`.
    Signature {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// References and callers of an added or modified changed symbol, on the
    /// target side (derived). Fails if the symbol is not one of
    /// `ChangedSymbols` at `revision`, or was removed. Answers with `Response::Impact`.
    Impact {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// What became of the references to a removed symbol (derived). Fails if the
    /// symbol is not a removed one of `ChangedSymbols` at `revision`. Answers
    /// with `Response::RemovalImpact`.
    RemovalImpact {
        project: ProjectId,
        revision: u32,
        symbol: SymbolId,
    },
    /// Whether an added or modified changed symbol is reached from an entry
    /// point (derived, §7.4). Fails like `Impact`. Answers with `Response::Reachability`.
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
    /// Changes what the live tree is compared against (§6.5). Review state is
    /// remapped onto the new base. The revision increases and every subscriber,
    /// including the requester, gets `SnapshotSettled`; fetch the manifest again.
    /// Fails with a message fit to show if the choice does not resolve.
    SetBase {
        project: ProjectId,
        base: BaseChoice,
    },
    /// What a base can be picked from.
    ListBaseCandidates {
        project: ProjectId,
    },
    /// Fetches the repository's remotes now, as a `Fetch` task, and answers with
    /// `Response::Done` when it ended. A fetch that fails (offline, refused) fails its
    /// task, not this request. Every subscriber then gets `BaseChanged`.
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
    /// The target-side text of any path of the current target snapshot, changed
    /// or not.
    FileText {
        project: ProjectId,
        path: String,
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
    /// Deletes a thread; this is also how a detached one is discarded.
    DeleteThread {
        project: ProjectId,
        thread: u32,
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
