//! What the backend pushes without being asked.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Base, ChangesetKey, Notification, PlatformStatus, ProjectId, RestoreFailure, ReviewInfo, ReviewUpdate, Task,
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
    /// connection that asked for `Accounts`. The shell answers with `SetCredentials`
    /// if its keychain holds a token for the host, and otherwise does nothing.
    CredentialsNeeded { host: String },
    /// A snapshot settled and the changeset moved to a new `revision`. The
    /// manifest, with review state remapped onto it, is stale: fetch it again.
    SnapshotSettled { project: ProjectId, revision: u32 },
    /// Another changeset is on view now, at `revision`. The manifest, with the threads, is
    /// stale: fetch it again. The requester gets it too.
    ChangesetOpened {
        project: ProjectId,
        revision: u32,
        changeset: ChangesetKey,
    },
    /// Review state of one file of the changeset changed at the current `revision`; a mark
    /// made in an interdiff or a commit is announced as that of the changeset's file.
    ReviewChanged {
        project: ProjectId,
        revision: u32,
        update: ReviewUpdate,
    },
    /// The first review action started a review. Sent to every subscriber,
    /// the requester included; a finished review is `SnapshotSettled`.
    ReviewStarted { project: ProjectId, review: ReviewInfo },
    /// A thread was created, replied to, resolved, re-anchored, edited or deleted,
    /// or a sync with the platform changed threads; fetch the threads again. Not
    /// sent for a settled snapshot, which is `SnapshotSettled`: that also
    /// invalidates the threads.
    ThreadsChanged { project: ProjectId, revision: u32 },
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
    /// A dependency restore failed, so analysis is limited. `failures` is every
    /// failure of the project so far, as in `Manifest::restore_failures`.
    RestoreFailed {
        project: ProjectId,
        failures: Vec<RestoreFailure>,
    },
    /// A fetch ended: the base is where it was, and `base` says how far its branch has
    /// moved on now. Sent to every subscriber, whether the fetch was asked for or periodic.
    BaseChanged { project: ProjectId, base: Base },
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
