//! The changesets the repository offers to put on view: the working tree and the local
//! branches and the remote ones worth reviewing, each with what Vet knows of its review.

use super::{LiveSession, Result};
use crate::changeset::{CommitId, RemoteKey, Source};
use crate::mutex::lock;
use crate::ports::{BranchInfo, GitRepo, LiveStore, SourceReview};
use crate::review::ReviewSummary;

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
        /// Unix seconds of the first review action.
        started_at: i64,
        /// The number of the version the viewer last marked something in.
        last_reviewed: Option<u32>,
        /// How far the review was when its marks last changed; `None` if it never was measured.
        progress: Option<ReviewSummary>,
    },
    Finished {
        complete: bool,
        /// Unix seconds of the first review action.
        started_at: i64,
        finished_at: i64,
        last_reviewed: Option<u32>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangesetRow {
    pub source: Source,
    /// "Working tree", or the branch's name (`origin/feature` for a remote one).
    pub title: String,
    /// The branch is the checked-out one.
    pub checked_out: bool,
    /// The branch the working tree is on, if any; `None` for a branch.
    pub branch: Option<String>,
    /// Threads of the changeset that are open.
    pub open_threads: u32,
    /// `None` for the working tree, and for a branch when the repository has no default branch.
    pub divergence: Option<Divergence>,
    /// For the default branch: its commits that its upstream on the remote does not have.
    pub unpushed: Option<u32>,
    /// Unix seconds of the branch's tip commit.
    pub last_commit: Option<i64>,
    /// Who made the branch's tip commit; `None` for the working tree.
    pub author: Option<String>,
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

/// What a source's latest review is, as a row shows it.
fn row_review(state: &SourceReview) -> RowReview {
    match &state.latest {
        None => RowReview::None,
        Some(review) if review.closed_at.is_none() => RowReview::Open {
            version: state.latest_version.max(1),
            started_at: review.opened_at,
            last_reviewed: review.last_reviewed,
            progress: review.summary,
        },
        Some(review) => RowReview::Finished {
            complete: review.complete,
            started_at: review.opened_at,
            finished_at: review.closed_at.unwrap_or(review.opened_at),
            last_reviewed: review.last_reviewed,
        },
    }
}

/// Whether a changeset is kept although nothing else asks for it: it is the one on view, or its
/// review is open, so its commits and content may still be needed.
fn kept(source: &Source, review: &RowReview, on_view: &Source) -> bool {
    source == on_view || matches!(review, RowReview::Open { .. })
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The working tree, every local branch, and the remote branches worth reviewing.
    pub async fn changesets(&self) -> Result<ChangesetList> {
        let on_view = self.published.read().source.clone();
        let git = &self.acquisition.git;
        let default = match git.default_reference().await? {
            Some(reference) => git.resolve_commit(&reference).await?,
            None => None,
        };
        let default_branch = git.default_branch().await?;
        let all = git.branches().await?;
        // The review database is read once for the whole list, not once per row.
        let sources: Vec<Source> = std::iter::once(Source::WorkingTree)
            .chain(all.iter().map(|branch| Source::Branch {
                name: branch.name.clone(),
            }))
            .collect();
        let states = self
            .acquisition
            .store
            .source_reviews(self.repo.clone(), &sources)
            .await?;
        let state_of = |source: &Source| states.iter().find(|state| &state.source == source);
        let (working, threads) = match state_of(&Source::WorkingTree) {
            Some(state) => (row_review(state), state.open_threads),
            None => (RowReview::None, 0),
        };
        let mut rows = vec![ChangesetRow {
            review: working,
            open_threads: threads,
            source: Source::WorkingTree,
            title: "Working tree".into(),
            checked_out: false,
            branch: all.iter().find(|b| b.current).map(|b| b.name.clone()),
            divergence: None,
            unpushed: None,
            last_commit: None,
            author: None,
        }];
        let now = (self.clock)();
        let (mut local, mut remote) = (Vec::new(), Vec::new());
        // Where the default branch is on the remote it came from: the commits it has beyond that
        // are the ones to review on it.
        let upstream = default_branch.as_ref().and_then(|name| {
            let upstream = format!("origin/{name}");
            all.iter()
                .find(|b| b.remote && b.name == upstream)
                .map(|b| b.commit.clone())
        });
        for branch in all {
            let source = Source::Branch {
                name: branch.name.clone(),
            };
            let (review, open_threads) = match state_of(&source) {
                Some(state) => (row_review(state), state.open_threads),
                None => (RowReview::None, 0),
            };
            let keep = kept(&source, &review, &on_view);
            // The default branch holds what the others are compared with: it has something to
            // review only while it has commits that are not pushed yet.
            let unpushed = if !branch.remote && default_branch.as_deref() == Some(branch.name.as_str()) {
                Some(match &upstream {
                    Some(upstream) => self.behind(&branch.commit, upstream).await?,
                    None => 0,
                })
            } else {
                None
            };
            if unpushed == Some(0) && !keep {
                continue;
            }
            let tip = self.commit_info(&branch.commit).await?;
            let last_commit = tip.time;
            let age_days = now.saturating_sub(last_commit) / (24 * 60 * 60);
            if branch.remote && !keep && age_days > RECENT_DAYS {
                continue;
            }
            let divergence = match &default {
                Some(default) => Some(self.divergence(&branch, default).await?),
                None => None,
            };
            if branch.remote && !offered_remote(divergence, age_days, keep) {
                continue;
            }
            let group = if branch.remote { &mut remote } else { &mut local };
            group.push(ChangesetRow {
                title: branch.name,
                checked_out: branch.current,
                branch: None,
                open_threads,
                source,
                divergence,
                unpushed,
                last_commit: Some(last_commit),
                author: Some(tip.author),
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

    /// What Vet has of the latest review of each of `sources`, in their order. One batched read.
    pub async fn reviews_of(&self, sources: &[Source]) -> Result<Vec<RowReview>> {
        let states = self
            .acquisition
            .store
            .source_reviews(self.repo.clone(), sources)
            .await?;
        Ok(states.iter().map(row_review).collect())
    }

    /// The pull requests of `unlisted`, which were fetched before and a listing no longer has,
    /// whose refs may be dropped: neither the one on view nor one whose review is open, which may
    /// still need their commits before the platform can be asked again.
    pub async fn prunable(&self, unlisted: Vec<(RemoteKey, u64)>) -> Result<Vec<(RemoteKey, u64)>> {
        let sources: Vec<Source> = unlisted
            .iter()
            .map(|(remote, number)| Source::PullRequest {
                remote: remote.clone(),
                number: *number,
            })
            .collect();
        let reviews = self.reviews_of(&sources).await?;
        let on_view = self.published.read().source.clone();
        Ok(unlisted
            .into_iter()
            .zip(sources)
            .zip(reviews)
            .filter(|((_, source), review)| !kept(source, review, &on_view))
            .map(|((pull_request, _), _)| pull_request)
            .collect())
    }

    async fn divergence(&self, branch: &BranchInfo, default: &CommitId) -> Result<Divergence> {
        Ok(Divergence {
            ahead: self.behind(&branch.commit, default).await?,
            behind: self.behind(default, &branch.commit).await?,
        })
    }

    /// The commits of `tip` that `base` lacks. `MAX_COUNT` is the only cap, so the key leaves it out.
    async fn behind(&self, tip: &CommitId, base: &CommitId) -> Result<u32> {
        let key = (tip.clone(), base.clone());
        let found = lock(&self.behind).get(&key).copied();
        if let Some(count) = found {
            return Ok(count);
        }
        let count = self.acquisition.git.commits_behind(tip, base, MAX_COUNT).await? as u32;
        lock(&self.behind).insert(key, count);
        Ok(count)
    }
}
