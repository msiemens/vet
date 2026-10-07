//! The timeline: the versions of the open review, for choosing what to compare, and what each
//! one changed.

use std::collections::BTreeMap;

use vet_core::content::{BlobId, Snapshot};
use vet_core::merkle::empty_snapshot;
use vet_core::version_diff::{VersionSides, version_diff};

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

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// The versions of the open review, oldest first; none while browsing, as versions are
    /// recorded only while a review is open.
    pub async fn versions(&self) -> Result<Vec<VersionView>> {
        let (changeset, last_reviewed) = {
            let st = self.state.lock().await;
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
    /// of the version before it; with none, against its own base. Content that cannot be read
    /// leaves its file counted as a change without lines.
    pub(super) async fn change_of(
        &self,
        previous: Option<(&Snapshot, &Snapshot)>,
        base: &Snapshot,
        target: &Snapshot,
    ) -> VersionChange {
        let empty = empty_snapshot();
        let (base_a, target_a) = previous.unwrap_or((&empty, &empty));
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
            let _ = self.load(&mut blobs, &path, blob).await;
        }
        let files = version_diff(sides, &paths, &mut blobs).changed_files();
        VersionChange {
            files: files
                .into_iter()
                .map(|(path, delta)| ChangedFile { path, delta })
                .collect(),
        }
    }
}
