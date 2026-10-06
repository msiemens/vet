//! Anchors, review state, threads (SPEC §5). Content-anchored, never line-anchored (I3).

use crate::content::{BlobId, Hash, LineRange, RepoPath, SnapshotId};
use crate::platform::PlatformThread;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Base,
    Target,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Anchor {
    pub side: Side,
    pub snapshot: SnapshotId,
    pub file: RepoPath,
    pub range: LineRange,
    pub content_hash_raw: Hash,
    /// LF line endings, trailing whitespace stripped.
    pub content_hash_norm: Hash,
    pub context_hashes: Vec<Hash>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Location {
    pub file: RepoPath,
    pub range: LineRange,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strategy {
    ContextWindow,
    CrossFileMove,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StaleReason {
    ContextChanged,
    FileDeleted,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AnchorState {
    Exact {
        whitespace_changed: bool,
    },
    Inferred {
        strategy: Strategy,
        confidence: Confidence,
    },
    Stale {
        reason: StaleReason,
    },
    /// Duplicate matches; distinct from `Stale`.
    Ambiguous {
        candidates: Vec<Location>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CommentTarget {
    Range(Anchor),
    File { side: Side, path: RepoPath },
    Changeset,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ThreadId(pub u64);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Message {
    pub author: String,
    pub body: String,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreadStatus {
    Open,
    Resolved,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Thread {
    pub id: ThreadId,
    pub target: CommentTarget,
    pub messages: Vec<Message>,
    pub status: ThreadStatus,
    pub platform: Option<PlatformThread>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanState {
    Seen,
    SeenAgainstOldBase,
}

/// The durable unit of review coverage (§5.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReviewedSpan {
    /// `None` for pure additions.
    pub base: Option<Anchor>,
    /// `None` for pure deletions.
    pub target: Option<Anchor>,
    pub state: SpanState,
}

/// One file on one side of a changeset, from the snapshot an anchor or span was
/// recorded in (`old_*`) to the snapshot it is resolved against (`new_*`). An
/// unchanged file has equal blobs and paths. Callers apply git-detected renames.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileRemapInput {
    pub old_path: Option<RepoPath>,
    pub new_path: Option<RepoPath>,
    pub old_blob: Option<BlobId>,
    pub new_blob: Option<BlobId>,
}
