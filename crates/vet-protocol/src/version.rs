//! The versions of a changeset and the views of its files.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CommitInfo, LineDelta};

/// What a request about the changeset's files reads: the changeset as it is, what the author
/// changed between two of its versions (an interdiff), or what one of its commits changed. The
/// files of an interdiff are those of its manifest; their old side is version `from` rebased onto
/// the base of version `to`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum View {
    Changeset,
    /// Version `from` against the later version `to`; `from` 0 is the base the review started
    /// from, so the view shows all of the changes of `to`. Fails if the review has no such versions.
    Interdiff {
        from: u32,
        to: u32,
    },
    /// The commit at `index` of `Commits`, compared with its parent (a merge: with the automatic
    /// merge of its parents). Marks made in it count in the changeset wherever the same lines
    /// are in it, and the other way round; comments are made in the changeset. Fails if there
    /// is no such commit.
    Commit {
        index: u32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VersionOrigin {
    /// The tree settled, or the review started with it.
    Settled,
    /// A branch's tip or merge base was seen to have moved.
    Observed,
}

/// One version of the open review's changeset.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VersionInfo {
    /// 1-based; the latest is the one on view.
    pub n: u32,
    /// Unix seconds the version was recorded.
    pub at: u32,
    pub origin: VersionOrigin,
    /// The commit the version is built on; absent once git has dropped it.
    pub tip: Option<CommitInfo>,
    /// The commit of the version's base; absent once git has dropped it.
    pub base: Option<CommitInfo>,
    /// Something was last marked in this version, which is where the next review starts.
    pub reviewed_here: bool,
    /// What the version changed against the one before it (the first: against its base), by
    /// path. A move of the base is not a change.
    pub files: Vec<VersionFile>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VersionFile {
    pub path: String,
    /// Zero for a file without text lines to count (a mode change, a binary file).
    pub delta: LineDelta,
}

/// What an interdiff manifest says about itself beyond its files.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InterdiffInfo {
    pub from: u32,
    pub to: u32,
    /// The base moved between the versions, so the old side is version `from` rebased.
    pub rebased: bool,
    /// Marks and comments are possible: `to` is the latest version.
    pub markable: bool,
    /// What does not work in this view and why, fit to show (SPEC I6).
    pub limits: Vec<String>,
}
