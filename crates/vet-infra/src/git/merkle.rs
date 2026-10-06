//! `SnapshotId`: a Merkle root over directories. A directory's hash covers its
//! sorted children, so a changed file only invalidates the hashes on its path.

use std::collections::BTreeMap;


use vet_core::content::{EntryKind, FileEntry, Hash, RepoPath, SnapshotId};

#[derive(Default)]
struct Dir<'a> {
    files: BTreeMap<&'a [u8], &'a FileEntry>,
    dirs: BTreeMap<&'a [u8], Dir<'a>>,
}

pub fn snapshot_id<'a>(entries: impl IntoIterator<Item = (&'a RepoPath, &'a FileEntry)>) -> SnapshotId {
    let mut root = Dir::default();
    for (path, entry) in entries {
        let mut parts = path.0.split(|b| *b == b'/').peekable();
        let mut dir = &mut root;
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                dir = dir.dirs.entry(part).or_default();
            } else {
                dir.files.insert(part, entry);
            }
        }
    }
    SnapshotId(dir_hash(&root))
}

/// Children are tagged and length-prefixed, so names cannot run into hashes.
fn dir_hash(dir: &Dir) -> Hash {
    let mut buf = Vec::new();
    for (name, entry) in &dir.files {
        buf.extend_from_slice(b"f");
        buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
        buf.extend_from_slice(name);
        buf.extend_from_slice(&entry.blob.0.0);
        buf.extend_from_slice(&entry.mode.to_le_bytes());
        buf.extend_from_slice(&[
            match entry.kind {
                EntryKind::File => 0,
                EntryKind::Symlink => 1,
                EntryKind::Submodule => 2,
            },
            entry.unfiltered as u8,
        ]);
    }
    for (name, child) in &dir.dirs {
        buf.extend_from_slice(b"d");
        buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
        buf.extend_from_slice(name);
        buf.extend_from_slice(&dir_hash(child).0);
    }
    Hash::of(&buf)
}

#[cfg(test)]
mod tests {
    use vet_core::content::BlobId;

    use super::*;

    fn entry(n: u8) -> FileEntry {
        FileEntry { blob: BlobId(Hash([n; 32])), mode: 0o100644, kind: EntryKind::File, unfiltered: false }
    }

    #[test]
    fn identity_follows_content_not_insertion_order() {
        let (a, b) = (RepoPath(b"a/x".to_vec()), RepoPath(b"b".to_vec()));
        let id1 = snapshot_id([(&a, &entry(1)), (&b, &entry(2))]);
        let id2 = snapshot_id([(&b, &entry(2)), (&a, &entry(1))]);
        assert_eq!(id1, id2);
        assert_ne!(id1, snapshot_id([(&a, &entry(1)), (&b, &entry(3))]));
    }

    #[test]
    fn directory_structure_matters() {
        let e = entry(1);
        let flat = RepoPath(b"a_b".to_vec());
        let nested = RepoPath(b"a/b".to_vec());
        assert_ne!(snapshot_id([(&flat, &e)]), snapshot_id([(&nested, &e)]));
    }
}
