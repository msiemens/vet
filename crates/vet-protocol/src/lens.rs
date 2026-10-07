//! The lenses a committed changeset is reviewed through, and its commits.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CommitInfo, Completion, LineDelta};

/// How a committed changeset is cut for review: all of it at once ("All changes"), or one unit
/// per commit ("By commit").
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Lens {
    Squash,
    PerCommit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CommitKind {
    Commit,
    Merge {
        /// The merge equals the automatic merge of its parents: there is nothing to review.
        clean: bool,
        /// Its diff is against the automatic merge of its parents. Without it (a merge of more
        /// than two parents) the diff is against its first parent.
        remerged: bool,
    },
}

/// A commit of the changeset with its own diff: what it changed against its parent (a merge:
/// against the automatic merge of its parents) and how much of that is seen.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitUnit {
    pub commit: CommitInfo,
    /// The message after its subject.
    pub body: String,
    /// Full ids; a merge has several.
    pub parents: Vec<String>,
    pub kind: CommitKind,
    pub delta: LineDelta,
    pub completion: Completion,
}
