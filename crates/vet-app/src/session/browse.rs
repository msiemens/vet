//! The directory tree of the target snapshot.

use vet_core::content::{EntryKind, RepoPath};

use super::{LiveSession, Result, SessionError, View};
use crate::ports::{GitRepo, LiveStore};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirEntryKind {
    File,
    Dir,
    Symlink,
    Submodule,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DirItem {
    pub name: String,
    pub kind: DirEntryKind,
    /// The path, or anything below a directory, is in the changeset.
    pub changed: bool,
    /// The review filter excludes the path, or everything below a directory.
    pub excluded: bool,
}

/// Directories first, then the others, each by name in byte order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DirListing {
    pub revision: u32,
    pub entries: Vec<DirItem>,
}

/// The files of the target snapshot by path, in byte order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileListing {
    pub revision: u32,
    pub paths: Vec<String>,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Every file (not a symlink or a submodule) of the target snapshot, for a search by name.
    pub async fn list_files(&self) -> Result<FileListing> {
        let (st, _) = self.viewing(View::Changeset).await?;
        let paths = st
            .target
            .entries
            .iter()
            .filter(|(_, entry)| entry.kind == EntryKind::File)
            .map(|(path, _)| String::from_utf8_lossy(&path.0).into_owned())
            .collect();
        Ok(FileListing {
            revision: st.revision,
            paths,
        })
    }

    /// The children of `path` (`""` is the root) in the target snapshot.
    pub async fn list_directory(&self, path: &str) -> Result<DirListing> {
        let (st, _) = self.viewing(View::Changeset).await?;
        let changes = &st.changes().diffed;
        let compiled = st.filter.compile(&changes.diff.generated);
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let prefix = prefix.as_bytes();
        let has_prefix = |p: &RepoPath, below: &[u8]| p.0.starts_with(below);

        let mut entries = Vec::new();
        let mut next = RepoPath(prefix.to_vec());
        while let Some((key, entry)) = st.target.entries.range(next.clone()..).next() {
            let Some(rest) = key.0.strip_prefix(prefix) else { break };
            let (name, below) = match rest.iter().position(|&b| b == b'/') {
                None => {
                    // Skips to the first key after this one.
                    next = RepoPath([key.0.as_slice(), &[0]].concat());
                    let kind = match entry.kind {
                        EntryKind::File => DirEntryKind::File,
                        EntryKind::Symlink => DirEntryKind::Symlink,
                        EntryKind::Submodule => DirEntryKind::Submodule,
                    };
                    entries.push(DirItem {
                        name: String::from_utf8_lossy(rest).into_owned(),
                        kind,
                        changed: changes.changed.contains(key),
                        excluded: !compiled.passes(key),
                    });
                    continue;
                }
                Some(i) => (&rest[..i], [&key.0[..prefix.len() + i], b"/"].concat()),
            };
            // Skips everything below the directory: `0` is the byte after `/`.
            next = RepoPath([&below[..below.len() - 1], b"0"].concat());
            let below = RepoPath(below);
            entries.push(DirItem {
                name: String::from_utf8_lossy(name).into_owned(),
                kind: DirEntryKind::Dir,
                changed: changes
                    .changed
                    .range(below.clone()..)
                    .next()
                    .is_some_and(|p| has_prefix(p, &below.0)),
                excluded: st
                    .target
                    .entries
                    .range(below.clone()..)
                    .take_while(|(p, _)| has_prefix(p, &below.0))
                    .all(|(p, _)| !compiled.passes(p)),
            });
        }
        if entries.is_empty() && !path.is_empty() {
            return Err(SessionError::NotFound(format!("directory {path}")));
        }
        entries.sort_by(|a, b| {
            let key = |i: &DirItem| (i.kind != DirEntryKind::Dir, i.name.clone().into_bytes());
            key(a).cmp(&key(b))
        });
        Ok(DirListing {
            revision: st.revision,
            entries,
        })
    }
}
