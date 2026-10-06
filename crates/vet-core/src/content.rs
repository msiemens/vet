//! Snapshots and changesets (SPEC §3). Pure content, no provenance (I1).

use std::collections::BTreeMap;

use unicode_normalization::UnicodeNormalization;

/// A 32-byte content hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// BLAKE3 of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// The 64 lowercase hex digits [`Self::from_hex`] reads back.
    pub fn to_hex(&self) -> String {
        use std::fmt::Write;
        self.0.iter().fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let digits = hex.as_bytes();
        if digits.len() != 64 || !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let mut out = [0; 32];
        for (byte, pair) in out.iter_mut().zip(digits.chunks(2)) {
            let pair = std::str::from_utf8(pair).ok()?;
            *byte = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(out))
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

impl SnapshotStatus {
    /// The status every snapshot has regardless of origin: collisions are a pure
    /// function of the paths, so a commit's tree is flagged like a worktree scan.
    pub fn of(snapshot: &Snapshot) -> Self {
        Self {
            path_collisions: path_collisions(snapshot.entries.keys()),
            content_unavailable: Vec::new(),
        }
    }
}

/// Paths that differ only in case or Unicode normalization (§6.3), sorted.
pub fn path_collisions<'a>(paths: impl IntoIterator<Item = &'a RepoPath>) -> Vec<RepoPath> {
    let mut groups: BTreeMap<Vec<u8>, Vec<&RepoPath>> = BTreeMap::new();
    for path in paths {
        let key = match std::str::from_utf8(&path.0) {
            Ok(s) => s.nfc().collect::<String>().to_lowercase().into_bytes(),
            Err(_) => path.0.clone(),
        };
        groups.entry(key).or_default().push(path);
    }
    let colliding: std::collections::BTreeSet<_> = groups
        .into_values()
        .filter(|g| g.len() > 1)
        .flatten()
        .cloned()
        .collect();
    colliding.into_iter().collect()
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
    /// An empty file added or deleted: it exists, but has no lines to review.
    Empty,
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

/// The media type of a common image format, from the file extension (any case).
pub fn image_media_type(path: &RepoPath) -> Option<&'static str> {
    let name = path.0.rsplit(|&b| b == b'/').next().unwrap_or_default();
    let extension = name.rsplit(|&b| b == b'.').next().filter(|_| name.contains(&b'.'))?;
    Some(match extension.to_ascii_lowercase().as_slice() {
        b"png" => "image/png",
        b"jpg" | b"jpeg" => "image/jpeg",
        b"gif" => "image/gif",
        b"webp" => "image/webp",
        b"bmp" => "image/bmp",
        b"ico" => "image/x-icon",
        b"svg" => "image/svg+xml",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_media_types_come_from_the_extension() {
        let media = |p: &str| image_media_type(&RepoPath(p.as_bytes().to_vec()));
        assert_eq!(media("a/b.PNG"), Some("image/png"));
        assert_eq!(media("x.jpeg"), Some("image/jpeg"));
        assert_eq!(media("x.svg"), Some("image/svg+xml"));
        assert_eq!(media("x.txt"), None);
        assert_eq!(media("png"), None);
        assert_eq!(media("dir.png/file"), None);
    }

    #[test]
    fn a_hash_round_trips_through_hex() {
        let hash = Hash::of(b"x");
        let hex = hash.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(Hash::from_hex(&hex), Some(hash));
        assert_eq!(Hash::from_hex(&hex[1..]), None);
        assert_eq!(Hash::from_hex(&format!("+{}", &hex[1..])), None);
    }

    #[test]
    fn paths_differing_only_in_case_or_normalization_collide() {
        let paths: Vec<_> = [
            "Foo.txt",
            "foo.txt",
            "caf\u{e9}",
            "cafe\u{301}",
            "other",
            "dir/a",
            "dir/b",
        ]
        .map(|p| RepoPath(p.as_bytes().to_vec()))
        .into();
        let colliding: Vec<_> = path_collisions(&paths).into_iter().map(|p| p.0).collect();
        let expected: Vec<Vec<u8>> = ["Foo.txt", "cafe\u{301}", "caf\u{e9}", "foo.txt"]
            .map(|p| p.as_bytes().to_vec())
            .into();
        assert_eq!(colliding, expected);
    }

    #[test]
    fn a_snapshot_status_flags_collisions_of_its_entries() {
        let entry = FileEntry {
            blob: BlobId::of(b""),
            mode: 0o100644,
            kind: EntryKind::File,
            unfiltered: false,
        };
        let entries = ["A", "a", "b"]
            .map(|p| (RepoPath(p.as_bytes().to_vec()), entry.clone()))
            .into();
        let snapshot = Snapshot {
            id: SnapshotId(Hash::of(b"")),
            entries,
        };
        let status = SnapshotStatus::of(&snapshot);
        assert_eq!(
            status.path_collisions,
            [RepoPath(b"A".to_vec()), RepoPath(b"a".to_vec())]
        );
        assert!(status.content_unavailable.is_empty());
    }
}
