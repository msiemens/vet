//! The timeline: the versions of the open review, for choosing what to compare, and what each
//! one changed.

use std::collections::BTreeMap;
use std::sync::Arc;

use vet_core::content::{BlobId, LineDelta, RepoPath, Snapshot};
use vet_core::version_diff::{VersionSides, version_diff};

use super::derived::Diffed;
use super::{LiveSession, Result};
use crate::changeset::{ChangedFile, VersionChange, VersionOrigin};
use crate::ports::{CommitInfo, GitRepo, LiveStore};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VersionView {
    /// 1-based, per changeset; the latest is the one on view.
    pub n: u32,
    /// Unix seconds.
    pub at: i64,
    pub origin: VersionOrigin,
    /// The commit the version is built on; `None` once git has dropped it.
    pub tip: Option<CommitInfo>,
    /// The commit of the version's base; `None` once git has dropped it.
    pub base: Option<CommitInfo>,
    /// Something was last marked in this version.
    pub reviewed_here: bool,
    pub change: VersionChange,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The versions of the open review, oldest first; none while browsing, as versions are
    /// recorded only while a review is open.
    pub async fn versions(&self) -> Result<Vec<VersionView>> {
        let (changeset, last_reviewed) = {
            let st = self.hold().await;
            match (st.changeset, &st.review) {
                (Some(changeset), Some(review)) => (changeset, review.last_reviewed),
                _ => return Ok(Vec::new()),
            }
        };
        let git = &self.acquisition.git;
        let mut views = Vec::new();
        for v in self.acquisition.store.versions(changeset).await? {
            views.push(VersionView {
                n: v.n,
                at: v.at,
                origin: v.origin,
                tip: git.commit_info(&v.tip).await.ok(),
                base: git.commit_info(&v.base_commit).await.ok(),
                reviewed_here: last_reviewed == Some(v.n),
                change: v.change,
            });
        }
        Ok(views)
    }

    /// What the version of `base` and `target` changed against `previous`, the base and the target
    /// of the version before it. Content that cannot be read leaves its file counted as a change
    /// without lines.
    pub(super) async fn change_of(
        &self,
        (base_a, target_a): (&Arc<Snapshot>, &Arc<Snapshot>),
        base: &Arc<Snapshot>,
        target: &Arc<Snapshot>,
    ) -> Result<VersionChange> {
        let sides = VersionSides {
            base_a,
            target_a,
            base_b: base,
            target_b: target,
        };
        let paths = sides.paths();
        let mut blobs: BTreeMap<BlobId, Vec<u8>> = BTreeMap::new();
        for (path, blob) in sides.blobs(&paths) {
            // Unreadable content is a file without text.
            let _ = self.acquisition.load(&mut blobs, &path, blob).await;
        }
        Ok(change(version_diff(sides, &paths, &mut blobs).diff.changed_files()))
    }
}

/// What the first version of a review changed: the changeset's diff.
pub(super) fn first_change(diffed: &Diffed) -> VersionChange {
    change(diffed.diff.changed_files())
}

fn change(files: Vec<(RepoPath, LineDelta)>) -> VersionChange {
    VersionChange {
        files: files
            .into_iter()
            .map(|(path, delta)| ChangedFile { path, delta })
            .collect(),
    }
}
