//! Background work the backend reports (SPEC "Background work is visible").

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ProjectId;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TaskKind {
    /// Reading the working tree.
    Scan,
    /// Comparing the base with the target.
    Diff,
    /// Fetching a project's dependencies for a language server.
    Restore,
    /// Starting a language server.
    ServerStart,
    /// A language server indexing the project.
    ServerIndex,
    /// Fetching the repository's remotes.
    Fetch,
    /// Keeping the working tree's changed files in Vet's store, ahead of a review.
    Store,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum TaskState {
    Running,
    Finished,
    /// The step ended without its result. A failure the viewer should act on is
    /// reported where it affects something (for example `RestoreFailed`).
    Failed {
        /// Fit to show.
        message: String,
    },
}

/// One step of background work. A task is announced with `Event::Task` when it
/// starts, as it progresses (at most a few times a second) and when it ends, in
/// that order; `Request::Tasks` lists the running ones.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Task {
    pub project: ProjectId,
    /// Unique among the tasks of the project, and the same in every announcement
    /// of the task.
    pub id: u32,
    pub kind: TaskKind,
    /// Fit to show: names the step, like "Indexing rust-analyzer".
    pub title: String,
    /// `done` of `total` units are complete; both are set or neither is, and
    /// without them the progress is indeterminate.
    pub done: Option<u32>,
    pub total: Option<u32>,
    /// What the step is doing now, when the tool says.
    pub detail: Option<String>,
    pub state: TaskState,
}
