//! The changesets the repository offers to put on view: the working tree and the local
//! branches and the remote ones worth reviewing, each with what Vet knows of its review.

use super::{LiveSession, Result};
use crate::changeset::{CommitId, Source};
use crate::ports::{BranchInfo, GitRepo, LiveStore};

/// A remote branch whose latest commit is older than this many days is not offered.
const RECENT_DAYS: i64 = 30;
/// Commits counted for how far a branch is from the default one.
const MAX_COUNT: usize = 10_000;

/// How far a branch is from the repository's default branch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Divergence {
    /// Commits on the branch that are not on the default branch.
    pub ahead: u32,
    /// Commits on the default branch that are not on the branch.
    pub behind: u32,
}

/// What Vet has of a changeset's latest review.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowReview {
    None,
    Open {
        /// The number of the changeset's latest version.
        version: u32,
    },
    Finished {
        complete: bool,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangesetRow {
    pub source: Source,
    /// "Working tree", or the branch's name (`origin/feature` for a remote one).
    pub title: String,
    /// The branch is the checked-out one.
    pub checked_out: bool,
    /// `None` for the working tree, and for a branch when the repository has no default branch.
    pub divergence: Option<Divergence>,
    /// Unix seconds of the branch's tip commit.
    pub last_commit: Option<i64>,
    pub review: RowReview,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangesetList {
    /// The changeset on view.
    pub on_view: Source,
    /// The working tree first, then the local branches and then the remote ones, each group
    /// with the most recently committed first.
    pub rows: Vec<ChangesetRow>,
}

/// Whether a remote branch is offered: it has commits the default branch lacks and was
/// committed to recently, or it is on view or has a review open. Local branches are all offered.
fn offered_remote(divergence: Option<Divergence>, age_days: i64, keep: bool) -> bool {
    keep || (divergence.is_none_or(|d| d.ahead > 0) && age_days <= RECENT_DAYS)
}

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// The working tree, every local branch, and the remote branches worth reviewing.
    pub async fn changesets(&self) -> Result<ChangesetList> {
        let on_view = self.state.lock().await.source.clone();
        let git = &self.acquisition.git;
        let default = match git.default_reference().await? {
            Some(reference) => git.resolve_commit(&reference).await.ok(),
            None => None,
        };
        let mut rows = vec![ChangesetRow {
            review: self.row_review(&Source::WorkingTree).await?,
            source: Source::WorkingTree,
            title: "Working tree".into(),
            checked_out: false,
            divergence: None,
            last_commit: None,
        }];
        let now = (self.clock)();
        let (mut local, mut remote) = (Vec::new(), Vec::new());
        for branch in git.branches().await? {
            let source = Source::Branch {
                name: branch.name.clone(),
            };
            let review = self.row_review(&source).await?;
            let divergence = match &default {
                Some(default) => Some(self.divergence(&branch, default).await?),
                None => None,
            };
            let last_commit = git.commit_info(&branch.commit).await?.time;
            let age_days = now.saturating_sub(last_commit) / (24 * 60 * 60);
            let keep = source == on_view || matches!(review, RowReview::Open { .. });
            if branch.remote && !offered_remote(divergence, age_days, keep) {
                continue;
            }
            let group = if branch.remote { &mut remote } else { &mut local };
            group.push(ChangesetRow {
                title: branch.name,
                checked_out: branch.current,
                source,
                divergence,
                last_commit: Some(last_commit),
                review,
            });
        }
        for group in [&mut local, &mut remote] {
            group.sort_by_key(|b| std::cmp::Reverse(b.last_commit));
        }
        rows.extend(local);
        rows.extend(remote);
        Ok(ChangesetList { on_view, rows })
    }

    async fn divergence(&self, branch: &BranchInfo, default: &CommitId) -> Result<Divergence> {
        let git = &self.acquisition.git;
        let ahead = git.commits_behind(&branch.commit, default, MAX_COUNT).await?;
        let behind = git.commits_behind(default, &branch.commit, MAX_COUNT).await?;
        Ok(Divergence {
            ahead: ahead as u32,
            behind: behind as u32,
        })
    }

    async fn row_review(&self, source: &Source) -> Result<RowReview> {
        let store = &self.acquisition.store;
        let Some(changeset) = store.changeset(self.repo.clone(), source.clone()).await? else {
            return Ok(RowReview::None);
        };
        let Some(review) = store.latest_review(changeset).await? else {
            return Ok(RowReview::None);
        };
        let Some(review) = store.review(review).await? else {
            return Ok(RowReview::None);
        };
        Ok(match review.closed_at {
            None => RowReview::Open {
                version: store.versions(changeset).await?.last().map_or(1, |v| v.n),
            },
            Some(_) => RowReview::Finished {
                complete: review.complete.unwrap_or(false),
            },
        })
    }
}
