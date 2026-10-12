//! `.gitattributes` of a snapshot, evaluated with gitoxide's attribute search over the files the
//! snapshot holds (not the ones on disk), so that a path is judged by the attributes of the side
//! it is on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gix::attrs::StateRef;
use gix::attrs::search::{MetadataCollection, Outcome};
use gix::bstr::ByteSlice;
use gix::glob::pattern::Case;
use vet_core::content::{BlobId, EntryKind, RepoPath, Snapshot};

const FILE: &[u8] = b".gitattributes";
const ATTRIBUTE: &str = "linguist-generated";

/// The `.gitattributes` files of `snapshot`, shallowest first, with their blobs.
pub(crate) fn files(snapshot: &Snapshot) -> Vec<(RepoPath, BlobId)> {
    let is_attributes = |path: &RepoPath| path.0.rsplit(|&b| b == b'/').next() == Some(FILE);
    let mut found: Vec<(RepoPath, BlobId)> = (snapshot.entries.iter())
        .filter(|(path, entry)| entry.kind == EntryKind::File && is_attributes(path))
        .map(|(path, entry)| (path.clone(), entry.blob))
        .collect();
    // A deeper file overrides the ones above it, so it is added after them.
    found.sort_by_key(|(path, _)| path.0.iter().filter(|&&b| b == b'/').count());
    found
}

/// Whether `linguist-generated` is set or unset for each of `paths` that the attribute reaches,
/// given the content of the files [`files`] found.
pub(crate) fn linguist_generated(files: &[(RepoPath, Vec<u8>)], paths: Vec<RepoPath>) -> BTreeMap<RepoPath, bool> {
    let mut collection = MetadataCollection::default();
    let mut search = gix::attrs::Search::default();
    for (path, bytes) in files {
        let Ok(source) = String::from_utf8(path.0.clone()) else {
            continue;
        };
        search.add_patterns_buffer(bytes, PathBuf::from(source), Some(Path::new("")), &mut collection, true);
    }
    let mut out = Outcome::default();
    out.initialize_with_selection(&collection, [ATTRIBUTE]);
    let mut found = BTreeMap::new();
    for path in paths {
        out.reset();
        search.pattern_matching_relative_path(path.0.as_bstr(), Case::Sensitive, Some(false), &mut out);
        // Linguist reads `=true` and `=false` like `set` and `unset`.
        let set = match out.iter_selected().next().map(|m| m.assignment.state) {
            Some(StateRef::Set) => Some(true),
            Some(StateRef::Unset) => Some(false),
            Some(StateRef::Value(v)) if v.as_bstr() == "true" => Some(true),
            Some(StateRef::Value(v)) if v.as_bstr() == "false" => Some(false),
            _ => None,
        };
        found.extend(set.map(|set| (path, set)));
    }
    found
}

#[cfg(test)]
mod tests;
