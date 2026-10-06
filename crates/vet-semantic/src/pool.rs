//! Server pool (§7.2). Servers are keyed by `(target, backend, policy)`, kept
//! warm while in use, and evicted LRU. The policy is part of the key so that a
//! server started without permission to execute project code stops serving once
//! the PR is trusted (and the reverse can never happen).

use std::path::PathBuf;

use vet_core::content::SnapshotId;

use crate::policy::AnalysisPolicy;
use crate::ports::{Backend, BackendId, LaunchError, Materializer, NotMaterializable, ServerLauncher};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Target {
    /// The dirty tree: served in place, answers never cached.
    Live,
    /// An immutable snapshot, served from a materialized directory.
    Snapshot(SnapshotId),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PoolKey {
    pub target: Target,
    pub backend: BackendId,
    pub policy: AnalysisPolicy,
}

#[derive(Debug)]
pub enum PoolError {
    NotMaterializable(NotMaterializable),
    Launch(LaunchError),
    /// The backend would execute project code and the policy forbids it. The UI
    /// shows an "analysis limited" marker for the language (§7.5).
    AnalysisLimited(BackendId),
}

/// Refuse a backend the policy does not allow, before anything is started.
pub fn admit(backend: &Backend, policy: AnalysisPolicy) -> Result<(), PoolError> {
    let config = backend.config();
    if config.executes_project_code_during_analysis && !policy.allow_executing {
        return Err(PoolError::AnalysisLimited(config.backend.clone()));
    }
    Ok(())
}

pub struct Pool<M, L> {
    pub materializer: M,
    pub launcher: L,
    pub live_root: PathBuf,
    pub capacity: usize,
}

impl<M: Materializer, L: ServerLauncher> Pool<M, L> {
    /// Get a warm server for `key`, starting one if needed.
    pub async fn server(&self, _key: &PoolKey, _backend: &Backend) -> Result<&mut L::Server, PoolError> {
        todo!("admit, LRU map, materialize, launch, drop guard on eviction")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::ServerConfig;

    fn backend(executes: bool) -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId("rust".into()),
            binary: "rust-analyzer".into(),
            filetypes: vec!["rs".into()],
            root_markers: vec!["Cargo.toml".into()],
            executes_project_code_during_analysis: executes,
        })
    }

    #[test]
    fn untrusted_policy_refuses_executing_backends_only() {
        let untrusted = AnalysisPolicy { allow_executing: false };
        let trusted = AnalysisPolicy { allow_executing: true };
        assert!(matches!(admit(&backend(true), untrusted), Err(PoolError::AnalysisLimited(_))));
        assert!(admit(&backend(false), untrusted).is_ok());
        assert!(admit(&backend(true), trusted).is_ok());
    }

    #[test]
    fn policy_is_part_of_the_pool_key() {
        let key = |allow_executing| PoolKey {
            target: Target::Live,
            backend: BackendId("rust".into()),
            policy: AnalysisPolicy { allow_executing },
        };
        assert_ne!(key(false), key(true));
    }
}
