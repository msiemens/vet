//! The backend's own settings (SPEC §8): app-wide, not per project.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackendSettings {
    /// Megabytes the answers of the language servers kept between launches may take; beyond
    /// it the least recently used go.
    pub semantic_cache_mb: u32,
}
