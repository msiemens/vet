//! What the backend pushes without being asked.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    ChangesetKey, CommitChecks, Diagnostic, FileSymbols, Lens, LineDelta, Notification, PlatformStatus, ProjectId,
    PullRequestUpdate, RestoreFailure, ReviewInfo, ReviewUpdate, ServerRuntimes, Task, Unavailable,
};

/// Events are invalidations or small state updates; the client re-requests
/// whatever it displays. An event that a request causes goes to every subscriber,
/// the requester included: of the requester's stores only the one that asked reads
/// the answer.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Event {
    /// The backend needs a credential for `host` and has none. Sent to every
    /// connection that asked to `Subscribe`. The shell answers with `SetCredentials`
    /// if its keychain holds a token for the host, and otherwise does nothing.
    CredentialsNeeded { host: String },
    /// A credential was given (`SetCredentials`, unless the token is the one held already
    /// and was not to be checked) or removed (`RemoveCredentials`): what `Accounts` says, and
    /// which pull requests the feed of each open project finds, may have changed. Sent to every
    /// connection that asked to `Subscribe`. Every open project's pull request feed lists again,
    /// and announces a different list as `PullRequestListChanged`; the open pull requests of the
    /// host are synced again, which they announce as usual.
    AccountsChanged,
    /// The recent list changed (`RecordRecentProject`, `ForgetRecentProject`): ask for
    /// `RecentProjects` again. Sent to every connection that asked to `Subscribe`.
    RecentProjectsChanged,
    /// The project at `root` was removed (`RemoveProject`): everything held for it is gone and, if
    /// it was open, so is its `ProjectId`. Sent to every connection that asked to `Subscribe`.
    ProjectRemoved { root: String },
    /// What the projects take on disk changed (`RemoveProject`, `ClearProjectCaches`): ask for
    /// `ProjectsStorage` again. Sent to every connection that asked to `Subscribe`.
    StorageChanged,
    /// The backend's own settings changed (`SetSettings`): ask for `Settings` again. Sent to
    /// every connection that asked to `Subscribe`.
    SettingsChanged,
    /// A preference was kept or forgotten (`SetPreference`): `value` is the new one, none for
    /// a forgotten one. Sent to every connection that asked to `Subscribe`.
    PreferenceChanged {
        scope: Option<String>,
        key: String,
        value: Option<String>,
    },
    /// The snapshot moved and the changeset is at a new `revision`: the working tree settled,
    /// the branch on view moved, or a fetch brought the pull request on view a push. The
    /// manifest, with review state remapped onto it, and the threads, whose anchors may have
    /// moved, are stale: fetch them again. `version` is the open review's latest version after
    /// the move, which a review records as its next version; none while browsing. Only with a
    /// `version` are the versions and the files changed since one stale.
    SnapshotMoved {
        project: ProjectId,
        revision: u32,
        version: Option<u32>,
    },
    /// The review filter changed (`SetProjectFilter`, `ExcludePath`, `NarrowReview`): no
    /// snapshot moved, so the threads and the versions are as they were, but the files in view
    /// and the counts are not. The manifest, which now has `revision`, and the files changed
    /// since a version are stale: fetch them again. Threads fetched at the revision before are
    /// valid at this one.
    FilterChanged { project: ProjectId, revision: u32 },
    /// The review ended (`FinishReview`) and the changeset is at a new `revision`, browsing: the
    /// manifest, the threads (the resolved ones leave the view), the versions and the files
    /// changed since a version are stale, and so are the changesets' rows, as the changeset no
    /// longer has a review open.
    ReviewFinished { project: ProjectId, revision: u32 },
    /// Edits of the working tree wait to settle into a snapshot (`pending`), or no longer do: the
    /// ones that wait are not part of any version yet. Sent whatever changeset is on view; no
    /// manifest is stale.
    EditsPending { project: ProjectId, pending: bool },
    /// Another changeset is on view now, at `revision`. The manifest, with the threads, is
    /// stale: fetch it again. The requester gets it too.
    ChangesetOpened {
        project: ProjectId,
        revision: u32,
        changeset: ChangesetKey,
    },
    /// Review state of one file of the changeset changed at the current `revision`; a mark
    /// made in an interdiff or a commit is announced as that of the changeset's file.
    /// `reviewed_at` is set when the mark moved where the review last was, which the first
    /// seen-mark of a version does (not an unmark, a flag or a later mark): the version it now
    /// is at. Only then do the versions, the files changed since a version and the queue's
    /// rows differ from before.
    ReviewChanged {
        project: ProjectId,
        revision: u32,
        update: ReviewUpdate,
        reviewed_at: Option<u32>,
    },
    /// The changeset on view is reviewed in `lens` now (`SetLens`). The manifest, which
    /// says the lens, is stale: fetch it again.
    LensChanged { project: ProjectId, lens: Lens },
    /// What the manifest says of the target changed without a new revision: how the
    /// platform names the pull request on view (its title, whether it is a draft), as a
    /// fetch of it found, or what analysis may execute (`SetProjectTrusted`,
    /// `TrustPullRequest`). Fetch the manifest again.
    TargetChanged { project: ProjectId },
    /// What `ListChangesets` lists, or the lines of the queue's rows, may have changed: a fetch
    /// moved branches or the pull request feed brought in pull request heads. The feed announces a
    /// different list as `PullRequestListChanged`. List the changesets again.
    ChangesetsChanged { project: ProjectId },
    /// The pull request feed published a different list. Read `ListPullRequests` again; it answers
    /// from the feed and asks no platform while the list is kept.
    PullRequestListChanged { project: ProjectId },
    /// What `ListPullRequests` lists of some pull requests changed because their details were
    /// read, which the platform's list leaves out. `changed` carries, for each, what its details
    /// add to its row; patch those rows, do not list again.
    PullRequestsChanged {
        project: ProjectId,
        changed: Vec<PullRequestUpdate>,
    },
    /// A chunk of a `ChangesetLines` answer, pushed in the request's order while the backend
    /// counts the rest (A16), so the first rows show before the last is counted. `changesets`
    /// are at `lines`, in the same order. The request's answer settles the whole list, so a
    /// client may apply this as it arrives and let the answer overwrite it.
    ChangesetLinesChanged {
        project: ProjectId,
        changesets: Vec<ChangesetKey>,
        lines: Vec<Option<LineDelta>>,
    },
    /// The first review action started a review. Sent to every subscriber,
    /// the requester included; a finished review is `ReviewFinished`.
    ReviewStarted { project: ProjectId, review: ReviewInfo },
    /// A thread was created, replied to, resolved, re-anchored, edited or deleted,
    /// or a sync with the platform changed threads; fetch the threads again. Not
    /// sent for a moved snapshot, which is `SnapshotMoved`: that also invalidates the threads.
    ThreadsChanged { project: ProjectId, revision: u32 },
    /// The changed symbols of one file of the changeset on view, as the backend analysed them. The
    /// backend analyses the changeset's files itself, while the review screen shows it, the files
    /// with an open tab first, and pushes each answer as it comes (SPEC §6.6, A13). `done` are the
    /// files analysed so far of `total`; a file whose symbols are unknown says so in
    /// `symbols.gap`, and one whose analysis failed outright carries the reason there too.
    FileSymbols {
        project: ProjectId,
        symbols: FileSymbols,
        done: u32,
        total: u32,
    },
    /// The diagnostics of one file of the changeset on view, as the backend analysed them. The
    /// backend compares the changeset's files with the base itself, while the review screen shows
    /// it, the files with an open tab first, and pushes each answer as it comes (SPEC §6.6, A13):
    /// one with `compared` false first, whose `diagnostics` have their origin pending, then one
    /// with `compared` true, whose origins are new, existing or unknown. `unavailable` says the
    /// file could not be analysed; the client does not ask per file.
    FileDiagnostics {
        project: ProjectId,
        revision: u32,
        path: String,
        diagnostics: Vec<Diagnostic>,
        /// How many of `diagnostics` to tell the viewer about: hints are not counted.
        count: u32,
        unavailable: Option<Unavailable>,
        /// Why the origins are unknown, when the base side could not be compared.
        origin_unavailable: Option<Unavailable>,
        /// Set (to `Incomplete`) when the server was at work on its own while it answered.
        incomplete: Option<Unavailable>,
        /// The base side has been compared: the origins are as known as they will be.
        compared: bool,
    },
    /// The checks (CI runs and statuses) of the commits the review view shows, as the backend last
    /// read them. The backend refreshes them itself while the review screen shows the changeset, so
    /// the client shows them without asking. `checks` are the commits answered for, in the order
    /// asked; `error` why some were left out; `limited_until` (unix seconds) until when the
    /// platform limits requests, which is why `error` says so.
    ChecksChanged {
        project: ProjectId,
        checks: Vec<CommitChecks>,
        error: Option<String>,
        limited_until: Option<u32>,
    },
    /// A line of output of an `InstallServer` in progress, to the connections of
    /// the project it was asked on.
    ServerInstallOutput {
        project: ProjectId,
        language: String,
        line: String,
    },
    /// A server was installed or a server path changed, which restarted the
    /// language's servers in every project: what `ListServers` says, and what the
    /// servers answer, may have changed. Sent to the connections of every project.
    ServersChanged { project: ProjectId },
    /// What the servers of the changeset on view are doing changed: one started, indexes, is
    /// ready, failed or was stopped, or another changeset came on view.
    ServerRuntimes {
        project: ProjectId,
        runtimes: ServerRuntimes,
    },
    /// A dependency restore failed, so analysis is limited. `failures` is every
    /// failure of the project so far, as in `Manifest::restore_failures`.
    RestoreFailed {
        project: ProjectId,
        failures: Vec<RestoreFailure>,
    },
    /// Background work of the project started, progressed or ended. Sent to every
    /// subscriber; a task that ended (`Finished` or `Failed`) is not announced again.
    Task(Task),
    /// How the backend gets on with the platform of the pull request on view, after it tried to
    /// sync with it. Sent to every subscriber each time, so a late one need not ask.
    PlatformStatus { project: ProjectId, status: PlatformStatus },
    /// Something the viewer should hear about, to every subscriber of the project: a card in the
    /// window's corner that the bell in the footer keeps in its list.
    Notification {
        project: ProjectId,
        notification: Notification,
    },
}
