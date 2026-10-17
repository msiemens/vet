//! What the repository says that does not depend on the review: the commits git has, how they
//! relate, and the diffs, interdiffs and line deltas derived from them. Nothing here reads review
//! state or asks for the session's token, so none of it waits for a command.

use std::sync::Arc;

use vet_core::content::Snapshot;
use vet_engine::{Engine, Limits};

use super::derived::{CommitUnit, CommitsBetween, Compared, Derived, Diffed, Interdiff, LinesBetween};
use super::lru::Lru;
use super::{Result, SessionError, UnixClock};
use crate::acquisition::Acquisition;
use crate::base;
use crate::changeset::{BaseChoice, CommitId, Source};
use crate::mutex::lock;
use crate::ports::{CommitInfo, GitError, GitRepo, LiveStore};
use crate::pull_request::PullRequestRefs;

/// Commits counted for how far a branch is from the default one.
const MAX_COUNT: usize = 10_000;

/// The commit that merged a tip into a branch, how many commits the merge brought in and how many
/// the branch has since.
type MergedAs = (CommitInfo, Option<u32>, Option<u32>);

/// How many line deltas the list of changesets keeps.
const KEPT_LINE_DELTAS: usize = 4096;

/// How many commit facts the session keeps; the one used longest ago goes first.
const KEPT_COMMIT_INFOS: usize = 1024;

/// How many merge facts the session keeps; the one used longest ago goes first.
const KEPT_MERGES: usize = 16;

/// How many ahead and behind counts the session keeps; the one used longest ago goes first.
const KEPT_BEHIND: usize = 4096;

/// How many commit counts the session keeps; the one used longest ago goes first.
const KEPT_COMMIT_COUNTS: usize = 16;

pub struct Facts<G, S> {
    /// What the facts are read through: the session's own handles.
    pub(super) acquisition: Arc<Acquisition<G, S>>,
    /// The repository root, which with a source is the key a changeset is stored under.
    pub(super) repo: String,
    pub(super) clock: UnixClock,
    /// The diffs, interdiffs and commits the review state is remapped onto.
    pub(super) derived: Arc<Derived>,
    /// The line deltas between commits that the list of changesets shows.
    pub(super) lines: Engine<LinesBetween>,
    /// What git said of the commits the manifests name, which it does not change.
    commit_infos: std::sync::Mutex<Lru<CommitId, CommitInfo>>,
    /// How a branch came to hold a tip, by the tip and the branch's commit, which git does not change.
    merges: std::sync::Mutex<Lru<(CommitId, CommitId), Option<MergedAs>>>,
    /// The commits reachable from the first commit and not from the second, counted up to the
    /// listing's cap. Git does not change it for the same two commits short of shallow deepening
    /// or `replace` refs.
    behind: std::sync::Mutex<Lru<(CommitId, CommitId), u32>>,
    /// How many commits lie on the first-parent line between a base and a tip, by the base
    /// commit and the tip, which git does not change.
    commit_counts: std::sync::Mutex<Lru<(CommitId, CommitId), u32>>,
}

impl<G, S> Facts<G, S> {
    pub(super) fn new(acquisition: &Arc<Acquisition<G, S>>, repo: String, clock: UnixClock) -> Self {
        Self {
            acquisition: Arc::clone(acquisition),
            repo,
            clock,
            derived: Arc::new(Derived::new(&acquisition.tasks)),
            lines: Engine::new(Limits::entries(KEPT_LINE_DELTAS)),
            commit_infos: std::sync::Mutex::new(Lru::new(KEPT_COMMIT_INFOS)),
            merges: std::sync::Mutex::new(Lru::new(KEPT_MERGES)),
            behind: std::sync::Mutex::new(Lru::new(KEPT_BEHIND)),
            commit_counts: std::sync::Mutex::new(Lru::new(KEPT_COMMIT_COUNTS)),
        }
    }
}

