//! Snapshots and changesets (SPEC §3). Pure content, no provenance.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

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
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        blake3::Hash::from_hex(hex).ok().map(|h| Self(*h.as_bytes()))
    }

    /// BLAKE3 of everything `reader` yields, without holding it in memory.
    pub fn of_reader(reader: impl std::io::Read) -> std::io::Result<Self> {
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(reader)?;
        Ok(Self(*hasher.finalize().as_bytes()))
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

/// Repository-relative, `/`-separated, keyed by exact bytes (§4.3).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RepoPath(pub Vec<u8>);

impl RepoPath {
    /// The part of the file name after its last dot; `None` for a name without a dot.
    pub fn extension(&self) -> Option<&[u8]> {
        let name = self.0.rsplit(|&b| b == b'/').next()?;
        let dot = name.iter().rposition(|&b| b == b'.')?;
        Some(&name[dot + 1..])
    }

    /// The path for display; invalid UTF-8 is replaced.
    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }

    /// The order the file tree and the file lists show paths in: segment by segment, at the first
    /// differing one by [`entry_order`]. Total, so it never ties for different paths.
    pub fn tree_order(&self, other: &Self) -> Ordering {
        let mut a = self.0.split(|&b| b == b'/').peekable();
        let mut b = other.0.split(|&b| b == b'/').peekable();
        loop {
            let (Some(x), Some(y)) = (a.next(), b.next()) else {
                // A file replaced by a folder of its name: the folder, with more segments, first.
                return other.0.len().cmp(&self.0.len());
            };
            if x == y {
                continue;
            }
            return entry_order((a.peek().is_some(), x), (b.peek().is_some(), y));
        }
    }
}

/// How the entries of one folder are ordered, each a (is a folder, name): folders first, then names
/// by Unicode lowercase, ties by bytes.
pub fn entry_order((a_dir, a): (bool, &[u8]), (b_dir, b): (bool, &[u8])) -> Ordering {
    let lower = |name: &[u8]| String::from_utf8_lossy(name).to_lowercase();
    b_dir
        .cmp(&a_dir)
        .then_with(|| lower(a).cmp(&lower(b)))
        .then_with(|| a.cmp(b))
}

impl From<&str> for RepoPath {
    fn from(path: &str) -> Self {
        Self(path.as_bytes().to_vec())
    }
}

impl From<String> for RepoPath {
    fn from(path: String) -> Self {
        Self(path.into_bytes())
    }
}

/// Whether `bytes` are not text: git's heuristic, a NUL in the first 8000 bytes.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8000)].contains(&0)
}

/// The text of `bytes` as the review shows it: invalid UTF-8 is replaced, and
/// `None` if they are binary.
pub fn decode_text(bytes: &[u8]) -> Option<String> {
    (!is_binary(bytes)).then(|| String::from_utf8_lossy(bytes).into_owned())
}

