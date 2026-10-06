//! The start record of a review (§6.1) and the base choice (§6.5).

use vet_core::content::SnapshotId;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CommitId(pub String);

impl CommitId {
    /// The abbreviation shown for a commit with no better name.
    pub fn short(&self) -> &str {
        &self.0[..self.0.len().min(7)]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PinId(pub u64);

/// What a session's base is (§6.5). The target is always the live tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BaseChoice {
    /// The tip of a local branch as it was when chosen; the branch moving on
    /// does not move the base.
    Branch { name: String },
    /// The merge base of `reference` (a branch or any revision) and `HEAD`.
    MergeBase { reference: String },
    /// A commit. As requested the id may be abbreviated or a revision; the
    /// session stores and reports the full id.
    Commit { id: CommitId },
}

/// Recorded when a review starts (§6.1): `HEAD` and the tree of version 1. Dirty
/// content is copied into the store, never into git's object database.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pin {
    pub id: PinId,
    pub head: CommitId,
    pub tree: SnapshotId,
}
