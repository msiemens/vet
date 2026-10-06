//! Anchors, review state, threads (SPEC §5). Content-anchored, never line-anchored (I3).

use crate::content::{BlobId, Changeset, FileDiff, Hash, LineRange, RenameMap, RepoPath, SnapshotId};
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
    Exact { whitespace_changed: bool },
    Inferred { strategy: Strategy, confidence: Confidence },
    Stale { reason: StaleReason },
    /// Duplicate matches; distinct from `Stale`.
    Ambiguous { candidates: Vec<Location> },
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

/// Everything remapping needs about one changed file, with no git in sight.
pub struct FileRemapInput<'a> {
    pub diff: &'a FileDiff,
    pub renames: &'a RenameMap,
    pub old_blob: Option<BlobId>,
    pub new_blob: Option<BlobId>,
}

/// Resolve an anchor into the same side of `changeset` (§5.1). Exact strategies
/// first, then scored inference; never first-success-wins for inferred candidates.
pub fn resolve_anchor(_anchor: &Anchor, _changeset: Changeset, _files: &[FileRemapInput<'_>]) -> AnchorState {
    todo!("§5.1 resolution algorithm")
}

/// Remap one span as a partition (§5.2): only exact strategies, one span may
/// become several or none. Triggered per changed file when either its base or
/// target blob id changes, so `SeenAgainstOldBase` is detected.
pub fn remap_span(_span: &ReviewedSpan, _file: &FileRemapInput<'_>) -> Vec<ReviewedSpan> {
    todo!("§5.2 partition remap")
}
