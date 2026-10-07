//! Which changeset is on view and which ones there are to put on view.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Names a changeset of a repository; stable across versions, rebases and restarts.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ChangesetKey {
    /// The repository's live tree.
    WorkingTree,
    /// The commits of a local branch, compared against where it left the default branch.
    Branch { name: String },
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

/// What Vet has of a changeset's latest review.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
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

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
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
    pub review: RowReview,
}
