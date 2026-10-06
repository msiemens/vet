//! Snapshots and changesets (SPEC §3). Pure content, no provenance (I1).

use std::collections::BTreeMap;

/// A 32-byte content hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// BLAKE3 of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// BLAKE3 of everything `reader` yields, without holding it in memory.
    pub fn of_reader(mut reader: impl std::io::Read) -> std::io::Result<Self> {
        let mut hasher = blake3::Hasher::new();
        let mut buf = [0; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return Ok(Self(*hasher.finalize().as_bytes())),
                Ok(n) => {
                    hasher.update(&buf[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

/// Identity of a file's content: the hash of its bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlobId(pub Hash);

impl BlobId {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Hash::of(bytes))
    }
}

/// Blob bytes by id. `None` when the content is unavailable (over the capture cap).
pub trait BlobSource {
    fn blob(&self, id: BlobId) -> Option<&[u8]>;
}

impl BlobSource for BTreeMap<BlobId, Vec<u8>> {
    fn blob(&self, id: BlobId) -> Option<&[u8]> {
        self.get(&id).map(Vec::as_slice)
    }
}

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

/// A line range, 0-based, end exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NonTextKind {
    Mode,
    /// Symlink target change, or a change between file and symlink.
    Symlink,
    /// Pinned commit change.
    Submodule,
    Binary,
    /// Content over the capture cap or otherwise not retained.
    ContentUnavailable,
    /// Same content at a new path.
    Rename,
}

/// A file-level change with no reviewable text. It is acknowledged as one item,
/// keyed by its before and after entries (§5.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NonTextChange {
    /// The new path, or the old one for a deletion.
    pub path: RepoPath,
    /// The old path of a rename.
    pub from: Option<RepoPath>,
    pub kind: NonTextKind,
    pub before: Option<FileEntry>,
    pub after: Option<FileEntry>,
}

/// Everything that differs between two snapshots.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SnapshotDiff {
    pub files: Vec<FileDiff>,
    pub non_text: Vec<NonTextChange>,
    pub renames: RenameMap,
}
