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
/// leading `/` anchors it at the root; `*` and `?` stop at `/`, `**` does not. Unless
/// `review_generated` is set, lockfiles and generated files are excluded as well.
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
    /// Lockfiles and generated files are reviewed like any other file. Otherwise the ones named
    /// like a lockfile or a tool's output, with `linguist-generated` set in `.gitattributes`, or
    /// with a header that says a tool wrote them are excluded.
    pub review_generated: bool,
}

/// What one glob of a draft filter matches among the changed files of the changeset on view.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PatternMatch {
    /// The changed files it matches, text or not.
    pub files: u32,
    /// The lines added and removed in them.
    pub lines: u32,
    /// Why the glob is invalid, in one short line; it then matches nothing.
    pub invalid: Option<String>,
}

/// What a view is compared against. The base is fixed by the kind of changeset and never chosen.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Base {
    /// The base commit.
    pub commit: CommitInfo,
    /// Set when the branch the base was taken from already contains the changeset's tip: there
    /// is nothing to review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub contained_in: Option<Contained>,
    /// The branch a pull request is merged into, when that is not the repository's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub into: Option<String>,
    /// The working tree's `HEAD` no longer builds on the base, so what is shown includes work that
    /// came from elsewhere (a rebase, a reset).
    pub head_off_base: bool,
}

/// The branch that already holds a changeset's tip.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Contained {
    /// The branch's short name.
    pub branch: String,
    /// The changeset's tip is the branch's tip: it has no commit of its own.
    pub same_tip: bool,
    /// The commit that brought the tip into the branch; absent when the branch only advanced past
    /// the tip, or git cannot say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub merged_by: Option<CommitInfo>,
    /// How many commits the merge brought into the branch; absent when `merged_by` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub commits: Option<u32>,
    /// How many commits the branch has since the merge; absent when `merged_by` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub since: Option<u32>,
}

/// Why the review leaves a file out.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum ExcludedBy {
    /// The project's filter shows it, but this review is narrowed to something else.
    Narrowing,
    /// Not under any of the filter's review roots.
    OutsideRoots,
    /// Matches none of the filter's include globs.
    NotIncluded,
    /// Matches this exclude glob of the filter.
    Pattern { pattern: String },
    /// A lockfile or a generated file.
    Generated,
    /// A binary change, hidden by the filter.
    Binary,
}

/// A file left out of the review, with the lines it changes.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExcludedFile {
    pub path: String,
    pub delta: LineDelta,
}

/// The files of an excluded group under one folder, or one file at the repository root.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExcludedFolder {
    pub path: String,
    /// A folder; otherwise `path` is a file at the repository root.
    pub dir: bool,
    pub files: u32,
    pub delta: LineDelta,
}

/// The files left out of the review by the same rules: lifting all of `by` brings them back.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExcludedGroup {
    pub by: Vec<ExcludedBy>,
    /// How many files, of which `files` lists the first ones.
    pub count: u32,
    pub delta: LineDelta,
    pub files: Vec<ExcludedFile>,
    /// All the group's files by folder, the largest first, at the depth of the narrowing.
    pub folders: Vec<ExcludedFolder>,
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

/// A hunk seen against an old base: the base-side lines as they were when it was marked, as a diff
/// to the lines the base has now.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BaseDiff {
    /// The revision the diff is of.
    pub revision: u32,
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
    /// The project's change counter as of the state this was read from. A client applies an update
    /// only if it is newer than what it holds of the file.
    #[ts(type = "number")]
    pub seq: u64,
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
    /// The project's change counter as of the state this was read from: the sequence of the
    /// newest review update it reflects. A client keeps the state it holds of a file if that is
    /// newer.
    #[ts(type = "number")]
    pub seq: u64,
    pub name: String,
    pub review: ReviewInfo,
    /// What the files are compared against.
    pub base: Base,
    /// What the files lead to.
    pub target: Target,
    /// The project's filter.
    pub filter: ReviewFilter,
    /// The filter this review is narrowed to (`NarrowReview`); absent unless it is. It replaces
    /// the project's filter until reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub narrowing: Option<ReviewFilter>,
    pub files: Vec<FileChange>,
    /// The files the filter leaves out, by the reason.
    pub excluded: Vec<ExcludedGroup>,
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
    /// What analysis of the changeset may execute (§7.5). `TargetChanged` updates it.
    pub trust: AnalysisTrust,
    /// Set for the manifest of an interdiff view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub interdiff: Option<InterdiffInfo>,
    /// The lens the changeset is reviewed in; `PerCommit` only for a changeset of several commits.
    pub lens: Lens,
    /// How many commits lie between the base and the tip; none for the working tree.
    pub commits: u32,
    /// The base of the changeset as a whole, whatever view this manifest is of.
    pub changeset_base: Base,
    /// How many files the changeset as a whole changes, whatever view this manifest is of.
    pub changeset_files: u32,
    /// The completion of the changeset as a whole, whatever view this manifest is of.
    pub changeset_completion: Completion,
}

/// What may execute project code when the changeset on view is analysed (§7.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnalysisTrust {
    /// The project (the repository) is marked trusted.
    pub project: bool,
    /// Set when the changeset is a pull request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pull_request: Option<PullRequestTrust>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestTrust {
    /// The head is in a repository other than the one the pull request is opened against,
    /// or that is not known.
    pub fork: bool,
    /// Analysis of the pull request's current head may execute project code. A new push
    /// ends what "Trust this PR" granted.
    pub trusted: bool,
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