/// Like [`decode_text`] for bytes that are given up: valid UTF-8 is not copied.
pub fn decode_text_owned(bytes: Vec<u8>) -> Option<String> {
    if is_binary(&bytes) {
        return None;
    }
    Some(String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

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
    /// Raw worktree bytes of a path with a custom filter driver (§4.3).
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
    /// Paths that differ only in case or normalization (§4.3).
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

/// Paths that differ only in case or Unicode normalization (§4.3), sorted.
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

/// Folders whose name a tool defines as its own store; their untracked contents are never part of
/// a snapshot (SPEC §4.1).
const TOOL_STORES: &[&[u8]] = &[b".pnpm-store"];

/// Whether `path` (repository-relative, `/`-separated) is a tool store or inside one.
pub fn in_tool_store(path: &[u8]) -> bool {
    path.split(|&b| b == b'/').any(|part| TOOL_STORES.contains(&part))
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

    /// Whether the ranges overlap. An empty range overlaps only a range it lies strictly inside.
    pub fn intersects(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Whether `other` lies within the range, an empty one at its edge included.
    pub fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

/// One hunk of a line-level diff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hunk {
    pub old: LineRange,
    pub new: LineRange,
}

/// How many lines a diff adds and removes: a file's `+N −M`, or a changeset's.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct LineDelta {
    /// Lines of the new side the hunks cover.
    pub added: u32,
    /// Lines of the old side the hunks cover.
    pub removed: u32,
}

impl LineDelta {
    pub fn of(hunks: &[Hunk]) -> Self {
        hunks.iter().fold(Self::default(), |sum, h| Self {
            added: sum.added + h.new.len(),
            removed: sum.removed + h.old.len(),
        })
    }
}

impl std::iter::Sum for LineDelta {
    fn sum<I: Iterator<Item = Self>>(parts: I) -> Self {
        parts.fold(Self::default(), |a, b| Self {
            added: a.added + b.added,
            removed: a.removed + b.removed,
        })
    }
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

impl FileDiff {
    /// The path the file is known by: the new one, else the old one of a deletion.
    pub fn path(&self) -> &RepoPath {
        (self.new_path.as_ref().or(self.old_path.as_ref())).expect("a diff has a path")
    }
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
    /// The paths of `files` and `non_text` that the repository's attributes or the files' headers
    /// mark as generated; the review filter leaves them out.
    pub generated: BTreeSet<RepoPath>,
}

impl SnapshotDiff {
    /// The files the diff changes, by path, with the lines they gain and lose. A file with no
    /// text lines to count (a mode change, a binary file) has none.
    pub fn changed_files(&self) -> Vec<(RepoPath, LineDelta)> {
        let mut files: BTreeMap<RepoPath, LineDelta> = BTreeMap::new();
        for file in &self.files {
            files.insert(file.path().clone(), LineDelta::of(&file.hunks));
        }
        for item in &self.non_text {
            files.entry(item.path.clone()).or_default();
        }
        files.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_order_puts_folders_first_then_names_without_case_and_ties_by_bytes() {
        let mut paths: Vec<RepoPath> = [
            "z.txt",
            "B.txt",
            "a.txt",
            "src/b/y.rs",
            "src/b.rs",
            "src/a.rs",
            "src/B/x.rs",
            "Src/q.rs",
            "src",
            "src/ä.rs",
            "src/z.rs",
            "a",
            "a/x",
        ]
        .map(RepoPath::from)
        .into();
        paths.sort_by(RepoPath::tree_order);
        let sorted: Vec<String> = paths.iter().map(RepoPath::to_string_lossy).collect();
        assert_eq!(
            sorted,
            [
                "a/x",
                "Src/q.rs",
                "src/B/x.rs",
                "src/b/y.rs",
                "src/a.rs",
                "src/b.rs",
                "src/z.rs",
                "src/ä.rs",
                "a",
                "a.txt",
                "B.txt",
                "src",
                "z.txt"
            ]
        );
    }

    fn hunk(old: (u32, u32), new: (u32, u32)) -> Hunk {
        Hunk {
            old: LineRange {
                start: old.0,
                end: old.1,
            },
            new: LineRange {
                start: new.0,
                end: new.1,
            },
        }
    }

    #[test]
    fn a_diff_adds_the_new_lines_and_removes_the_old_ones() {
        let hunks = [
            hunk((2, 5), (2, 4)),
            // A pure addition and a pure deletion.
            hunk((9, 9), (8, 11)),
            hunk((20, 22), (22, 22)),
        ];
        assert_eq!(LineDelta::of(&hunks), LineDelta { added: 5, removed: 5 });
        assert_eq!(LineDelta::of(&[]), LineDelta::default());
    }

    #[test]
    fn a_path_has_the_extension_of_its_file_name() {
        let extension = |p: &str| RepoPath::from(p).extension().map(<[u8]>::to_vec);
        assert_eq!(extension("a/b.PNG"), Some(b"PNG".to_vec()));
        assert_eq!(extension("x.tar.gz"), Some(b"gz".to_vec()));
        assert_eq!(extension(".gitignore"), Some(b"gitignore".to_vec()));
        assert_eq!(extension("x."), Some(Vec::new()));
        assert_eq!(extension("png"), None);
        assert_eq!(extension("dir.png/file"), None);
    }

    #[test]
    fn binary_is_a_nul_in_the_first_8000_bytes() {
        assert!(!is_binary(b""));
        assert!(!is_binary(b"text\n"));
        assert!(is_binary(b"a\0b"));
        let mut late = vec![b'a'; 8000];
        late.push(0);
        assert!(!is_binary(&late));
        late[7999] = 0;
        assert!(is_binary(&late));
    }

    #[test]
    fn text_is_decoded_lossily_unless_binary() {
        assert_eq!(decode_text(b"caf\xc3\xa9\n").as_deref(), Some("caf\u{e9}\n"));
        assert_eq!(decode_text(b"a\xffb").as_deref(), Some("a\u{fffd}b"));
        assert_eq!(decode_text(b"a\0b"), None);
    }

    #[test]
    fn owned_bytes_decode_as_borrowed_ones_do() {
        for bytes in [&b"caf\xc3\xa9\n"[..], b"a\xffb", b"a\0b", b""] {
            assert_eq!(decode_text_owned(bytes.to_vec()), decode_text(bytes));
        }
    }

    #[test]
    fn line_ranges_intersect_and_contain() {
        let range = |start, end| LineRange { start, end };
        assert!(range(2, 5).intersects(range(4, 8)));
        assert!(range(4, 8).intersects(range(2, 5)));
        assert!(!range(2, 5).intersects(range(5, 8)));
        assert!(range(2, 5).intersects(range(3, 3)));
        assert!(!range(2, 5).intersects(range(2, 2)));
        assert!(!range(2, 5).intersects(range(5, 5)));
        assert!(range(2, 5).contains(range(2, 5)));
        assert!(range(2, 5).contains(range(3, 3)));
        assert!(range(2, 5).contains(range(5, 5)));
        assert!(!range(2, 5).contains(range(1, 3)));
        assert!(!range(2, 5).contains(range(4, 6)));
    }

    #[test]
    fn a_diff_is_known_by_its_new_path_else_its_old_one() {
        let diff = |old: Option<&str>, new: Option<&str>| FileDiff {
            old_path: old.map(RepoPath::from),
            new_path: new.map(RepoPath::from),
            old_blob: None,
            new_blob: None,
            hunks: Vec::new(),
        };
        assert_eq!(diff(Some("a"), Some("b")).path(), &RepoPath::from("b"));
        assert_eq!(diff(Some("a"), None).path(), &RepoPath::from("a"));
        assert_eq!(diff(None, Some("b")).path(), &RepoPath::from("b"));
    }

    #[test]
    fn a_tool_store_and_everything_inside_it_is_out_of_scope() {
        assert!(in_tool_store(b".pnpm-store"));
        assert!(in_tool_store(b"web/.pnpm-store/v3/files/ab"));
        assert!(!in_tool_store(b"src/lib.rs"));
        assert!(!in_tool_store(b"x.pnpm-store/a"));
        assert!(!in_tool_store(b""));
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
        .map(RepoPath::from)
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
        let entries = ["A", "a", "b"].map(|p| (RepoPath::from(p), entry.clone())).into();
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
