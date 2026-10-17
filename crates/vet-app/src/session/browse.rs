//! The directory tree of the target snapshot.

use std::collections::BTreeMap;

use vet_core::content::{EntryKind, LineDelta, RepoPath, entry_order};
use vet_core::filter::{PatternMatcher, ReviewFilter, is_under};

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

/// Directories first, then the others, each in the order of [`entry_order`].
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

/// What one glob of a draft filter matches in the changeset.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PatternMatch {
    /// The changed files it matches, text or not.
    pub files: u32,
    /// The lines added and removed in them.
    pub lines: u32,
    /// Why the glob is invalid; it then matches nothing.
    pub invalid: Option<String>,
}

/// What each root and glob of a draft filter matches, in the order the filter lists them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PatternMatches {
    pub revision: u32,
    /// For each root, the changed files at or below it.
    pub roots: Vec<u32>,
    pub include: Vec<PatternMatch>,
    pub exclude: Vec<PatternMatch>,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// What each include and exclude glob of `filter` matches, and how many changed files lie below
    /// each root, among the changed files of the changeset on view, whatever else the filter does.
    pub async fn pattern_matches(&self, filter: &ReviewFilter) -> Result<PatternMatches> {
        let (st, _) = self.viewing(View::Changeset).await?;
        let diff = &st.changes().diffed.diff;
        let mut lines: BTreeMap<&RepoPath, u32> = BTreeMap::new();
        for file in &diff.files {
            let delta = LineDelta::of(&file.hunks);
            *lines.entry(file.path()).or_default() += delta.added + delta.removed;
        }
        for change in &diff.non_text {
            lines.entry(&change.path).or_default();
        }
        let matched = |pattern: &String| match PatternMatcher::new(pattern) {
            Ok(matcher) => {
                let hit: Vec<u32> = lines
                    .iter()
                    .filter(|(p, _)| matcher.matches(p))
                    .map(|(_, n)| *n)
                    .collect();
                PatternMatch {
                    files: hit.len() as u32,
                    lines: hit.iter().sum(),
                    invalid: None,
                }
            }
            Err(invalid) => PatternMatch {
                files: 0,
                lines: 0,
                invalid: Some(invalid),
            },
        };
        Ok(PatternMatches {
            revision: st.revision,
            roots: filter
                .roots
                .iter()
                .map(|root| lines.keys().filter(|p| is_under(p, root)).count() as u32)
                .collect(),
            include: filter.include.iter().map(matched).collect(),
            exclude: filter.exclude.iter().map(matched).collect(),
        })
    }

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
        fn key(i: &DirItem) -> (bool, &[u8]) {
            (i.kind == DirEntryKind::Dir, i.name.as_bytes())
        }
        entries.sort_by(|a, b| entry_order(key(a), key(b)));
        Ok(DirListing {
            revision: st.revision,
            entries,
        })
    }
}
