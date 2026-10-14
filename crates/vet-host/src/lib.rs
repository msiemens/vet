//! The logic of the Tauri host that needs no window, so that it is tested with the workspace.

// The host is no part of the backend's budget and process rules: it runs one backend child with
// threads of its own that wait on it.
#![allow(clippy::disallowed_methods)]

pub mod backend;
pub mod credential_hosts;
pub mod files;
pub mod paths;
pub mod web;
