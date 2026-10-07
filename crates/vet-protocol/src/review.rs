//! The changeset under review: its base, target, filter and progress.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{FileChange, FileReview, LineDelta, RestoreFailure, SessionId};

/// A side of the changeset.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Side {
    Base,
    Target,
}

/// What the review shows and counts (§5.2). A path passes if it is under one of
/// `roots` (or there are none), matches `include` (or there is none) and does not
/// match `exclude`. A glob without a `/` matches the file name at any depth, a
/// leading `/` anchors it at the root; `*` and `?` stop at `/`, `**` does not.
#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewFilter {
    /// Directories, `a/b` without a trailing slash.
    pub roots: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Binary changes are neither shown nor counted.
    pub hide_binary: bool,
}

/// How a session's base is chosen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum BaseChoice {
    /// The tip of a local branch as it is when chosen; the branch moving on
    /// does not move the base (`Base::behind`), choosing it again does.
    Branch { name: String },
    /// Where `HEAD` branched off `reference` (a branch or any revision): their
    /// merge base.
    MergeBase { reference: String },
    /// A commit: a full or abbreviated id, or any revision.
    Commit { id: String },
}

/// A base as resolved, with what to show of it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Base {
    /// A commit is reported by its full id.
    pub choice: BaseChoice,
    /// The base commit.
    pub commit: CommitInfo,
    /// What to call the commit: the branch if it is a branch tip, else
    /// `branch~N` on a local branch's first-parent history (the chosen branch,
    /// the current one, the default one, then the rest), else its short id.
    pub name: String,
    /// How many commits a chosen branch has gained since the base was chosen;
    /// absent unless the choice is a branch that has moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub behind: Option<u32>,
}

/// What the changes lead to: the live working tree.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Target {
    /// "Working tree".
    pub name: String,
    /// The checked-out branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
}

/// Whether a review has started. It starts with the first review action (a
/// mark, an acknowledgement, a new thread) and ends with `FinishReview`;
/// before and after, the session is browsing: the live diff, nothing kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ReviewInfo {
    Browsing,
    Open {
        /// Of the current snapshot: 1 is the review's start snapshot, each
        /// later settled snapshot adds one.
        version: u32,
        /// Unix seconds the version settled.
        updated_at: u32,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchInfo {
    pub name: String,
    /// Full id of the tip.
    pub commit: String,
    /// The checked-out branch.
    pub current: bool,
    /// The remote's default branch (`origin/HEAD`); never set without a remote.
    pub default: bool,
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

/// A file's new review state with the completion it implies, so a shell never
/// derives progress itself.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewUpdate {
    pub path: String,
    pub review: FileReview,
    /// The file's completion.
    pub completion: Completion,
    /// The changeset's completion after the change.
    pub changeset: Completion,
}

/// Progress against the review filter (§5.2): changed lines (counted per side)
/// and non-text items.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Completion {
    pub lines: u32,
    pub seen_lines: u32,
    /// Lines seen against an older base; they count as unseen in `seen_lines`.
    pub old_base_lines: u32,
    pub items: u32,
    pub acknowledged_items: u32,
}

/// The changeset as displayed through the repository's review filter.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Manifest {
    pub session: SessionId,
    /// Increases whenever the changeset's target changes.
    pub revision: u32,
    pub name: String,
    pub review: ReviewInfo,
    /// What the files are compared against.
    pub base: Base,
    /// What the files lead to.
    pub target: Target,
    pub filter: ReviewFilter,
    pub files: Vec<FileChange>,
    /// The lines `files` add and remove, together.
    pub delta: LineDelta,
    /// Completion of the whole changeset.
    pub completion: Completion,
    /// Changed lines and non-text items the filter hides, so "complete" never
    /// implies every change was seen (§5.2).
    pub excluded_lines: u32,
    pub excluded_items: u32,
    /// Set when the backend polls the tree instead of watching it, so changes
    /// show up late (§4.1, §6.4); the shell shows a banner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub degraded: Option<WatchDegraded>,
    /// Dependency restores that failed so far; the shell says analysis is limited
    /// (§7.5). `Event::RestoreFailed` updates it.
    #[serde(default)]
    pub restore_failures: Vec<RestoreFailure>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WatchDegraded {
    /// A WSL worktree under `/mnt/*`: file events do not cross 9P.
    MountedWindowsDrive,
    /// The OS watch limit is exhausted.
    WatchLimit,
}
