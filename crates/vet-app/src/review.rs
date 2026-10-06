//! Review (§5): owns the `Session`, which holds a `PinId` from Acquisition rather
//! than the pin itself. Remapping runs per changed file, triggered when either
//! the base or the target blob id changes (the base case is `SeenAgainstOldBase`).

use vet_core::content::{Changeset, FileEntry, ReviewFilter, RepoPath};
use vet_core::review::{ReviewedSpan, Thread};

use crate::acquisition::PinId;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SessionId(pub u64);

/// A non-text change acknowledged like a hunk, keyed by the before and after
/// entries (§5.2). `None` for an added or deleted path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NonTextAck {
    pub before: Option<(RepoPath, FileEntry)>,
    pub after: Option<(RepoPath, FileEntry)>,
}

pub struct Session {
    pub id: SessionId,
    pub pin: PinId,
    pub changeset: Changeset,
    pub filter: ReviewFilter,
    pub spans: Vec<ReviewedSpan>,
    pub acks: Vec<NonTextAck>,
    pub threads: Vec<Thread>,
    /// Unix seconds; `None` while open.
    pub closed_at: Option<i64>,
}
