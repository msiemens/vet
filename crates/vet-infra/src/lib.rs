//! Adapters. No business logic: each module implements one port.

/// `vet_app::ports::GitRepo` over gitoxide.
pub mod git;

/// `vet_app::ports::Watcher`: FSEvents, `ReadDirectoryChangesW`, inotify, and
/// polling for `/mnt/*`.
pub mod watch;

/// `vet_app::ports::Clock` over tokio time.
pub mod clock;

pub mod store;

/// The languages Vet analyses.
pub mod language;

pub mod lsp;

/// `vet_app::ports::ProcessRunner`.
pub mod exec;

/// `vet_semantic::ports::Restorer`.
pub mod restore;

/// `vet_app::ports::ServerResolver`.
pub mod servers;

/// `RepoPath` to and from filesystem paths.
pub(crate) mod paths;

/// `vet_app::ports::Platform`: an enum over GitHub and Azure DevOps.
#[cfg(feature = "platform")]
pub mod platform {}
