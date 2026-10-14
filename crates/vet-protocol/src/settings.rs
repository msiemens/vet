//! The backend's own settings and the window's preferences (SPEC §8).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackendSettings {
    /// Megabytes the answers of the language servers kept between launches may take; beyond
    /// it the least recently used go.
    pub semantic_cache_mb: u32,
    /// Days a changeset is kept after its last review was finished, unless a thread of it is
    /// open; it is removed at the first start after that.
    pub finished_review_days: u32,
}

/// A value the window remembers (SPEC §8), under a key the window chose.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Preference {
    pub key: String,
    /// JSON text; the backend keeps it as it is.
    pub value: String,
}
