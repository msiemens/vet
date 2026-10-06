//! `SnapshotId` as a Merkle root over directories (SPEC §3). A node is one
//! directory's sorted entries; its hash is `Hash::of` of its encoding, so a
//! changed file only changes the nodes on its path and unchanged directories
//! can be shared between snapshots.

use std::collections::BTreeMap;

use crate::content::{BlobId, EntryKind, FileEntry, Hash, RepoPath, SnapshotId};

/// One directory: files by name, subdirectories by name and node hash.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TreeNode {
    pub files: BTreeMap<Vec<u8>, FileEntry>,
    pub dirs: BTreeMap<Vec<u8>, Hash>,
}

impl TreeNode {
    /// Files first, then directories, each in name order. Names are length
    /// prefixed, so the encoding is unambiguous.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, e) in &self.files {
            out.push(b'f');
            put_name(&mut out, name);
            out.extend_from_slice(&e.blob.0.0);
            out.extend_from_slice(&e.mode.to_le_bytes());
            out.push(match e.kind {
                EntryKind::File => 0,
                EntryKind::Symlink => 1,
                EntryKind::Submodule => 2,
            });
            out.push(e.unfiltered as u8);
        }
        for (name, hash) in &self.dirs {
            out.push(b'd');
            put_name(&mut out, name);
            out.extend_from_slice(&hash.0);
        }
        out
    }

    pub fn decode(mut bytes: &[u8]) -> Option<Self> {
        let mut node = Self::default();
        while let Some((&tag, rest)) = bytes.split_first() {
            let (name, rest) = take_name(rest)?;
            match tag {
                b'f' => {
                    let (blob, rest) = rest.split_first_chunk::<32>()?;
                    let (mode, rest) = rest.split_first_chunk::<4>()?;
                    let [kind, unfiltered, rest @ ..] = rest else {
                        return None;
                    };
                    let kind = match kind {
                        0 => EntryKind::File,
                        1 => EntryKind::Symlink,
                        2 => EntryKind::Submodule,
                        _ => return None,
                    };
                    let entry = FileEntry {
                        blob: BlobId(Hash(*blob)),
                        mode: u32::from_le_bytes(*mode),
                        kind,
                        unfiltered: *unfiltered != 0,
                    };
                    node.files.insert(name.to_vec(), entry);
                    bytes = rest;
                }
                b'd' => {
                    let (hash, rest) = rest.split_first_chunk::<32>()?;
                    node.dirs.insert(name.to_vec(), Hash(*hash));
                    bytes = rest;
                }
                _ => return None,
            }
        }
        Some(node)
    }

    pub fn hash(&self) -> Hash {
        Hash::of(&self.encode())
    }
}

fn put_name(out: &mut Vec<u8>, name: &[u8]) {
    out.extend_from_slice(&(name.len() as u32).to_le_bytes());
    out.extend_from_slice(name);
}

fn take_name(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let (len, rest) = bytes.split_first_chunk::<4>()?;
    rest.split_at_checked(u32::from_le_bytes(*len) as usize)
}

/// The root id and every node of a snapshot, by hash.
pub fn build(entries: &BTreeMap<RepoPath, FileEntry>) -> (SnapshotId, BTreeMap<Hash, TreeNode>) {
    #[derive(Default)]
    struct Dir<'a> {
        files: BTreeMap<&'a [u8], &'a FileEntry>,
        dirs: BTreeMap<&'a [u8], Dir<'a>>,
    }

    fn finish(dir: &Dir, nodes: &mut BTreeMap<Hash, TreeNode>) -> Hash {
        let node = TreeNode {
            files: dir.files.iter().map(|(n, e)| (n.to_vec(), (*e).clone())).collect(),
            dirs: dir.dirs.iter().map(|(n, d)| (n.to_vec(), finish(d, nodes))).collect(),
        };
        let hash = node.hash();
        nodes.insert(hash, node);
        hash
    }

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
    let mut nodes = BTreeMap::new();
    let root = finish(&root, &mut nodes);
    (SnapshotId(root), nodes)
}

pub fn snapshot_id(entries: &BTreeMap<RepoPath, FileEntry>) -> SnapshotId {
    build(entries).0
}

/// Rebuild a snapshot's entries from its nodes. `None` if a node is missing
/// or malformed.
pub fn entries(root: SnapshotId, node: &impl Fn(Hash) -> Option<TreeNode>) -> Option<BTreeMap<RepoPath, FileEntry>> {
    fn walk(
        hash: Hash,
        prefix: &[u8],
        node: &impl Fn(Hash) -> Option<TreeNode>,
        out: &mut BTreeMap<RepoPath, FileEntry>,
    ) -> Option<()> {
        let join = |name: &[u8]| [prefix, name].concat();
        let n = node(hash)?;
        for (name, entry) in n.files {
            out.insert(RepoPath(join(&name)), entry);
        }
        for (name, child) in n.dirs {
            walk(child, &[join(&name).as_slice(), b"/"].concat(), node, out)?;
        }
        Some(())
    }
    let mut out = BTreeMap::new();
    walk(root.0, b"", node, &mut out)?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u8) -> FileEntry {
        FileEntry {
            blob: BlobId::of(&[n]),
            mode: 0o100644,
            kind: EntryKind::File,
            unfiltered: false,
        }
    }

    fn snapshot(files: &[(&str, u8)]) -> BTreeMap<RepoPath, FileEntry> {
        files
            .iter()
            .map(|(p, n)| (RepoPath(p.as_bytes().to_vec()), entry(*n)))
            .collect()
    }

    #[test]
    fn same_entries_give_the_same_root() {
        let a = snapshot(&[("a/x", 1), ("b", 2)]);
        let b = snapshot(&[("b", 2), ("a/x", 1)]);
        assert_eq!(build(&a).0, build(&b).0);
        assert_ne!(snapshot_id(&a), snapshot_id(&snapshot(&[("a/x", 1), ("b", 3)])));
    }

    #[test]
    fn directory_structure_matters() {
        assert_ne!(
            snapshot_id(&snapshot(&[("a_b", 1)])),
            snapshot_id(&snapshot(&[("a/b", 1)]))
        );
    }

    #[test]
    fn a_change_only_touches_nodes_on_its_path() {
        let before = build(&snapshot(&[("a/b/x", 1), ("a/y", 1), ("c/z", 1)]));
        let after = build(&snapshot(&[("a/b/x", 2), ("a/y", 1), ("c/z", 1)]));
        let new: Vec<_> = after.1.keys().filter(|h| !before.1.contains_key(h)).collect();
        // root, `a` and `a/b`; `c` is shared.
        assert_eq!(new.len(), 3);
        assert_eq!(
            before.1.len() - after.1.keys().filter(|h| before.1.contains_key(h)).count(),
            3
        );
    }

    #[test]
    fn nodes_round_trip_and_rebuild_entries() {
        let files = snapshot(&[("a/b/x", 1), ("a/y", 2), ("c", 3)]);
        let (root, nodes) = build(&files);
        for (hash, node) in &nodes {
            assert_eq!(TreeNode::decode(&node.encode()).as_ref(), Some(node));
            assert_eq!(node.hash(), *hash);
        }
        let rebuilt = entries(root, &|h| nodes.get(&h).cloned());
        assert_eq!(rebuilt, Some(files));
        assert_eq!(entries(root, &|_| None), None);
    }
}
