//! The crate's integration tests, one binary: every file under `tests/` would otherwise
//! link the whole dependency tree on its own.

mod base;
mod common;
mod exec;
mod fetch;
mod fetch_http;
mod gh_accounts;
mod git;
mod git_edges;
mod listing;
mod lsp;
mod materialize;
mod merge_base_ties;
mod project_files;
mod remerge;
mod restore;
mod semantic_cache;
mod servers;
mod store_contract;
mod unreadable;
mod versions;
mod windows_shim;
