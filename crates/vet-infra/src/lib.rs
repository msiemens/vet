//! Adapters. No business logic: each module implements one port.

/// `vet_app::ports::GitRepo` over gitoxide. Checkout plus store overlay and the
/// persisted stat-to-hash cache are still to come.
pub mod git;

/// `vet_app::ports::Watcher`: FSEvents, `ReadDirectoryChangesW`, inotify, and
/// polling for `/mnt/*`.
pub mod watch;

/// `vet_app::ports::Clock` over tokio time.
pub mod clock;

pub mod store;

/// `vet_semantic::ports::ServerLauncher` and `LanguageServer`: one concrete
/// stdio LSP client, plus the `ServerCapabilities` assertion on start.
pub mod lsp {}

/// `vet_app::ports::ProcessRunner`.
pub mod exec;

/// `vet_app::ports::Platform`: an enum over GitHub and Azure DevOps.
#[cfg(feature = "platform")]
pub mod platform {}
