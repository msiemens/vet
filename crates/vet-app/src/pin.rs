//! The pin recorded at review open (§6.1).

use vet_core::content::SnapshotId;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CommitId(pub String);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PinId(pub u64);

/// What a session's base is (§6.5). The target is always the live tree.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum BaseChoice {
    /// `HEAD` at open.
    #[default]
    Pin,
    /// The merge base of `reference` (a branch or any revision) and `HEAD`.
    MergeBase { reference: String },
    /// A commit. As requested the id may be abbreviated or a revision; the
    /// session stores and reports the full id.
    Commit { id: CommitId },
}

/// Recorded at review open (§6.1). Dirty content is copied into the store, never
/// into git's object database.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pin {
    pub id: PinId,
    pub head: CommitId,
    pub tree: SnapshotId,
}
