//! The checks of commits on a hosting platform: CI runs and statuses.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CiState;

/// How far a check, a run or a step has come, which its state alone does not say.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CheckPhase {
    /// Waiting for a runner or an agent.
    Queued,
    Running,
    Finished,
    Cancelled,
}

/// A run of a pipeline (a GitHub workflow run, an Azure DevOps pipeline run) whose jobs are
/// `CheckRun`s with its `id` as `pipeline`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Pipeline {
    /// Names the run in `PipelineAction`.
    pub id: String,
    pub name: String,
    /// What tells this run from the others of its pipeline: its number and why it ran.
    pub detail: String,
    /// 1 for the first execution, higher after a re-run.
    pub attempt: u32,
    /// What the jobs of the run make together.
    pub state: CiState,
    pub phase: CheckPhase,
}

/// What the viewer asks a platform to do with a run.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum PipelineAction {
    /// Runs the failed jobs of a `Pipeline` again.
    RerunFailed { run: String },
    /// Runs all jobs of a finished `Pipeline` again.
    RerunAll { run: String },
    /// Runs one job (a `CheckRun::id` with a `pipeline`) again.
    RerunJob { check: String },
    /// Stops a `Pipeline` that has not finished.
    Cancel { run: String },
}

/// What the platform made of a `PipelineAction`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum PipelineOutcome {
    /// The platform took the request; what it does shows in the next `Checks`.
    Accepted,
    /// The token is valid but may not change runs.
    Forbidden { message: String },
    /// The platform asks to wait; unix seconds.
    RateLimited { message: String, until: Option<u32> },
}

/// One step of a job, or one task of a stage.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PipelineStep {
    pub name: String,
    pub state: CiState,
    pub phase: CheckPhase,
    /// Seconds it ran for; absent while it runs or waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub duration: Option<u32>,
}

/// One check of a commit.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckRun {
    /// Names the run in `CheckLog`.
    pub id: String,
    pub name: String,
    pub state: CiState,
    pub phase: CheckPhase,
    /// The `Pipeline::id` this is a job of; absent for a status a service set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pipeline: Option<String>,
    /// Seconds it ran for; absent while it runs, or when the platform does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub duration: Option<u32>,
    /// Where the platform shows the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url: Option<String>,
    /// `CheckLog` can fetch its log.
    pub log: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitChecks {
    /// The full commit id, as asked for.
    pub commit: String,
    /// The combined state of the runs; absent for a commit without any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub state: Option<CiState>,
    pub runs: Vec<CheckRun>,
    /// The runs the jobs among `runs` belong to, in the order of their first job.
    pub pipelines: Vec<Pipeline>,
    /// Where the platform lists the commit's checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url: Option<String>,
}

/// Where a log shows what failed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogFailure {
    /// 0-based line of `CheckLog::text`.
    pub line: u32,
    /// The step that failed, when the platform says which.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckLog {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub failure: Option<LogFailure>,
}
