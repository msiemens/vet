//! The timeline: the versions of the open review, for choosing what to compare.

use super::{LiveSession, Result};
use crate::changeset::VersionOrigin;
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
            });
        }
        Ok(views)
    }
}
