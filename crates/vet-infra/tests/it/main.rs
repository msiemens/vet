//! The crate's integration tests, one binary: every file under `tests/` would otherwise
//! link the whole dependency tree on its own.

mod base;
mod common;
mod exec;
mod fetch;
mod git;
mod lsp;
mod materialize;
mod remerge;
mod restore;
mod semantic_cache;
mod servers;
