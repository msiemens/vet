//! What a session shows of a changeset: files with their review state, the
//! base and target, and the lifecycle.

use vet_core::completion::{Completion, HunkState};
use vet_core::content::{FileEntry, Hunk, LineDelta, NonTextKind, RepoPath, SnapshotId};
use vet_core::filter::ReviewFilter;

use crate::changeset::{BaseChoice, CommitId};
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
}

/// What the changes lead to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TargetView {
    /// "Working tree".
    pub name: String,
    /// The checked-out branch, if any.
    pub branch: Option<String>,
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
    pub head: CommitId,
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
