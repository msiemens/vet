//! What the backend pushes without being asked.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{RestoreFailure, ReviewInfo, ReviewUpdate, RunStream, SessionId};

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
