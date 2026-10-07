//! Run targets, approvals and run output.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::LineDelta;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// What to run and how (§10).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunTarget {
    /// Stable across snapshots, so an edit made to a discovered target keeps applying.
    pub id: String,
    /// The manifest it was discovered from, relative to the repository root;
    /// `None` for a target the user added. The backend ignores it in
    /// `SaveRunTarget`: discovery decides.
    pub source: Option<String>,
    pub label: String,
    /// The program and its arguments as one line: words split at whitespace, double
    /// quotes group (with `\"` and `\\`), single quotes group literally, nothing is
    /// expanded. The backend sends it formatted and splits what it receives.
    pub command: String,
    /// Relative to the repository root; empty for the root.
    pub cwd: String,
    pub env: Vec<EnvVar>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunTargetEntry {
    pub target: RunTarget,
    /// The user edited or added it: it is stored, and `ResetRunTarget` applies.
    pub stored: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunChange {
    Added,
    Modified,
    Removed,
}

/// An executor-relevant file that changed since the last approved run.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApprovalChange {
    pub path: String,
    pub change: RunChange,
}

/// One changed file of an approval prompt.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApprovalFile {
    pub path: String,
    /// The old path if git detected a rename; the diff is then against the text there,
    /// and the removed file has no entry of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,
    pub change: RunChange,
    /// The changed lines with two lines of context, between the last approved version
    /// and the one about to run. `None` if there is no text to compare (binary, or
    /// over the capture cap); empty for a file that only moved.
    pub diff: Option<ApprovalDiff>,
    pub delta: LineDelta,
}

/// The rows of a file's diff in an approval prompt.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApprovalDiff {
    pub rows: Vec<DiffRow>,
    /// The diff had more rows than are sent.
    pub truncated: bool,
}

/// `number` is a 1-based line number: of the new text for `Context` and `Added`, of the
/// old one for `Removed`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum DiffRow {
    Context {
        number: u32,
        text: String,
    },
    Added {
        number: u32,
        text: String,
    },
    Removed {
        number: u32,
        text: String,
    },
    /// Unchanged lines left out.
    Gap,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunStream {
    Stdout,
    Stderr,
}