impl<G, S> Facts<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    /// What git says of `commit`, asked once while the session lives.
    pub(super) async fn commit_info(&self, commit: &CommitId) -> Result<CommitInfo> {
        let found = lock(&self.commit_infos).get(commit).cloned();
        if let Some(info) = found {
            return Ok(info);
        }
        let info = self.acquisition.git.commit_info(commit).await?;
        lock(&self.commit_infos).insert(commit.clone(), info.clone());
        Ok(info)
    }

    /// How the branch at `into` came to hold `tip`: the commit that merged it, how many commits
    /// the merge brought in and how many the branch has since. Asked once for the same two commits.
    pub(super) async fn merged_into_branch(&self, tip: &CommitId, into: &CommitId) -> Option<MergedAs> {
        const COUNTED: usize = 10_000;
        let key = (tip.clone(), into.clone());
        let kept = lock(&self.merges).get(&key).cloned();
        if let Some(merged) = kept {
            return merged;
        }
        let git = &self.acquisition.git;
        let merged = match base::merging_commit(git, tip, into).await {
            Ok(None) => None,
            Ok(Some(merging)) => {
                let counted = |n: std::result::Result<usize, GitError>| n.ok().map(|n| n as u32);
                let commits = match &merging.before {
                    Some(before) => counted(git.commits_behind(tip, before, COUNTED).await),
                    None => None,
                };
                let since = counted(git.commits_behind(into, &merging.commit, COUNTED).await);
                self.commit_info(&merging.commit)
                    .await
                    .ok()
                    .map(|info| (info, commits, since))
            }
            Err(e) => {
                tracing::warn!(error = %e, "the commit that merged the changeset is not found");
                return None;
            }
        };
        lock(&self.merges).insert(key, merged.clone());
        merged
    }

    /// How many commits lie on the first-parent line from `base` to `tip`, walked once while the
    /// session lives.
    pub(super) async fn commit_count(&self, base: &CommitId, tip: &CommitId) -> Result<u32> {
        let key = (base.clone(), tip.clone());
        let found = lock(&self.commit_counts).get(&key).copied();
        if let Some(count) = found {
            return Ok(count);
        }
        let count = self.acquisition.git.first_parent_range(base, tip).await?.len() as u32;
        lock(&self.commit_counts).insert(key, count);
        Ok(count)
    }

    /// The commits of `tip` that `base` lacks. `MAX_COUNT` is the only cap, so the key leaves it out.
    pub(super) async fn behind(&self, tip: &CommitId, base: &CommitId) -> Result<u32> {
        let key = (tip.clone(), base.clone());
        let found = lock(&self.behind).get(&key).copied();
        if let Some(count) = found {
            return Ok(count);
        }
        let count = self.acquisition.git.commits_behind(tip, base, MAX_COUNT).await? as u32;
        lock(&self.behind).insert(key, count);
        Ok(count)
    }

    /// The diff of `old` and `new` as the session reads it.
    pub(super) async fn diff_of(&self, old: &Arc<Snapshot>, new: &Arc<Snapshot>) -> Result<Arc<Diffed>> {
        self.derived.diff(&self.acquisition, old, new).await
    }

    /// The tree of `commit` as the session reads it: a snapshot already read is not walked again,
    /// and a walk under way is joined.
    pub(super) async fn snapshot_of(&self, commit: &CommitId) -> Result<Arc<Snapshot>> {
        Ok(self.derived.snapshot(&self.acquisition, commit).await?)
    }

    /// The interdiff of `key`, as [`Facts::diff_of`].
    pub(super) async fn interdiff_of(&self, key: Interdiff) -> Result<Arc<Compared>> {
        self.derived.interdiff(&self.acquisition, key).await
    }

    /// The commits of `key`, as [`Facts::diff_of`].
    pub(super) async fn commits_of(&self, key: CommitsBetween) -> Result<Arc<Vec<CommitUnit>>> {
        self.derived.commits(&self.acquisition, key).await
    }

    /// The base a changeset of `source`, whose tip is `tip`, is compared against by its kind: where
    /// a branch left the repository's default branch, or where a pull request's head left the
    /// fetched target. The working tree is not one: its base is pinned when it is opened. The one
    /// rule the review view and the queue's line counts use.
    pub(super) async fn base_for(&self, source: &Source, tip: &CommitId) -> Result<(BaseChoice, CommitId)> {
        let git = &self.acquisition.git;
        let resolved = match source {
            Source::PullRequest { remote, number } => {
                base::for_pull_request(git, &PullRequestRefs::of(remote, *number), tip).await
            }
            Source::Branch { .. } => base::for_branch(git, tip).await,
            Source::WorkingTree => {
                return Err(SessionError::Invalid(
                    "the working tree's base is pinned when it is opened".into(),
                ));
            }
        };
        Ok(resolved?)
    }
}
