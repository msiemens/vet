//! Git's ignore rules (`.gitignore`, `info/exclude`, `core.excludesFile`) over
//! gitoxide's exclude stack.

use gix::bstr::ByteSlice;
use gix::index::entry::Mode;
use gix::worktree::stack::state::ignore::Source;
use vet_app::ports::GitError;
use vet_core::content::{RepoPath, in_tool_store};

use super::err;
use crate::paths;

/// `paths` without those git ignores and without tool stores. A path with tracked files
/// at or below it is never ignored, as in git.
pub(crate) fn unignored(repo: &gix::Repository, paths: Vec<RepoPath>) -> Result<Vec<RepoPath>, GitError> {
    let index = repo.index_or_empty().map_err(err)?;
    let mut stack = repo
        .excludes(&index, None, Source::WorktreeThenIdMappingIfNotSkipped)
        .map_err(err)?;
    let root = repo
        .workdir()
        .ok_or_else(|| GitError::failed("bare repository"))?
        .to_owned();
    let mut kept = Vec::new();
    for path in paths {
        let rela = path.0.as_bstr();
        let is_dir = std::fs::symlink_metadata(root.join(paths::to_path(rela).map_err(err)?)).is_ok_and(|m| m.is_dir());
        let mut prefix = path.0.clone();
        prefix.push(b'/');
        let tracked = index.entry_by_path(rela).is_some()
            || (is_dir && index.prefixed_entries(prefix.as_bstr()).is_some_and(|e| !e.is_empty()));
        let mode = if is_dir { Mode::DIR } else { Mode::FILE };
        if tracked || (!in_tool_store(&path.0) && !stack.at_entry(rela, Some(mode)).map_err(err)?.is_excluded()) {
            kept.push(path);
        }
    }
    Ok(kept)
}
