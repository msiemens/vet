//! The pin recorded at review open (§6.1).

use vet_core::content::SnapshotId;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CommitId(pub String);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PinId(pub u64);

/// Recorded at review open (§6.1). Dirty content is copied into the store, never
/// into git's object database.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pin {
    pub id: PinId,
    pub head: CommitId,
    pub tree: SnapshotId,
}
