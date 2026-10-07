//! What the backend pushes without being asked.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{RestoreFailure, ReviewInfo, ReviewUpdate, SessionId, Task};

/// Events are invalidations or small state updates; the client re-requests
/// whatever it displays.
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
    /// A dependency restore failed, so analysis is limited. `failures` is every
    /// failure of the session so far, as in `Manifest::restore_failures`.
    RestoreFailed {
        session: SessionId,
        failures: Vec<RestoreFailure>,
    },
    /// Background work of the session started, progressed or ended. Sent to every
    /// subscriber; a task that ended (`Finished` or `Failed`) is not announced again.
    Task(Task),
}
