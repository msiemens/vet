//! Review (§5): owns the `Session`, which holds a `PinId` from Acquisition rather
//! than the pin itself. Remapping runs per changed file, triggered when either
//! the base or the target blob id changes (the base case is `SeenAgainstOldBase`).

use vet_core::content::{Changeset, FileEntry, NonTextKind, RepoPath, SnapshotId};
use vet_core::filter::ReviewFilter;
use vet_core::review::{ReviewedSpan, Thread};

use crate::pin::{BaseChoice, CommitId, PinId};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SessionId(pub u64);

/// A non-text change acknowledged like a hunk, keyed by its kind and the
/// before and after entries (§5.2). `None` for an added or deleted path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NonTextAck {
    pub kind: NonTextKind,
    pub before: Option<(RepoPath, FileEntry)>,
    pub after: Option<(RepoPath, FileEntry)>,
}

/// A span as recorded, in the snapshots it was marked in; never remapped in place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecordedSpan {
    pub id: SpanId,
    pub span: ReviewedSpan,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SpanId(pub u64);

pub struct Session {
    pub id: SessionId,
    pub pin: PinId,
    pub changeset: Changeset,
    /// How `changeset.base` was chosen.
    pub base: BaseChoice,
    /// The commit `changeset.base` is the tree of; `None` for the pin's `HEAD`.
    pub base_commit: Option<CommitId>,
    pub filter: ReviewFilter,
    pub spans: Vec<RecordedSpan>,
    pub acks: Vec<NonTextAck>,
    pub threads: Vec<Thread>,
    /// Unix seconds; `None` while open.
    pub closed_at: Option<i64>,
    /// Whether the review was complete when it was finished; `None` while open.
    pub complete: Option<bool>,
}

/// A settled snapshot of a session's tip and when it settled (unix seconds).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Version {
    pub snapshot: SnapshotId,
    pub at: i64,
}
