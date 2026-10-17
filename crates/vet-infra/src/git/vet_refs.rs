//! `vet_app::ports::VetRefs`: the refs below `refs/vet/` of a repository found by its root, which
//! a removed project's folder may no longer be.

use std::path::{Path, PathBuf};

use vet_app::changeset::ChangesetId;
use vet_app::ports::{GitError, VetRefs};

use super::{chained, changeset_ref_prefix};
use crate::budget::Budget;
use crate::paths;

/// Where the refs of fetched pull requests are kept.
const PULL_REQUESTS: &str = "refs/vet/pr/";

#[derive(Clone)]
pub struct RepoRefs {
    io: Budget,
}

impl RepoRefs {
    /// Opens repositories on `io`'s threads.
    pub fn new(io: Budget) -> Self {
        Self { io }
    }
}

/// Deletes every ref below one of `prefixes`.
fn delete_below(root: &Path, prefixes: &[String]) -> Result<(), GitError> {
    let repo = gix::open(root).map_err(chained)?;
    let platform = repo.references().map_err(chained)?;
    let mut doomed = Vec::new();
    for prefix in prefixes {
        for reference in platform.prefixed(prefix.as_str()).map_err(chained)? {
            doomed.push(reference.map_err(chained)?);
        }
    }
    for reference in doomed {
        reference.delete().map_err(chained)?;
    }
    Ok(())
}

impl VetRefs for RepoRefs {
    async fn common_dir(&self, root: &str) -> Option<PathBuf> {
        let root = PathBuf::from(root);
        self.io
            .blocking(move || {
                let repo = gix::open(&root).ok()?;
                paths::canonical(repo.common_dir()).ok()
            })
            .await
            .ok()
            .flatten()
    }

    async fn delete_refs(&self, root: &str, changesets: Vec<ChangesetId>, pull_requests: bool) -> Result<(), GitError> {
        let root = PathBuf::from(root);
        let mut prefixes: Vec<String> = changesets.into_iter().map(changeset_ref_prefix).collect();
        if pull_requests {
            prefixes.push(PULL_REQUESTS.to_owned());
        }
        self.io
            .blocking(move || delete_below(&root, &prefixes))
            .await
            .map_err(chained)?
    }
}
