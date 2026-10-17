//! What the backend holds for each project on disk, and for none.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A project is a repository root as it was recorded.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectStorage {
    pub root: String,
    /// The folder is still there; removing the project works without it.
    pub folder_exists: bool,
    /// The project is open in the backend now.
    pub open: bool,
    /// The project is on the recent list.
    pub recent: bool,
    /// Unix seconds the project was last opened; only a project on the recent list has it.
    pub last_opened: Option<u32>,
    /// What analysis of the project may execute was allowed (SPEC §6.5); removing forgets it.
    pub trusted: bool,
    /// Bytes of content only this project's snapshots reach, as stored.
    #[ts(type = "number")]
    pub review_data: u64,
    /// Bytes of the project's kept source trees.
    #[ts(type = "number")]
    pub source_trees: u64,
    /// Of `source_trees`, those a language server uses or an open review needs.
    #[ts(type = "number")]
    pub source_trees_in_use: u64,
    /// Bytes of the language servers' answers given for the project's snapshots.
    #[ts(type = "number")]
    pub semantic_cache: u64,
    /// What removing the project loses: reviews that are open, messages only Vet has (drafts and
    /// local comments), and votes that wait to be sent.
    pub open_reviews: u32,
    pub unposted_comments: u32,
    pub pending_votes: u32,
    /// Unix seconds the figures were computed at; they are kept for a short while.
    pub computed_at: u32,
}

/// What is kept for no project in particular, in bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppStorage {
    /// The file of all projects' answers of the language servers.
    #[ts(type = "number")]
    pub semantic_cache: u64,
    /// The language servers Vet installed.
    #[ts(type = "number")]
    pub language_servers: u64,
    /// The databases of reviews and content.
    #[ts(type = "number")]
    pub databases: u64,
}
