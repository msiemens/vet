//! The checks of commits on a hosting platform: CI runs and statuses.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CiState;

/// One check of a commit.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckRun {
    /// Names the run in `CheckLog`.
    pub id: String,
    pub name: String,
    pub state: CiState,
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
