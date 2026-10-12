//! What a session shows of a changeset: files with their review state, the
//! base and target, and the lifecycle.

use vet_core::completion::{Completion, HunkState};
use vet_core::content::{FileEntry, Hunk, LineDelta, NonTextKind, RepoPath, SnapshotId};
use vet_core::filter::ReviewFilter;

use crate::changeset::{BaseChoice, CommitId, Lens, Source};
use crate::ports::{CommitInfo, Degraded};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReviewView {
    /// One per hunk of the file.
    pub hunks: Vec<HunkState>,
    /// One per hunk of the file: its changed lines covered only by spans seen against an old base.
    pub old_base_lines: Vec<u32>,
    /// One per hunk of the file: it is flagged for later review.
    pub flagged: Vec<bool>,
    pub acknowledged: Vec<NonTextKind>,
}

/// One side of a non-text change, with what a card needs beyond the entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EntryInfo {
    pub entry: FileEntry,
    /// Where a symlink points, or the commit a submodule is pinned at.
    pub target: Option<String>,
    /// Content size of a binary, renamed or mode-changed file.
    pub size: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ItemView {
    pub kind: NonTextKind,
    pub before: Option<EntryInfo>,
    pub after: Option<EntryInfo>,
    /// An image with a side to show: a card previews it.
    pub has_preview: bool,
    /// For a `Mode` item.
    pub mode_change: Option<ModeChange>,
}

/// How a file's mode changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModeChange {
    GainedExecutable,
    LostExecutable,
    Other,
}

impl ModeChange {
    pub(super) fn of(before: Option<&FileEntry>, after: Option<&FileEntry>) -> Self {
        let executable = |e: Option<&FileEntry>| e.is_some_and(|e| e.mode & 0o111 != 0);
        match (executable(before), executable(after)) {
            (false, true) => Self::GainedExecutable,
            (true, false) => Self::LostExecutable,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileView {
    pub path: RepoPath,
    pub old_path: Option<RepoPath>,
    pub status: FileStatus,
    pub base_lines: u32,
    pub target_lines: u32,
    pub hunks: Vec<Hunk>,
    pub delta: LineDelta,
    pub non_text: Vec<ItemView>,
    pub review: ReviewView,
    pub completion: Completion,
}

/// The session's base as chosen and as resolved, with what to show of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BaseView {
    pub choice: BaseChoice,
    pub commit: CommitInfo,
    /// What a person calls the commit: a branch, `branch~N` or the short hash.
    pub name: String,
    /// How many commits a chosen branch has gained since; `None` unless it moved.
    pub behind: Option<u32>,
    /// The chosen target when it already contains the changeset's tip, so the changeset is empty.
    pub contained_in: Option<String>,
}

/// What the changes lead to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetView {
    pub source: Source,
    /// "Working tree", or the branch's name.
    pub name: String,
    /// The branch the target is built on: the checked-out one for the working tree, if any.
    pub branch: Option<String>,
    /// The full id of a branch's or a pull request's tip; `None` for the working tree.
    pub head: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lifecycle {
    /// No review has started; the diff is live and nothing is kept.
    Browsing,
    Open {
        /// Of the current snapshot: 1 is the review's start.
        version: u32,
        /// Unix seconds the version was recorded.
        updated_at: i64,
    },
}

/// The changeset through the review filter; hidden files are only counted in
/// `completion`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ManifestView {
    pub revision: u32,
    pub name: String,
    pub lifecycle: Lifecycle,
    pub base: BaseView,
    pub target: TargetView,
    pub filter: ReviewFilter,
    pub files: Vec<FileView>,
    /// The lines the shown files add and remove.
    pub delta: LineDelta,
    pub completion: Completion,
    /// Why the tree is polled instead of watched, if it is.
    pub degraded: Option<Degraded>,
    /// Set when the manifest is of an interdiff.
    pub interdiff: Option<InterdiffView>,
    /// The lens the changeset is reviewed in.
    pub lens: Lens,
    /// How many commits lie between the base and the tip; none for the working tree.
    pub commits: u32,
    /// How many files the changeset as a whole changes, whatever view the manifest is of.
    pub changeset_files: u32,
    /// The completion of the changeset as a whole, whatever view the manifest is of.
    pub changeset_completion: Completion,
}

/// What an interdiff view says about itself beyond its files.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InterdiffView {
    pub from: u32,
    pub to: u32,
    /// The base moved between the versions, so the old side is version `from` rebased.
    pub rebased: bool,
    /// Marks and comments land in the changeset of version `to`, which only the latest has.
    pub markable: bool,
    /// What does not work in this view and why, fit to show (SPEC I6).
    pub limits: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReviewUpdateView {
    pub revision: u32,
    pub path: RepoPath,
    pub review: ReviewView,
    pub completion: Completion,
    pub changeset: Completion,
}

/// The two snapshots of a revision, each with the commit it is built on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SemanticSides {
    pub base: SnapshotId,
    pub base_commit: CommitId,
    pub target: SnapshotId,
    /// The commit the target is built on: the tip of a branch, `HEAD` for the working tree.
    pub head: CommitId,
    /// The target is the live tree, so its servers read the worktree; otherwise it is a
    /// commit's tree, served from a materialized snapshot like the base.
    pub live_target: bool,
}

/// The result of a request that can start the review.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome<T> {
    pub value: T,
    /// The review as it began, if this request started it.
    pub started: Option<Lifecycle>,
}

impl<T> std::ops::Deref for Outcome<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}
