//! Which changeset is on view and which ones there are to put on view.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::LineDelta;

/// Names a changeset of a repository; stable across versions, rebases and restarts.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ChangesetKey {
    /// The repository's live tree.
    WorkingTree,
    /// The commits of a local branch, compared against where it left the default branch.
    Branch { name: String },
    /// A pull request of a hosted repository, compared against where its head left the branch it
    /// is merged into.
    PullRequest {
        /// The repository as `github.com/acme/app`: the host, then the path on it.
        remote: String,
        number: u32,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub time: u32,
}

/// How far a branch is from the repository's default branch.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Divergence {
    /// Commits on the branch that are not on the default branch.
    pub ahead: u32,
    /// Commits on the default branch that are not on the branch.
    pub behind: u32,
}

/// How far an open review is, as of its last change of marks, version or filter.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewProgress {
    /// Share of the changeset's lines and non-text items that are seen, in percent.
    pub seen_percent: f32,
    /// Share that is seen against an old base only, in percent.
    pub old_base_percent: f32,
}

/// What Vet has of a changeset's latest review.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum RowReview {
    None,
    Open {
        /// The number of the changeset's latest version.
        version: u32,
        /// Unix seconds of the first review action.
        started_at: u32,
        /// The number of the version the viewer last marked something in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        last_reviewed: Option<u32>,
        /// Absent for a review that was never measured.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        progress: Option<ReviewProgress>,
    },
    Finished {
        complete: bool,
        /// Unix seconds of the first review action.
        started_at: u32,
        finished_at: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        last_reviewed: Option<u32>,
    },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangesetRow {
    pub key: ChangesetKey,
    /// "Working tree", or the branch's name.
    pub title: String,
    /// The branch is the checked-out one.
    pub checked_out: bool,
    /// The branch the working tree is on; absent for a branch, and for a detached HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    /// Threads of the changeset that are open.
    pub open_threads: u32,
    /// Absent for the working tree, and for a branch when the repository has no default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub divergence: Option<Divergence>,
    /// Unix seconds of the branch's tip commit; absent for the working tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_commit: Option<u32>,
    /// Who made the branch's tip commit; absent for the working tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub author: Option<String>,
    pub review: RowReview,
}

/// The rolled-up state of a commit's checks.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CiState {
    Pending,
    Success,
    Failure,
    Neutral,
    Skipped,
}

/// Where a pull request stands for the viewer; the first that applies in this order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum QueueStatus {
    Draft,
    /// The platform says it cannot be merged as it is.
    Blocked,
    ChangesRequested,
    WaitingForYou,
    Approved,
}

/// How a reviewer holds a pull request back from approval.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChangesAsked {
    /// GitHub's review that requests changes.
    Requested,
    /// Azure DevOps' vote "Rejected".
    Rejected,
    /// Azure DevOps' vote "Waiting for author".
    WaitingForAuthor,
}

/// A reviewer who holds a pull request back from approval, and how.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangesRequest {
    /// As the platform shows the reviewer to people.
    pub reviewer: String,
    pub how: ChangesAsked,
}

/// An open pull request that waits for the viewer or is the viewer's own.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestRow {
    /// Opens it with `OpenChangeset`.
    pub key: ChangesetKey,
    pub title: String,
    pub author: String,
    /// The address of the author's picture, when the platform gives one a browser can load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub author_avatar: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    /// The repository the source branch is in, as the platform names it, when that is not the
    /// pull request's own: a fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fork: Option<String>,
    /// Absent when the platform's list does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lines: Option<LineDelta>,
    /// Comments on the pull request and on its code.
    pub comments: u32,
    /// The checks of the head commit; absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ci: Option<CiState>,
    pub draft: bool,
    /// Absent when none applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<QueueStatus>,
    /// Who asked for changes, and how, in the platform's order; empty unless that is the status.
    pub changes_requested: Vec<ChangesRequest>,
    /// Review is asked of the viewer or their team, and they have not reviewed it as it is now.
    pub waiting_for_me: bool,
    pub unresolved_threads: u32,
    /// Unix seconds the platform last saw it change.
    pub updated_at: u32,
    /// What Vet has of the viewer's review of it.
    pub review: RowReview,
    /// Where it is on the platform.
    pub url: String,
}

/// How asking one hosted repository for its pull requests went.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestSource {
    /// The repository as `github.com/acme/app`.
    pub remote: String,
    /// Why its pull requests could not be listed, fit to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// The error is that there is no credential for the host: the backend has asked for one with
    /// `CredentialsNeeded`.
    pub signed_out: bool,
}
