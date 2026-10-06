//! Snapshots and changesets (SPEC §3). Pure content, no provenance (I1).

use std::collections::BTreeMap;

/// A 32-byte content hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Hash(pub [u8; 32]);

/// Identity of a file's content, as git would store it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlobId(pub Hash);

/// Identity of a snapshot: a Merkle root over its directories, so a settle only
/// recomputes the changed paths.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SnapshotId(pub Hash);

/// Repository-relative, `/`-separated, keyed by exact bytes (§6.3).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RepoPath(pub Vec<u8>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryKind {
    File,
    Symlink,
    Submodule,
}

/// Only properties of the content itself live here, and only these feed
/// `SnapshotId`. Topology-dependent facts (`PathCollision`, degraded watching)
/// belong on [`SnapshotStatus`] or session status, never in the identity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileEntry {
    pub blob: BlobId,
    /// Git mode, e.g. `0o100644`.
    pub mode: u32,
    pub kind: EntryKind,
    /// Raw worktree bytes of a path with a custom filter driver (§6.3).
    pub unfiltered: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub entries: BTreeMap<RepoPath, FileEntry>,
}

/// Facts about a snapshot that must not change its identity.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SnapshotStatus {
    /// Paths that differ only in case or normalization (§6.3).
    pub path_collisions: Vec<RepoPath>,
    /// Blobs over the capture size cap: hashed, but content not retained.
    pub content_unavailable: Vec<RepoPath>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Changeset {
    pub base: SnapshotId,
    pub target: SnapshotId,
}

/// A view over a changeset; never part of snapshot identity (§3).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ReviewFilter {
    pub roots: Vec<RepoPath>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub hide_binary: bool,
}

/// A line range, 0-based, end exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

/// One hunk of a line-level diff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hunk {
    pub old: LineRange,
    pub new: LineRange,
}

/// A git-free line diff between two versions of a file. Acquisition produces
/// these; Review and the symbol-to-hunk intersection consume them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileDiff {
    pub old_path: Option<RepoPath>,
    pub new_path: Option<RepoPath>,
    pub old_blob: Option<BlobId>,
    pub new_blob: Option<BlobId>,
    pub hunks: Vec<Hunk>,
}

/// Git-detected renames between two snapshots, as `old -> new`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RenameMap(pub BTreeMap<RepoPath, RepoPath>);
