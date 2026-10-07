//! A recently opened repository.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecentProject {
    /// The path as it was recorded.
    pub path: String,
    /// Unix seconds.
    pub opened_at: u32,
}
