//! The changeset under review: its base, target, filter and progress.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    ChangesetKey, CommitInfo, Completion, FileChange, FileReview, InterdiffInfo, Lens, LineDelta, ProjectId,
    RestoreFailure,
};

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

/// How a review's base is chosen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum BaseChoice {
    /// Compared against `reference` (a branch or any revision): the merge base of
    /// it and the changeset's tip. The reference moving on does not move the base
    /// (`Base::behind`), choosing it again does.
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
    /// How many commits the chosen target branch has gained since the base;
    /// absent unless it has moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub behind: Option<u32>,
    /// Set when the chosen target already contains the changeset's tip: there is
    /// nothing to review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub contained_in: Option<String>,
}

/// What the changes lead to: the live working tree, or a branch's tip.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Target {
    /// Which changeset this is.
    pub changeset: ChangesetKey,
    /// "Working tree", or the branch's name.
    pub name: String,
    /// The branch the target is built on: the checked-out one for the working tree, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    /// The full id of the commit the target is: a branch's or a pull request's tip; absent for the
    /// working tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    /// A pull request's title as the platform last said it, kept across restarts; absent for any
    /// other changeset, and for a pull request the platform was never asked about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// A pull request that is a draft.
    pub draft: bool,
}

/// Whether a review has started. It starts with the first review action (a
/// mark, an acknowledgement, a new thread) and ends with `FinishReview`;
/// before and after, the project is browsing: the live diff, nothing kept.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ReviewInfo {
    Browsing,
    Open {
        /// Of the current snapshot: 1 is the review's start snapshot, each
        /// later settled snapshot and each base on another commit adds one.
        version: u32,
        /// Unix seconds the version was recorded.
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

/// A hunk seen against an old base: the base-side lines as they were when it was marked, as a diff
/// to the lines the base has now.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BaseDiff {
    /// The base the hunk was marked against; absent once git has dropped the commit.
    pub from: Option<CommitInfo>,
    /// The base now.
    pub to: CommitInfo,
    pub rows: Vec<BaseDiffRow>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BaseDiffRow {
    pub kind: BaseDiffKind,
    /// 1-based, in the old base for a removed line and in the current base for the others.
    pub line: u32,
    /// The line without its terminator.
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BaseDiffKind {
    Context,
    Removed,
    Added,
}

/// A file's new review state with the completion it implies, so a shell never
/// derives progress itself.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
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

/// The changeset as displayed through the repository's review filter.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Manifest {
    pub project: ProjectId,
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
    /// Set for the manifest of an interdiff view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub interdiff: Option<InterdiffInfo>,
    /// The lens the changeset is reviewed in; `PerCommit` only for a changeset of several commits.
    pub lens: Lens,
    /// How many commits lie between the base and the tip; none for the working tree.
    pub commits: u32,
    /// How many files the changeset as a whole changes, whatever view this manifest is of.
    pub changeset_files: u32,
    /// The completion of the changeset as a whole, whatever view this manifest is of.
    pub changeset_completion: Completion,
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
