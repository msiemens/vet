//! Adapters. No business logic: each module implements one port.

// Tests build their fixtures with git, fake servers and threads of their own.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

/// Bounds on blocking work and processes.
pub mod budget;

/// `vet_app::ports::GitRepo` over gitoxide.
pub mod git;

/// `vet_app::ports::Watcher`: FSEvents, `ReadDirectoryChangesW`, inotify, and
/// polling for `/mnt/*`.
pub mod watch;

/// `vet_app::ports::Clock` over tokio time.
pub mod clock;

pub mod store;

/// `vet_app::platform::CredentialValidator` over HTTP.
pub mod credentials;

/// `vet_app::platform::Platform` over HTTP, one module per platform.
pub mod platform;

pub(crate) mod http;

pub(crate) mod sync;

/// `vet_semantic::ports::Materializer`.
pub mod materialize;

/// The languages Vet analyses.
pub mod language;

pub mod lsp;

/// `vet_app::ports::ProcessRunner`.
pub mod exec;

/// `vet_semantic::ports::Restorer`.
pub mod restore;

/// The JSON schemas bundled for the YAML and JSON servers.
pub mod schemas;

/// `vet_app::ports::ServerResolver`.
pub mod servers;

/// `vet_semantic::reuse::SemanticCache`.
pub mod semantic_cache;

/// `vet_app::search::LineMatcher` over the `regex` crate.
pub mod search;

/// `RepoPath` to and from filesystem paths.
pub(crate) mod paths;
pub use paths::is_file_in;
